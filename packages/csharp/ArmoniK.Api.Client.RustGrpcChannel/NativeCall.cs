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
using System.Runtime.ExceptionServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal interface ICallSink
{
  void TerminalReturned();

  void Cancel();

  /// <summary>Answers whether returning the payload is now the consumer's obligation.</summary>
  bool Publish(NativeMethods.AkEventKind kind,
               in NativeMethods.AkBytes payload,
               int statusCode);
}

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

  private void FailHead(RpcException reason)
  {
    if (headers_.TrySetException(reason))
    {
      // Read so it counts as observed: a caller that only awaits the response never reads the
      // head, and an unobserved exception is raised again from the finalizer thread.
      _ = headers_.Task.Exception;
    }
  }

  private int inFlight_;

  private int holding_;

  private readonly ArrivalSignal handedBack_ = new();

  // The write waiting for its acquittal, or null between writes. A write linearizes at its
  // WRITE_DONE and not at the commit, which is what lets one writer send in a row against a
  // window of one: the emission that completes a write has already freed the slot the next lend
  // asks for, so a conformant writer never meets SLOT_BUSY.
  private TaskCompletionSource<bool>? writing_;

  private readonly Marshaller<TResponse> marshaller_;
  private readonly NativeRuntime runtime_;

  private GCHandle self_;
  private ulong handle_;
  private Metadata trailers_ = Metadata.Empty;
  private Task<TResponse>? drained_;

  private readonly CancellationTokenSource ending_ = new();

  private static readonly uint StartOptionsSize = (uint)Marshal.SizeOf<NativeMethods.AkCallStartOptions>();

  private static readonly ConditionalWeakTable<string, byte[]> MethodNames = new();

  private CancellationTokenRegistration cancellation_;

  private NativeCall(NativeRuntime runtime,
                     int deliveryCredits,
                     Marshaller<TResponse> marshaller)
  {
    runtime_    = runtime;
    marshaller_ = marshaller;

    // One slot more than the window, because the terminal goes out with every credit spent.
    //
    // `NativeRuntimeFactory.MaxDeliveryCredits` is what keeps this loop finite: a shift is not
    // checked in C#, so an unbounded window would take `size` through `int.MinValue` to zero and
    // spin here for ever on the caller's thread.
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

  internal Task<TResponse> Drained
    => drained_ ?? throw new InvalidOperationException("the call was not started");

  internal Task<Status> TerminalAsync
    => terminal_.Task;

  internal Metadata Trailers
    => trailers_;

  /// <param name="streams">
  ///   Whether the application reads the response itself. A single-response cardinality is read
  ///   by the call, so its reader runs from here; a server stream is read by whoever holds it,
  ///   and starting a reader here would race the application for the ring.
  /// </param>
  internal static NativeCall<TResponse> Start(NativeRuntime runtime,
                                              ulong channel,
                                              int deliveryCredits,
                                              string method,
                                              Metadata? metadata,
                                              Marshaller<TResponse> marshaller,
                                              bool streams = false)
  {
    var call = new NativeCall<TResponse>(runtime,
                                         deliveryCredits,
                                         marshaller);
    var methodBytes = MethodNames.GetValue(method,
                                           static name => Encoding.UTF8.GetBytes(name));
    var metadataBytes = RawMetadata.Encode(metadata);

    // The engine copies both before it answers, so the pin lasts exactly the call.
    unsafe
    {
      fixed (byte* methodPinned = methodBytes)
      fixed (byte* metadataPinned = metadataBytes)
      {
        var options = new NativeMethods.AkCallStartOptions
                      {
                        StructSize = StartOptionsSize,
                        Method = NativeMethods.AkBytesIn.Borrow(methodPinned,
                                                                methodBytes.Length),
                        Metadata = NativeMethods.AkBytesIn.Borrow(metadataPinned,
                                                                  metadataBytes.Length),
                      };

        var status = NativeMethods.ak_call_start(channel,
                                                 ref options,
                                                 GCHandle.ToIntPtr(call.self_),
                                                 out call.handle_);
        if (status != NativeMethods.AkStatus.Ok)
        {
          call.self_.Free();

          // A channel that has begun closing, or a handle whose generation is spent, is the
          // channel going away under a call that raced its disposal. That is the same answer
          // `NativeChannel.StartCall` gives when it sees the disposal first.
          throw new RpcException(new Status(status is NativeMethods.AkStatus.InvalidState
                                                   or NativeMethods.AkStatus.HandleStale
                                              ? StatusCode.Unavailable
                                              : StatusCode.Internal,
                                            $"the call could not be started ({status})"));
        }
      }
    }

    call.ending_.Token.Register(call.EndNative);

    call.settling_ = call.SettlingAsync();
    if (!streams)
    {
      call.drained_ = call.SingleAsync();
    }

    return call;
  }

  public bool Publish(NativeMethods.AkEventKind kind,
                      in NativeMethods.AkBytes payload,
                      int statusCode)
  {
    if (kind == NativeMethods.AkEventKind.WriteDone)
    {
      Interlocked.Decrement(ref inFlight_);
      Volatile.Read(ref writing_)
              ?.TrySetResult(true);
      return false;
    }

    var at = (int)(head_ & mask_);
    ring_[at].Payload = payload;
    ring_[at].Kind    = kind;
    ring_[at].Status  = statusCode;

    // From this write the slot is the reader's, and so is giving the payload back.
    Volatile.Write(ref head_,
                   head_ + 1);
    arrived_.Set();
    return true;
  }

  public void TerminalReturned()
  {
    // `Free` clears the handle, so this reads false the second time. What it guards is the
    // throw: `GCHandle.Free` on a slot already freed raises rather than freeing another's, and
    // the terminal is delivered once per call, so nothing should reach this twice anyway.
    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }

  internal Task SendUnaryAsync<TRequest>(Marshaller<TRequest> marshaller,
                                         TRequest request)
    => Sent(marshaller,
            request,
            halfClose: true);

  /// <summary>One message of a client stream, complete when the engine has acquitted it.</summary>
  /// <remarks>The acquittal and not the commit, because that is what frees the send window: a
  /// caller honouring <c>IClientStreamWriter</c>'s one-writer contract therefore always finds the
  /// window open at its next lend, whatever depth the ABI allows a host that pipelines deeper.
  /// </remarks>
  internal async Task WriteAsync<TRequest>(Marshaller<TRequest> marshaller,
                                           TRequest request)
  {
    var acquitted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    Volatile.Write(ref writing_,
                   acquitted);

    await Sent(marshaller,
               request,
               halfClose: false)
      .ConfigureAwait(false);

    // The terminal is watched beside the acquittal because a write left pending would hang the
    // caller. Level 1 emits every acquittal before the terminal, so a call that reaches its
    // terminal first is an engine that broke that promise, and the caller hears it as the status.
    var settled = await Task.WhenAny(acquitted.Task,
                                     terminal_.Task)
                            .ConfigureAwait(false);
    if (settled != acquitted.Task)
    {
      throw new RpcException(await terminal_.Task.ConfigureAwait(false),
                             trailers_);
    }
  }

  private async Task Sent<TRequest>(Marshaller<TRequest> marshaller,
                                    TRequest request,
                                    bool halfClose)
  {
    try
    {
      await SendingAsync(marshaller,
                         request,
                         halfClose)
        .ConfigureAwait(false);
    }
    catch
    {
      ending_.Cancel();
      throw;
    }
  }

  private async Task SendingAsync<TRequest>(Marshaller<TRequest> marshaller,
                                            TRequest request,
                                            bool halfClose)
  {
    Interlocked.Increment(ref holding_);
    try
    {
      await HoldingABufferAsync(marshaller,
                                request,
                                halfClose)
        .ConfigureAwait(false);
    }
    finally
    {
      if (Interlocked.Decrement(ref holding_) == 0)
      {
        handedBack_.Set();
      }
    }
  }

  private async Task HoldingABufferAsync<TRequest>(Marshaller<TRequest> marshaller,
                                                   TRequest request,
                                                   bool halfClose)
  {
    using var lent = new LentBuffer(handle_);
    marshaller.ContextualSerializer(request,
                                    lent);

    while (true)
    {
      // Counted once the engine has taken it, so a refusal leaves nothing to acquit. The WRITE_DONE
      // may land before this returns and drive the count below zero; `RunAsync` reads the count
      // only once this method has returned, which is why it reads the sum and not a moment of it.
      var status = lent.Commit();
      if (status == NativeMethods.AkStatus.Ok)
      {
        Interlocked.Increment(ref inFlight_);
        break;
      }

      if (status is NativeMethods.AkStatus.InvalidState or NativeMethods.AkStatus.HandleStale)
      {
        throw new CallEnded(status);
      }

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
        throw new RpcException(new Status(StatusCode.Cancelled,
                                          "the call ended while its send waited for room against the ceiling"));
      }
    }

    if (halfClose)
    {
      HalfClose();
    }
  }

  /// <summary>Says nothing more is coming.</summary>
  internal void HalfClose()
  {
    // The two the engine answers for a call that is already over, which the sender cannot rule
    // out and which the terminal reports anyway.
    var closed = NativeMethods.ak_call_end_send(handle_);
    if (closed is not (NativeMethods.AkStatus.Ok or NativeMethods.AkStatus.HandleStale
                                                 or NativeMethods.AkStatus.InvalidState))
    {
      throw Failed($"the half-close was refused ({closed})");
    }
  }

  // ---- the reader machine ---------------------------------------------------------------
  //
  // One consumer of the delivery ring at a time, and which one is decided by a transition
  // rather than by a peek: a read moves the reader from `waiting` to `parsing`, and that move
  // is what confers ownership, so the drain's handoff - which takes the ring only from a
  // reader that holds no slot - and a read in flight can never both believe they hold the
  // tail. `Phase` is the model's `reader_state` extended with the drain, so the arbiter is one
  // word and there is nothing to keep consistent between two.

  private enum Phase
  {
    Idle,
    Waiting,
    Parsing,
    Draining,
    Finished,
  }

  /// <summary>The reader's phase and the read that owns it, as one value.</summary>
  /// <remarks>One store rather than two: a token callback that saw the new phase against the
  /// previous read, or the reverse, is the stale attribution the ordering exists to
  /// prevent.</remarks>
  private sealed class Reading
  {
    internal Reading(Phase phase,
                     ReadOp? op)
    {
      Phase = phase;
      Op    = op;
    }

    internal Phase Phase { get; }

    internal ReadOp? Op { get; }
  }

  private Reading reading_ = new(Phase.Idle,
                                 null);

  /// <summary>A drain is owed, and takes the ring as soon as no read holds a slot.</summary>
  private int drainOwed_;

  private readonly TaskCompletionSource<bool> settled_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private Task? settling_;

  /// <summary>What a read finds when it reaches for the tail.</summary>
  /// <remarks>An empty ring is not a lost race, and a bool cannot say so: nothing published yet
  /// means wait, the drain holding the ring means end exceptionally.</remarks>
  private enum Claim
  {
    Acquired,
    Empty,
    Lost,
  }

  /// <summary>One read, and the single arbiter between its result and its token.</summary>
  private sealed class ReadOp
  {
    private readonly NativeCall<TResponse> call_;

    private int state_;

    internal ReadOp(NativeCall<TResponse> call)
      => call_ = call;

    /// <summary>The token fired, so cancel the call - that is what MoveNext(token) means.</summary>
    /// <remarks>Only if this read had not already completed, so a token arriving after the fact
    /// cancels nothing. It never waits for the marshaller: the disarm may already be waiting for
    /// this callback, and waiting back would close the cycle.</remarks>
    internal void Fire()
    {
      if (Interlocked.CompareExchange(ref state_,
                                      2,
                                      0) == 0)
      {
        call_.CancelAndDrain();
      }
    }

    internal bool TryWin()
      => Interlocked.CompareExchange(ref state_,
                                     1,
                                     0) == 0;
  }

  /// <summary>The message the last <see cref="MoveNext" /> that answered true decoded.</summary>
  internal TResponse Current { get; private set; } = default!;

  /// <summary>Completes when the terminal has been consumed and the engine is owed nothing.</summary>
  internal Task Settled
    => settling_ ?? throw new InvalidOperationException("the call was not started");

  /// <summary>The next event of the response, or false at its clean end.</summary>
  /// <remarks>Every issue - a message, a clean end, a failed end, a cancellation, a decode
  /// failure - leaves by one path, because they share the acquittal.</remarks>
  internal async Task<bool> MoveNext(CancellationToken token)
  {
    // Published before the registration, because Register runs the callback inline when the
    // token is already cancelled: it has to find THIS read and not the one before it.
    // TLA: BeginMoveNext
    var op = new ReadOp(this);
    while (true)
    {
      var seen = Volatile.Read(ref reading_);
      switch (seen.Phase)
      {
        case Phase.Finished:
          // A stream that ended answers the same thing however often it is asked. Publishing a
          // read here would wait on an event that can no longer come, and would overwrite the
          // phase that says so.
          return Answered();

        case Phase.Draining:
          // The drain holds the ring, which only a cancelled or abandoned call does.
          throw Cancelled();

        case Phase.Waiting:
        case Phase.Parsing:
          // IAsyncStreamReader admits one read at a time, and the phase is where that shows.
          throw new InvalidOperationException("a read is already in flight on this call");
      }

      if (Interlocked.CompareExchange(ref reading_,
                                      new Reading(Phase.Waiting,
                                                  op),
                                      seen) == seen)
      {
        break;
      }
    }

    CancellationTokenRegistration registration;
    try
    {
      registration = token.Register(static state => ((ReadOp)state!).Fire(),
                                    op);
    }
    catch
    {
      // Register throws when the caller's source is already disposed. Nothing is armed, so
      // nothing is disarmed - but a read is published with no one to run it, and the next
      // MoveNext would see a phantom concurrent read.
      AbortWaitingRead(op);
      throw;
    }

    Slot slot;
    try
    {
      // The transition is the only thing that confers ownership - the signal is a wake-up and
      // grants nothing - and it is latched, so a publication landing between the empty
      // observation and the wait is not lost. Leaving `waiting` is the reaction's job, not the
      // waiter's: whatever cancels sets the signal, and the claim then answers Lost.
      // TLA: BeginParseEvent against HandoffToDrain
      while (true)
      {
        var claim = TryBeginParse(op,
                                  out slot);
        if (claim == Claim.Acquired)
        {
          break;
        }

        if (claim == Claim.Lost)
        {
          throw Cancelled();
        }

        await arrived_.WaitAsync()
                      .ConfigureAwait(false);
      }
    }
    catch
    {
      // Any exit before ownership settles the same way and then drains: the read is retracted,
      // and a call whose reader is gone while the call is still active has nobody left to
      // collect what the engine published. Both helpers are idempotent, so on a lost claim they
      // find the drain already holding the ring and change nothing.
      registration.Dispose();
      AbortWaitingRead(op);
      CancelAndDrain();
      throw;
    }

    // NO finally around the block above: a finally runs on the normal exit too, which would
    // disarm the registration the moment the slot is claimed and delete the whole
    // cancellation-during-decode race. The registration stays armed until the marshaller has
    // returned.

    // The slot is this read's from here, and the borrow lasts exactly as long as the decode:
    // native bytes are readable while the reader is parsing and not after.
    // TLA: ParsingReadOwnsItsSlot
    var terminal = slot.Kind == NativeMethods.AkEventKind.Status;
    var metadata = slot.Kind == NativeMethods.AkEventKind.InitialMetadata;
    TResponse? message = null;
    Status? end = null;
    Exception? decodeFailure = null;

    try
    {
      // Branch on the sum BEFORE any marshaller runs. A terminal slot carries the status, the
      // trailers and an error message - never a message of the response type - so handing it to
      // that marshaller decodes the wrong format.
      if (terminal)
      {
        end = DecodedStatus(slot);
      }
      else if (metadata)
      {
        headers_.TrySetResult(RawMetadata.Decode(Bytes(slot.Payload)));
      }
      else
      {
        message = marshaller_.ContextualDeserializer(new ReceivedMessage(slot.Payload));
      }
    }
    catch (Exception thrown)
    {
      // Remembered, not thrown: the slot is ours and the release below is what pays for it.
      decodeFailure = thrown;
    }

    // A terminal always yields a terminal outcome, even when its decode failed. After the
    // release nobody can produce one - the event is gone and its trailers with it - so the
    // status, the drain and the settlement would wait forever on something no step can resolve.
    if (terminal && end is null)
    {
      end = new Status(StatusCode.Internal,
                       $"the call's terminal could not be read: {decodeFailure?.Message}");
    }

    bool won;
    try
    {
      // Disarm before deciding: Dispose returns only once no callback of this registration runs
      // or ever will, so the winner is settled and cannot change under the decision. One
      // arbiter decides between the token and everything else, a decode failure included.
      registration.Dispose();
      won = op.TryWin();
    }
    finally
    {
      // One step of the machine, so nothing leaves between its parts: resolve the status, acquit
      // the slot exactly once, republish the reader and pump a drain that is owed. Guaranteed
      // even if the disarm above throws, and nothing here still reaches the native payload.
      // TLA: FinishConsumePayload if this read won, FinishCancelledParse if the token did, then
      // HandoffToDrain
      if (end is not null)
      {
        Resolve(end.Value);
      }

      NativeMethods.ak_event_consumed(slot.Payload);
      tail_++;
      PublishIdleOrFinished(terminal);
    }

    // Only now the public result, and only for the winner.
    if (!won)
    {
      throw Cancelled();
    }

    if (terminal)
    {
      // The terminal answers from the status, a failed decode included: the synthetic value is
      // what every later read and the call's status report, and the read that consumed the
      // terminal must not answer something else. A stable terminal result is what
      // IAsyncStreamReader promises.
      if (end!.Value.StatusCode != StatusCode.OK)
      {
        throw new RpcException(end.Value,
                               trailers_);
      }

      return false;
    }

    if (decodeFailure is not null)
    {
      // Bytes that will not decode leave the stream unusable, so this faults the call as well as
      // the read. The token had its chance at the same arbiter and lost; there is no second
      // policy.
      CancelAndDrain();
      ExceptionDispatchInfo.Capture(decodeFailure)
                           .Throw();
    }

    if (metadata)
    {
      // The prologue is consumed inside a read, under that read's own registration, so a token
      // firing while the head is outstanding faults the read and the headers together rather
      // than finding no operation to cancel. It is not a result, so this read continues.
      return await MoveNext(token)
               .ConfigureAwait(false);
    }

    Current = message!;
    return true;
  }

  /// <summary>Moves the reader from waiting to parsing, which is what takes the slot.</summary>
  private Claim TryBeginParse(ReadOp op,
                              out Slot slot)
  {
    slot = default;

    var seen = Volatile.Read(ref reading_);
    if (seen.Phase != Phase.Waiting || seen.Op != op)
    {
      // The reaction moved the reader out of `waiting`, or the drain holds the ring.
      return Claim.Lost;
    }

    if (Volatile.Read(ref head_) == tail_)
    {
      return Claim.Empty;
    }

    if (Interlocked.CompareExchange(ref reading_,
                                    new Reading(Phase.Parsing,
                                                op),
                                    seen) != seen)
    {
      return Claim.Lost;
    }

    slot = ring_[(int)(tail_ & mask_)];
    return Claim.Acquired;
  }

  /// <summary>Retracts a read that never took a slot.</summary>
  private void AbortWaitingRead(ReadOp op)
  {
    var seen = Volatile.Read(ref reading_);
    if (seen.Phase == Phase.Waiting && seen.Op == op)
    {
      Interlocked.CompareExchange(ref reading_,
                                  new Reading(Phase.Idle,
                                              null),
                                  seen);
    }
  }

  /// <summary>Publishes the reader after a read, then hands the ring over if a drain is owed.</summary>
  private void PublishIdleOrFinished(bool terminal)
  {
    Volatile.Write(ref reading_,
                   new Reading(terminal
                                 ? Phase.Finished
                                 : Phase.Idle,
                               null));

    if (terminal)
    {
      settled_.TrySetResult(true);
    }

    HandoffToDrain();
  }

  /// <summary>Ends the call and makes sure someone collects what it published.</summary>
  /// <remarks>Idempotent, non-blocking and non-throwing, because a token's callback runs it and
  /// the disarm may already be waiting for that callback. Its reaction is per phase: a waiting
  /// reader is faulted and loses the ring; a parsing one keeps its slot, which stays the
  /// marshaller's until it returns, and the drain is owed instead.</remarks>
  internal void CancelAndDrain()
  {
    Volatile.Write(ref drainOwed_,
                   1);
    ending_.Cancel();

    var seen = Volatile.Read(ref reading_);
    if (seen.Phase == Phase.Waiting)
    {
      // TLA: CancelWaitingRead
      Interlocked.CompareExchange(ref reading_,
                                  new Reading(Phase.Idle,
                                              null),
                                  seen);
      FailHead(Cancelled());
    }

    HandoffToDrain();

    // Last, so a waiter that observed an empty ring before any of this wakes and sees the claim
    // lost. The signal grants nothing; it is only what stops the wait.
    arrived_.Set();
  }

  /// <summary>Gives the ring to a drain, if one is owed and no read holds a slot.</summary>
  private void HandoffToDrain()
  {
    if (Volatile.Read(ref drainOwed_) == 0)
    {
      return;
    }

    var seen = Volatile.Read(ref reading_);
    if (seen.Phase is Phase.Parsing or Phase.Draining or Phase.Finished)
    {
      // A parse in flight owns its slot; a drain already holds the ring; a finished reader has
      // consumed the terminal and there is nothing left to collect.
      return;
    }

    if (Interlocked.CompareExchange(ref reading_,
                                    new Reading(Phase.Draining,
                                                null),
                                    seen) != seen)
    {
      return;
    }

    _ = Task.Run(DrainAsync);
  }

  /// <summary>Consumes what the application will not, so the call can settle.</summary>
  private async Task DrainAsync()
  {
    while (true)
    {
      while (Volatile.Read(ref head_) == tail_)
      {
        await arrived_.WaitAsync()
                      .ConfigureAwait(false);
      }

      var slot = ring_[(int)(tail_ & mask_)];
      var terminal = slot.Kind == NativeMethods.AkEventKind.Status;

      try
      {
        if (terminal)
        {
          Resolve(DecodedStatus(slot));
        }
        else if (slot.Kind == NativeMethods.AkEventKind.InitialMetadata)
        {
          headers_.TrySetResult(RawMetadata.Decode(Bytes(slot.Payload)));
        }
      }
      catch (Exception thrown)
      {
        if (terminal)
        {
          Resolve(new Status(StatusCode.Internal,
                             $"the call's terminal could not be read: {thrown.Message}"));
        }
      }
      finally
      {
        NativeMethods.ak_event_consumed(slot.Payload);
        tail_++;
      }

      if (terminal)
      {
        Volatile.Write(ref reading_,
                       new Reading(Phase.Finished,
                                   null));
        settled_.TrySetResult(true);
        return;
      }
    }
  }

  private Status DecodedStatus(in Slot slot)
  {
    RawMetadata.DecodeStatus(Bytes(slot.Payload),
                             out var reason,
                             out var trailers);
    trailers_ = trailers;

    return new Status((StatusCode)slot.Status,
                      reason);
  }

  private void Resolve(Status ended)
  {
    terminal_.TrySetResult(ended);

    if (ended.StatusCode == StatusCode.OK)
    {
      headers_.TrySetResult(Metadata.Empty);
    }
    else
    {
      FailHead(new RpcException(ended,
                                trailers_));
    }
  }

  /// <summary>The terminal, as every read past the end and the call's status report it.</summary>
  /// <remarks>Read without awaiting: the status is resolved in the same guaranteed block that
  /// publishes the finished phase, so a reader that observed that phase can see the value.
  /// </remarks>
  private bool Answered()
  {
    var ended = terminal_.Task.GetAwaiter()
                         .GetResult();
    if (ended.StatusCode != StatusCode.OK)
    {
      throw new RpcException(ended,
                             trailers_);
    }

    return false;
  }

  private static RpcException Cancelled()
    => new(new Status(StatusCode.Cancelled,
                      "the call was cancelled"));

  /// <summary>The one message a single-response cardinality answers with.</summary>
  /// <remarks>There is no second read path: unary, client streaming and any other cardinality
  /// that answers once take the same reader as a server stream and reduce it to a single - one
  /// message, then a terminal, and anything else is a server that did not honour the
  /// cardinality. What differs between them is what they send, not how they read.</remarks>
  private async Task<TResponse> SingleAsync()
  {
    TResponse response;
    try
    {
      // MoveNext rethrows a decode failure as it stands, because that is what a stream reader
      // owes its caller. This surface owes the binding's one public rule instead: whatever went
      // wrong, a caller of a single-response cardinality reads it as an RpcException.
      try
      {
      if (!await MoveNext(CancellationToken.None)
             .ConfigureAwait(false))
      {
        throw new RpcException(new Status(StatusCode.Internal,
                                          "a unary call answered with no message"),
                               trailers_);
      }

      response = Current;

      if (await MoveNext(CancellationToken.None)
            .ConfigureAwait(false))
      {
        // The terminal is still unconsumed, so somebody has to collect it or the call never
        // settles and the channel's drain waits on it for good.
        CancelAndDrain();
        throw new RpcException(new Status(StatusCode.Internal,
                                          "a unary call answered with more than one message"),
                               trailers_);
      }
      }
      catch (Exception thrown) when (thrown is not RpcException)
      {
        throw new RpcException(new Status(StatusCode.Internal,
                                          $"the call's events could not be read: {thrown.Message}"),
                               trailers_);
      }
    }
    finally
    {
      // A call that still holds a buffer is not settled, whatever its terminal says: the
      // channel's drain waits on this, so completing early would let `ak_channel_release` and
      // then the runtime's shutdown run while the engine is still owed what the serializer has.
      await Settled.ConfigureAwait(false);
    }

    // Reachable now, and read where the sender is known to have finished: it raises the count
    // and only then lets `holding_` fall, and `Settled` waits on that. Read from the reader the
    // count could be -1 - a WRITE_DONE that landed while the sender was off the CPU between its
    // P/Invoke returning and its own increment - and a call the server answered OK would fail.
    var unacquitted = Volatile.Read(ref inFlight_);
    if (unacquitted != 0)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the call reached its terminal with {unacquitted} send(s) unacquitted"),
                             trailers_);
    }

    return response;
  }

  private async Task SettlingAsync()
  {
    await settled_.Task.ConfigureAwait(false);

    while (Volatile.Read(ref holding_) != 0)
    {
      await handedBack_.WaitAsync()
                       .ConfigureAwait(false);
    }

    ending_.Cancel();
    cancellation_.Dispose();
  }

  /// <summary>Ends this call when the caller's token is cancelled.</summary>
  /// <remarks>Registered on <c>ending_</c> rather than on the token directly, so it is set up
  /// before the call is started and cannot race the reader disposing it: a call the engine ends
  /// at once - a channel released underneath it - reaches its terminal and disposes
  /// <c>cancellation_</c> while the invoker is still on its way here, and the registration would
  /// then outlive the call for as long as the caller's token source does. A token already
  /// cancelled cancels immediately, which is what a caller passing one expects.</remarks>
  internal void CancelWith(CancellationToken token)
  {
    if (!token.CanBeCanceled)
    {
      return;
    }

    var registration = token.Register(Cancel);
    if (ending_.IsCancellationRequested)
    {
      registration.Dispose();
      return;
    }

    cancellation_ = registration;
  }

  /// <summary>Ends the call, and makes sure what it already published is collected.</summary>
  /// <remarks>The drain is the whole point: a caller that disposes a response stream half read
  /// leaves events behind, and nothing else would take them - the channel's disposal waits for
  /// every call to have settled, so a cancel that only ended the call would hang it.</remarks>
  public void Cancel()
    => CancelAndDrain();

  private void EndNative()
  {
    if (!terminal_.Task.IsCompleted)
    {
      NativeMethods.ak_call_cancel(handle_);
    }
  }

  private static ReadOnlySpan<byte> Bytes(in NativeMethods.AkBytes payload)
    => UnmanagedMemoryManager.Span(payload.Ptr,
                                   payload.Len);

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
