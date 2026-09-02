using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>What the trampoline publishes into, without needing to know the response type.</summary>
internal interface ICallSink
{
  void Publish(NativeMethods.AkEventKind kind,
               in NativeMethods.AkBytes payload,
               int statusCode);
}

/// <summary>
///   One call in flight: a ring the library's threads publish into, and one drain that empties it.
/// </summary>
/// <remarks>
///   The ring is the stream queue - metadata, messages and the terminal all ride it, so there is
///   one buffer per call and not two. It is sized past what the ABI can leave outstanding, so
///   publishing cannot fail, cannot allocate and cannot block: the proved liveness of the native
///   actor assumes the callback returns, and a callback that can fail is a callback that can fail
///   to return.
///   <para>
///     Payloads ride the ring owned, and the drain gives each one back after parsing it, on a
///     managed thread. One drain and one only: two would interleave the releases, and their order
///     is not recoverable afterwards.
///   </para>
/// </remarks>
internal sealed class NativeCall<TResponse> : ICallSink, IDisposable
  where TResponse : class
{
  private readonly Slot[] ring_;
  private readonly int mask_;
  private long head_;
  private long tail_;
  private readonly AsyncAutoResetEvent arrived_ = new();

  private readonly TaskCompletionSource<Metadata> headers_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly TaskCompletionSource<Status> terminal_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly Marshaller<TResponse> marshaller_;
  private readonly ulong runtime_;

  private GCHandle self_;
  private ulong handle_;
  private Metadata trailers_ = new();
  private CancellationTokenRegistration cancellation_;
  private int cancelled_;

  private NativeCall(ulong runtime,
                     int deliveryCredits,
                     Marshaller<TResponse> marshaller)
  {
    runtime_    = runtime;
    marshaller_ = marshaller;

    // The smallest power of two holding the window's peak occupancy, which is one past the
    // credits: a terminal goes out on a spent window. Publishing therefore never tests for
    // fullness, which is what lets it run inside the callback.
    var size = 1;
    while (size < deliveryCredits + 1)
    {
      size <<= 1;
    }

    ring_ = new Slot[size];
    mask_ = size - 1;

    // Nobody is obliged to await the headers, and an unobserved fault is noise, not news.
    _ = headers_.Task.ContinueWith(static answered => _ = answered.Exception,
                                   TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously);

    self_ = GCHandle.Alloc(this);
  }

  internal Task<Metadata> ResponseHeadersAsync
    => headers_.Task;

  internal Task<Status> TerminalAsync
    => terminal_.Task;

  internal Metadata Trailers
    => trailers_;

  internal static NativeCall<TResponse> Start(ulong runtime,
                                              ulong channel,
                                              int deliveryCredits,
                                              string method,
                                              Metadata? metadata,
                                              Marshaller<TResponse> marshaller)
  {
    var call = new NativeCall<TResponse>(runtime,
                                         deliveryCredits,
                                         marshaller);
    var methodBytes = System.Text.Encoding.UTF8.GetBytes(method);
    var metadataBytes = Blob.Encode(metadata);

    var methodPin = GCHandle.Alloc(methodBytes,
                                   GCHandleType.Pinned);
    var metadataPin = GCHandle.Alloc(metadataBytes,
                                     GCHandleType.Pinned);
    try
    {
      var options = new NativeMethods.AkCallStartOptions
                    {
                      StructSize = (uint)Marshal.SizeOf<NativeMethods.AkCallStartOptions>(),
                      Method = Borrow(methodPin,
                                      methodBytes.Length),
                      Metadata = Borrow(metadataPin,
                                        metadataBytes.Length),
                    };

      var status = NativeMethods.ak_call_start(channel,
                                               ref options,
                                               GCHandle.ToIntPtr(call.self_),
                                               out call.handle_);
      if (status != NativeMethods.AkStatus.Ok)
      {
        // It answered before the call existed, so no callback can ever carry this root.
        call.self_.Free();
        throw Failed($"the call could not be started ({status})");
      }

      return call;
    }
    finally
    {
      methodPin.Free();
      metadataPin.Free();
    }
  }

  /// <summary>Takes one event, on one of the library's threads. Allocates nothing and cannot fail.</summary>
  public void Publish(NativeMethods.AkEventKind kind,
                      in NativeMethods.AkBytes payload,
                      int statusCode)
  {
    // WRITE_DONE settles a send rather than carrying one, and a unary caller never observes it.
    if (kind == NativeMethods.AkEventKind.WriteDone)
    {
      return;
    }

    var at = (int)(head_ & mask_);
    ring_[at].Payload = payload;
    ring_[at].Kind    = kind;
    ring_[at].Status  = statusCode;
    Volatile.Write(ref head_,
                   head_ + 1);
    arrived_.Set();

    if (kind == NativeMethods.AkEventKind.Status)
    {
      // The last callback of the call, so native use of the root ends here. The drain keeps its
      // own reference, so this collects nothing.
      self_.Free();
    }
  }

  /// <summary>Sends one message and half-closes, which is the whole of a unary request.</summary>
  internal async Task SendUnaryAsync<TRequest>(Marshaller<TRequest> marshaller,
                                               TRequest request,
                                               CancellationToken token)
  {
    using var lent = new LentBuffer(handle_);
    marshaller.ContextualSerializer(request,
                                    lent);

    while (true)
    {
      var status = lent.Commit();
      if (status == NativeMethods.AkStatus.Ok)
      {
        break;
      }

      if (status is not (NativeMethods.AkStatus.BudgetBusy or NativeMethods.AkStatus.SlotBusy))
      {
        throw Failed($"the message was refused ({status})");
      }

      await WaitForRoomAsync(token)
        .ConfigureAwait(false);
    }

    var closed = NativeMethods.ak_call_end_send(handle_);
    // A call already at its terminal has nothing left to half-close; anything else is a bug here.
    if (closed is not (NativeMethods.AkStatus.Ok or NativeMethods.AkStatus.HandleStale
                                                 or NativeMethods.AkStatus.InvalidState))
    {
      throw Failed($"the half-close was refused ({closed})");
    }
  }

  /// <summary>
  ///   Empties the ring to the terminal, and answers the call: the one response of a unary call,
  ///   or the status that says why there is none.
  /// </summary>
  internal async Task<TResponse> RunAsync()
  {
    TResponse? response = null;
    var seen = 0;

    while (true)
    {
      while (Volatile.Read(ref head_) == tail_)
      {
        await arrived_.WaitAsync()
                      .ConfigureAwait(false);
      }

      var slot = ring_[(int)(tail_ & mask_)];
      try
      {
        switch (slot.Kind)
        {
          case NativeMethods.AkEventKind.InitialMetadata:
            headers_.TrySetResult(Blob.Decode(Bytes(slot.Payload)));
            break;

          case NativeMethods.AkEventKind.Message:
            seen++;
            response = marshaller_.ContextualDeserializer(new ReceivedMessage(slot.Payload.Ptr,
                                                                             (int)slot.Payload.Len));
            break;

          case NativeMethods.AkEventKind.Status:
            Blob.DecodeStatus(Bytes(slot.Payload),
                              out var reason,
                              out var trailers);
            trailers_ = trailers;
            var ended = new Status((StatusCode)slot.Status,
                                   reason);
            terminal_.TrySetResult(ended);
            // The head is synthesized when the wire carries none, so reaching here with the
            // headers still pending means the call died before them. That is what a caller
            // awaiting them needs to hear, and an empty collection would not say it.
            if (ended.StatusCode == StatusCode.OK)
            {
              headers_.TrySetResult(new Metadata());
            }
            else
            {
              headers_.TrySetException(new RpcException(ended,
                                                        trailers));
            }

            break;
        }
      }
      finally
      {
        NativeMethods.ak_event_consumed(slot.Payload);
        tail_++;
      }

      if (slot.Kind == NativeMethods.AkEventKind.Status)
      {
        break;
      }
    }

    cancellation_.Dispose();

    var status = await terminal_.Task.ConfigureAwait(false);
    if (status.StatusCode != StatusCode.OK)
    {
      throw new RpcException(status,
                             trailers_);
    }

    // A unary call's answer is its message and its status together, and exactly one message: a
    // server that sends none or several has not answered this method.
    if (seen != 1)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"a unary call answered with {seen} messages"),
                             trailers_);
    }

    return response!;
  }

  internal void CancelWith(CancellationToken token)
  {
    if (token.CanBeCanceled)
    {
      cancellation_ = token.Register(Cancel);
    }
  }

  internal void Cancel()
  {
    if (Interlocked.Exchange(ref cancelled_,
                             1) == 0)
    {
      NativeMethods.ak_call_cancel(handle_);
    }
  }

  /// <summary>
  ///   Asks for the call to end. It does not wait for it: the drain does, and it is what releases
  ///   the payloads and the root.
  /// </summary>
  /// <remarks>
  ///   The terminal arrives with no further host action - `CancellationCompletes` in the model -
  ///   so there is nothing here to hand over and nothing to poll for.
  /// </remarks>
  public void Dispose()
    => Cancel();

  private async Task WaitForRoomAsync(CancellationToken token)
  {
    // The ceiling has no event announcing room, so this is the poll the header prescribes.
    while (true)
    {
      token.ThrowIfCancellationRequested();
      await Task.Delay(TimeSpan.FromMilliseconds(2),
                       token)
                .ConfigureAwait(false);

      if (NativeMethods.ak_runtime_memory_usage(runtime_,
                                                out var usage) != NativeMethods.AkStatus.Ok
          || usage.Ceiling == 0
          || usage.BytesUsed < usage.Ceiling)
      {
        return;
      }
    }
  }

  private static unsafe ReadOnlySpan<byte> Bytes(in NativeMethods.AkBytes payload)
    => new((void*)payload.Ptr,
           (int)payload.Len);

  private static NativeMethods.AkBytesIn Borrow(GCHandle pinned,
                                                int length)
    => new()
       {
         Ptr = length == 0
                 ? IntPtr.Zero
                 : pinned.AddrOfPinnedObject(),
         Len = (UIntPtr)length,
       };

  private static RpcException Failed(string reason)
    => new(new Status(StatusCode.Internal,
                      reason));

  private struct Slot
  {
    internal NativeMethods.AkBytes Payload;
    internal NativeMethods.AkEventKind Kind;
    internal int Status;
  }
}
