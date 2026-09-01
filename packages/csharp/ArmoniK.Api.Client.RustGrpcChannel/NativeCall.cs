using System;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   One call in flight: what the trampoline publishes into, and what the caller reads out of.
/// </summary>
/// <remarks>
///   There is no dispatcher between the two. The trampoline publishes here and the caller reads
///   directly, so an event crosses one buffer instead of two - which is only safe because nothing
///   here runs application code: every completion source completes asynchronously, so a caller's
///   continuation never runs on the library's thread and never holds its actor up.
/// </remarks>
internal sealed class NativeCall : IDisposable
{
  private readonly TaskCompletionSource<Metadata> headers_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly TaskCompletionSource<Status> terminal_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly ConcurrentQueue<byte[]> messages_ = new();
  private readonly AsyncAutoResetEvent arrived_ = new();

  private GCHandle self_;
  private ulong handle_;
  private Metadata trailers_ = new();
  private int disposed_;

  private NativeCall()
    => self_ = GCHandle.Alloc(this);

  /// <summary>The token the library hands back in each of this call's events.</summary>
  internal IntPtr Context
    => GCHandle.ToIntPtr(self_);

  internal Task<Metadata> ResponseHeadersAsync
    => headers_.Task;

  /// <summary>Starts a call on <paramref name="channel" />, or throws what the ABI answered.</summary>
  internal static NativeCall Start(ulong channel,
                                   string method,
                                   Metadata? metadata)
  {
    var call = new NativeCall();
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
                                               call.Context,
                                               out call.handle_);
      if (status != NativeMethods.AkStatus.Ok)
      {
        call.Dispose();
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

  /// <summary>
  ///   Sends one message and half-closes, which is the whole of a unary request.
  /// </summary>
  /// <remarks>
  ///   The payload has to sit in a buffer the library lends: that is the allocation its memory
  ///   ceiling accounts for, and the one it may keep until the frame is on the wire.
  /// </remarks>
  internal void SendUnary(byte[] request)
  {
    var status = NativeMethods.ak_get_call_buffer(handle_,
                                                  (UIntPtr)request.Length,
                                                  out var buffer);
    if (status != NativeMethods.AkStatus.Ok)
    {
      throw Failed($"no buffer to serialize into ({status})");
    }

    Marshal.Copy(request,
                 0,
                 buffer.Ptr,
                 request.Length);

    status = NativeMethods.ak_call_send_message(handle_,
                                                buffer);
    if (status != NativeMethods.AkStatus.Ok)
    {
      // Refused, so the buffer is still ours and this is its only exit.
      NativeMethods.ak_return_call_buffer(buffer);
      throw Failed($"the message was refused ({status})");
    }

    NativeMethods.ak_call_end_send(handle_);
  }

  /// <summary>The one response of a unary call, or the status that says why there is none.</summary>
  internal async Task<byte[]> ReadUnaryAsync()
  {
    while (true)
    {
      if (messages_.TryDequeue(out var message))
      {
        return message;
      }

      if (terminal_.Task.IsCompleted)
      {
        var status = await terminal_.Task.ConfigureAwait(false);
        throw new RpcException(status.StatusCode == StatusCode.OK
                                 ? new Status(StatusCode.Internal,
                                              "the call ended without a response message")
                                 : status,
                               trailers_);
      }

      await arrived_.WaitAsync()
                    .ConfigureAwait(false);
    }
  }

  internal Task<Status> TerminalAsync
    => terminal_.Task;

  internal Metadata Trailers
    => trailers_;

  internal void Cancel()
  {
    if (handle_ != 0)
    {
      NativeMethods.ak_call_cancel(handle_);
    }
  }

  /// <summary>Takes one event from the library's thread. Runs no application code.</summary>
  internal void OnEvent(NativeMethods.AkEventKind kind,
                        byte[] payload,
                        int statusCode)
  {
    switch (kind)
    {
      case NativeMethods.AkEventKind.InitialMetadata:
        headers_.TrySetResult(Blob.Decode(payload));
        break;

      case NativeMethods.AkEventKind.Message:
        messages_.Enqueue(payload);
        break;

      case NativeMethods.AkEventKind.Status:
        Blob.DecodeStatus(payload,
                          out var reason,
                          out var trailers);
        trailers_ = trailers;
        // A call that ends before its head still answers the question, with nothing in it.
        headers_.TrySetResult(new Metadata());
        terminal_.TrySetResult(new Status((StatusCode)statusCode,
                                          reason));
        break;
    }

    arrived_.Set();
  }

  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) != 0)
    {
      return;
    }

    // The handle goes stale at a moment this side does not choose, so cancelling a call that has
    // already been reclaimed is the ordinary case and not an error.
    Cancel();
    terminal_.TrySetResult(new Status(StatusCode.Cancelled,
                                      "the call was disposed"));
    headers_.TrySetResult(new Metadata());
    arrived_.Set();

    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }

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
}
