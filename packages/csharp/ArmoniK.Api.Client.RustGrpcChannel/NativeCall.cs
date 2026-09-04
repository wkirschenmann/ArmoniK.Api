// This file is part of the ArmoniK project
//
// Copyright (C) ANEO, 2021-2026. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License")
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>What the trampoline publishes into, without needing to know the response type.</summary>
internal interface ICallSink
{
  /// <summary>
  ///   The terminal's callback has returned, so nothing native holds this call any more.
  /// </summary>
  /// <remarks>
  ///   Separate from <see cref="Publish" /> and called after it, because that is the order the
  ///   model states: the root must be live for as long as a callback carrying it is running, so
  ///   releasing it from inside the terminal's own callback makes
  ///   <c>RootSurvivesCallbacks</c> momentarily false. Nothing reads it in that window today -
  ///   the header promises every WRITE_DONE precedes the terminal and the engine enforces it -
  ///   which is a reason it was harmless, not a reason to keep relying on it.
  /// </remarks>
  void TerminalReturned();

  /// <summary>Asks the call to end, without waiting for it.</summary>
  void Cancel();

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
internal sealed class NativeCall<TResponse> : ICallSink
  where TResponse : class
{
  private readonly Slot[] ring_;
  private readonly int mask_;
  private long head_;
  private long tail_;
  private readonly ArrivalSignal arrived_ = new();

  private readonly TaskCompletionSource<Metadata> headers_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly TaskCompletionSource<Status> terminal_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  /// <summary>Faults the head, and marks the fault observed.</summary>
  /// <remarks>
  ///   Nobody is obliged to await the headers, and an unobserved fault is noise rather than news.
  ///   Marked at the two places a fault is set, rather than by a continuation registered on every
  ///   call - most of which never fault, and each of which paid a task for the privilege.
  /// </remarks>
  private void FailHead(RpcException reason)
  {
    if (headers_.TrySetException(reason))
    {
      _ = headers_.Task.Exception;
    }
  }

  /// <summary>
  ///   Sends this call has committed and whose WRITE_DONE has not arrived.
  /// </summary>
  /// <remarks>
  ///   The model gives its writer an <c>awaiting_write_done</c> state between the commit and the
  ///   acquittal, and closes the writer only from <c>idle</c>. This is that state, as a count:
  ///   dropping WRITE_DONE left the write finished at the commit, which is a step the model does
  ///   not have and which nothing could check.
  ///   <para>
  ///     Charged before the commit and not after it, because the acquittal is emitted from a
  ///     library thread and can arrive before a downcall has returned. A commit that is refused
  ///     gives the charge back, there being nothing to acquit.
  ///   </para>
  /// </remarks>
  private int inFlight_;

  private readonly Marshaller<TResponse> marshaller_;
  private readonly NativeRuntime runtime_;

  private GCHandle self_;
  private ulong handle_;
  /// <summary>
  ///   The trailing metadata. Empty until the terminal decodes it, which is every call that
  ///   reaches one; the shared empty collection is what stands in for the path where the decode
  ///   itself throws.
  /// </summary>
  private Metadata trailers_ = Metadata.Empty;
  private Task<TResponse>? drained_;

  /// <summary>
  ///   Cancelled when this call stops taking work, whichever reason arrives first.
  /// </summary>
  /// <remarks>
  ///   The model names the reasons as one expression (<c>WaitIsHopeless</c>) and says why: the
  ///   guard on the wait's resolution and the antecedent of the promise that the wait ends are
  ///   the same causes, so they have to be the same thing. This is that thing. A caller's token,
  ///   a dispose and the terminal all reach it, and the send's wait for room observes it - where
  ///   a flag read after the wait returned could only turn a completed wait into a failure, and
  ///   left a cancelled send parked until the byte ceiling happened to free.
  /// </remarks>
  private readonly CancellationTokenSource ending_ = new();

  /// <summary>
  ///   The size prefix every <c>ak_call_start</c> carries, asked of the layout once.
  /// </summary>
  private static readonly uint StartOptionsSize = (uint)Marshal.SizeOf<NativeMethods.AkCallStartOptions>();

  /// <summary>
  ///   The method name as UTF-8, kept per <see cref="Method{TRequest,TResponse}" />.
  /// </summary>
  /// <remarks>
  ///   A generated stub holds one <c>Method</c> in a static field and every call on it carries the
  ///   same name, so transcoding it per call was work with a fixed answer. Keyed weakly on the
  ///   method object, so the table is bounded by what the process keeps alive rather than by how
  ///   many distinct names it has ever seen.
  /// </remarks>
  private static readonly ConditionalWeakTable<string, byte[]> MethodNames = new();

  private CancellationTokenRegistration cancellation_;

  private NativeCall(NativeRuntime runtime,
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

    self_ = GCHandle.Alloc(this);
  }

  internal Task<Metadata> ResponseHeadersAsync
    => headers_.Task;

  /// <summary>
  ///   The drain: past the terminal, every payload consumed and every buffer given back.
  /// </summary>
  internal Task<TResponse> Drained
    => drained_ ?? throw new InvalidOperationException("the call was not started");

  internal Task<Status> TerminalAsync
    => terminal_.Task;

  internal Metadata Trailers
    => trailers_;

  internal static NativeCall<TResponse> Start(NativeRuntime runtime,
                                              ulong channel,
                                              int deliveryCredits,
                                              string method,
                                              Metadata? metadata,
                                              Marshaller<TResponse> marshaller)
  {
    var call = new NativeCall<TResponse>(runtime,
                                         deliveryCredits,
                                         marshaller);
    var methodBytes = MethodNames.GetValue(method,
                                           static name => Encoding.UTF8.GetBytes(name));
    var metadataBytes = RawMetadata.Encode(metadata);

    var methodPin = GCHandle.Alloc(methodBytes,
                                   GCHandleType.Pinned);
    var metadataPin = GCHandle.Alloc(metadataBytes,
                                     GCHandleType.Pinned);
    try
    {
      var options = new NativeMethods.AkCallStartOptions
                    {
                      StructSize = StartOptionsSize,
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

      // Registered now that there is a handle to cancel, and it runs at most once however many
      // reasons arrive: that is what a token source gives that a flag and an interlocked
      // exchange were standing in for.
      call.ending_.Token.Register(call.EndNative);

      // The drain starts here and not at a caller's discretion. It is what gives the library its
      // payloads back, so a call whose drain never ran is a call that is never reclaimed and a
      // runtime that never quiesces - not something to leave to whoever holds the call next.
      call.drained_ = call.RunAsync();

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
    // WRITE_DONE settles a send rather than carrying one, so it does not ride the ring - which
    // is sized to the delivery window and must never be tested for fullness. It is counted
    // instead: the call cannot be handed to a caller as answered while a send it accepted is
    // still unacquitted, and something has to be able to say so.
    if (kind == NativeMethods.AkEventKind.WriteDone)
    {
      Interlocked.Decrement(ref inFlight_);
      return;
    }

    var at = (int)(head_ & mask_);
    ring_[at].Payload = payload;
    ring_[at].Kind    = kind;
    ring_[at].Status  = statusCode;
    Volatile.Write(ref head_,
                   head_ + 1);
    arrived_.Set();

  }

  /// <inheritdoc />
  public void TerminalReturned()
  {
    // The drain keeps its own reference, so this collects nothing.
    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }

  /// <summary>Sends one message and half-closes, which is the whole of a unary request.</summary>
  internal async Task SendUnaryAsync<TRequest>(Marshaller<TRequest> marshaller,
                                               TRequest request)
  {
    try
    {
      await SendingAsync(marshaller,
                         request)
        .ConfigureAwait(false);
    }
    catch
    {
      // The call is over whether or not the engine has said so, and this is what knows it. A
      // drain left parked on a terminal nobody will provoke is what made the caller compensate
      // in a `catch` of its own, one layer up from the state it was compensating for.
      ending_.Cancel();
      throw;
    }
  }

  private async Task SendingAsync<TRequest>(Marshaller<TRequest> marshaller,
                                            TRequest request)
  {
    using var lent = new LentBuffer(handle_);
    marshaller.ContextualSerializer(request,
                                    lent);

    while (true)
    {
      // Charged first: the acquittal comes from a library thread and may land before this
      // downcall has returned, so counting after it could see the decrement first.
      Interlocked.Increment(ref inFlight_);
      var status = lent.Commit();
      if (status == NativeMethods.AkStatus.Ok)
      {
        break;
      }

      // Refused, so there is nothing to acquit and nothing to wait for.
      Interlocked.Decrement(ref inFlight_);

      // BUDGET_BUSY and nothing else waits here. SLOT_BUSY would mean this call's send window
      // is full, and the header says its wake-up is the call's next WRITE_DONE - which this side
      // counts but does not wait on - so waiting on the byte ceiling would be waiting on the
      // wrong thing. It is also unreachable: the window is one, the writer is single, and a
      // unary call sends once, which is what `ManagedWriterNeverObservesSlotBusy` asserts. So it
      // is a bug here rather than a state to wait out.
      if (status != NativeMethods.AkStatus.BudgetBusy)
      {
        throw Failed($"the message was refused ({status})");
      }

      try
      {
        await runtime_.WaitForRoomAsync(ending_.Token)
                      .ConfigureAwait(false);
      }
      catch (OperationCanceledException)
      {
        // The wait ended because the call did, which is a cancellation and reads as one. A flag
        // checked after the wait returned could only turn a completed wait into a failure, and
        // left a cancelled send parked until the ceiling happened to free.
        throw new RpcException(new Status(StatusCode.Cancelled,
                                          "the call ended while its send waited for room against the ceiling"));
      }
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
  /// <summary>
  ///   Reads the ring to the terminal, giving every payload back on the way.
  /// </summary>
  /// <remarks>
  ///   Private, and started by <see cref="Start" />: a call has one drain and one only, which is
  ///   what lets a payload's release order be recoverable and what makes
  ///   <see cref="ArrivalSignal" />'s single-waiter precondition structural rather than a
  ///   convention. <see cref="Drained" /> hands out the task it is already running.
  /// </remarks>
  private async Task<TResponse> RunAsync()
  {
    TResponse? response = null;
    var seen = 0;
    Exception? refused = null;

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
            headers_.TrySetResult(RawMetadata.Decode(Bytes(slot.Payload)));
            break;

          case NativeMethods.AkEventKind.Message:
            seen++;
            response = marshaller_.ContextualDeserializer(new ReceivedMessage(slot.Payload.Ptr,
                                                                              (int)slot.Payload.Len));
            break;

          case NativeMethods.AkEventKind.Status:
            Settle(slot);
            break;
        }
      }
      catch (Exception thrown)
      {
        // Kept, not thrown: leaving this loop would abandon every later slot, the terminal
        // above all, and a payload never consumed is a call never reclaimed and a runtime that
        // never quiesces - one malformed message would cost the process its engine. So the
        // drain carries the failure to the end and reports it there.
        refused ??= thrown;

        // The terminal's own decode failing is the case that cannot be deferred: nothing else
        // will resolve the call, so it gets a status saying why rather than none at all.
        if (slot.Kind == NativeMethods.AkEventKind.Status)
        {
          var synthetic = new Status(StatusCode.Internal,
                                     $"the call's terminal could not be read: {thrown.Message}");
          terminal_.TrySetResult(synthetic);
          FailHead(new RpcException(synthetic));
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

    // The call is over, so anything still waiting on it should stop. `EndNative` asks the engine
    // nothing, the terminal having arrived.
    //
    // The source itself is not disposed: `AsyncUnaryCall.Dispose` reaches `Cancel` after the
    // answer has been awaited, and a caller is entitled to do that. It holds no timer, so what
    // disposing would release is the registration below, which is released. Cancelling an
    // already-cancelled source is the no-op this relies on.
    ending_.Cancel();
    cancellation_.Dispose();

    // Now that every payload is back, whatever the drain could not read is the answer.
    if (refused is not null)
    {
      throw refused is RpcException rpc
              ? rpc
              : new RpcException(new Status(StatusCode.Internal,
                                            $"the call's events could not be read: {refused.Message}"),
                                 trailers_);
    }

    // The header promises every WRITE_DONE precedes the terminal, and the model closes a writer
    // only from idle. This is where that is worth checking: past this point the call is answered
    // and nobody would look again. A send still in flight here means the engine acquitted late,
    // which would leave `AwaitingWriteDoneHasOneComing` false and a buffer charged against a
    // call that is finished.
    var unacquitted = Volatile.Read(ref inFlight_);
    if (unacquitted != 0)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the call reached its terminal with {unacquitted} send(s) unacquitted"),
                             trailers_);
    }

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

  /// <summary>Resolves the call from its terminal event.</summary>
  private void Settle(in Slot slot)
  {
    RawMetadata.DecodeStatus(Bytes(slot.Payload),
                      out var reason,
                      out var trailers);
    trailers_ = trailers;

    var ended = new Status((StatusCode)slot.Status,
                           reason);
    terminal_.TrySetResult(ended);

    // The head is synthesized when the wire carries none, so reaching here with the headers
    // still pending means the call died before them. That is what a caller awaiting them needs
    // to hear, and an empty collection would not say it.
    if (ended.StatusCode == StatusCode.OK)
    {
      // The shared empty one: the engine delivers a head for every call, so this only ever
      // resolves a head already resolved, and allocating to be dropped is waste.
      headers_.TrySetResult(Metadata.Empty);
    }
    else
    {
      FailHead(new RpcException(ended,
                                trailers));
    }
  }

  internal void CancelWith(CancellationToken token)
  {
    if (token.CanBeCanceled)
    {
      cancellation_ = token.Register(Cancel);
    }
  }

  /// <inheritdoc />
  public void Cancel()
    => ending_.Cancel();

  /// <summary>Tells the engine, unless it has already ended the call itself.</summary>
  /// <remarks>
  ///   The check is not a second latch - the token source already answers once - it is what keeps
  ///   the terminal from provoking a downcall that says nothing: a call past its terminal cancels
  ///   the source so a waiting send stops, and there is nothing left to ask the engine.
  /// </remarks>
  private void EndNative()
  {
    if (!terminal_.Task.IsCompleted)
    {
      NativeMethods.ak_call_cancel(handle_);
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
