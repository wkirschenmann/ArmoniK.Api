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
using System.Runtime.ExceptionServices;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>One call's inbound half: what the engine delivered, and who is entitled to read it.</summary>
///
/// It owns the delivery queue and everything the response is made of - the headers, the status and
/// the trailers - because it is what produces them. The call forwards to it whatever a caller asks
/// of the response.
internal sealed class Receiver<TResponse>
  where TResponse : class
{
  private readonly ICallState call_;
  private readonly DeliveryRing delivered_;
  private readonly Marshaller<TResponse> marshaller_;
  private readonly int credits_;

  private readonly TaskCompletionSource<Metadata> headers_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly TaskCompletionSource<Status> terminal_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private Metadata trailers_ = Metadata.Empty;

  // Where the head came from, once one of the ring's consumers has taken it: what the terminal
  // answers the headers with when the head did not.
  private ak_head_origin headOrigin_ = ak_head_origin.AK_HEAD_RECEIVED;

  internal Receiver(ICallState call,
                    int deliveryCredits,
                    Marshaller<TResponse> marshaller)
  {
    call_       = call;
    marshaller_ = marshaller;
    credits_    = deliveryCredits;
    delivered_  = new DeliveryRing(deliveryCredits);
  }

  /// <summary>The response headers, which starts the task that answers them.</summary>
  internal Task<Metadata> ResponseHeadersAsync
  {
    get
    {
      StartPrologue();
      return headers_.Task;
    }
  }

  internal Task<Status> TerminalAsync
    => terminal_.Task;

  internal Metadata Trailers
    => trailers_;

  /// <summary>Starts consuming the initial metadata, on no read's behalf, once: when a caller
  /// asks for the headers and nothing has taken the head yet.</summary>
  /// <remarks>Only from <c>Idle</c>, which is the phase a reader waiting for a single response
  /// leaves the ring in, so a head that arrives alone answers the headers at once. With a read in
  /// flight, the read takes the head, and with the head taken, the headers are answered.</remarks>
  internal void StartPrologue()
  {
    if (Interlocked.Exchange(ref prologueAsked_,
                             1) != 0)
    {
      return;
    }

    // Before the phase: a reader that sees `Prologue` then waits on the task that ends it.
    Volatile.Write(ref prologueStarted_,
                   true);
    var seen = Volatile.Read(ref reading_);
    if (seen.Phase != Phase.Idle || Volatile.Read(ref headTaken_) != 0 || Interlocked.CompareExchange(ref reading_,
                                                                                                      new Reading(Phase.Prologue,
                                                                                                                  null),
                                                                                                      seen) != seen)
    {
      prologueEnded_.TrySetResult(true);
      return;
    }

    _ = RunPrologueAsync();
  }

  private async Task RunPrologueAsync()
  {
    try
    {
      await PrologueAsync()
        .ConfigureAwait(false);
    }
    finally
    {
      prologueEnded_.TrySetResult(true);
    }
  }

  /// <summary>The prologue, once it has let go of the queue; completed when there is none.</summary>
  internal Task PrologueFinished
    => Volatile.Read(ref prologueStarted_)
         ? prologueEnded_.Task
         : Task.CompletedTask;

  /// <summary>Takes an event without waking the reader, which <see cref="Arrived" /> does once a
  /// callback has stored all it carries, and answers whether returning its payload is now this
  /// half's obligation.</summary>
  internal bool Store(ak_event_kind kind,
                      in ak_bytes payload,
                      int statusCode)
  {
    delivered_.Store(kind,
                     payload,
                     statusCode);
    return true;
  }

  internal void Arrived()
    => delivered_.Arrived();

  /// <summary>Whether every event stored has been given back: what the engine's delivery window
  /// waits for, read by the tests that play the engine.</summary>
  internal bool AllGivenBack
    => delivered_.IsEmpty;

  private void FailHead(RpcException reason)
  {
    if (headers_.TrySetException(reason))
    {
      // Read so it counts as observed: a caller that only awaits the response never reads the
      // head, and an unobserved exception is raised again from the finalizer thread.
      _ = headers_.Task.Exception;
    }
  }

  // ---- the reader machine ---------------------------------------------------------------
  //
  // One consumer of the delivery ring at a time, and which one is decided by a transition
  // rather than by a peek: a read moves the reader from `waiting` to `parsing`, and that move
  // is what confers ownership, so the drain's handoff - which takes the ring only from a
  // reader that holds no slot - and a read in flight can never both believe they hold the
  // tail. `Phase` carries the model's `reader_state`, its `consumer_phase` and the drain in one
  // word, so the arbiter is one word and there is nothing to keep consistent between two.
  //
  // `Prologue` is the model's `consumer_phase = "prologue"`, and holding the ring is what makes
  // it exclusive. The model lets a read be `waiting` while the phase is still the prologue; this
  // does not, which is fewer behaviours and the same invariants.

  private enum Phase
  {
    Prologue,
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

  private int prologueAsked_;
  private bool prologueStarted_;

  private readonly TaskCompletionSource<bool> prologueEnded_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  /// <summary>1 once a consumer of the ring has taken the head.</summary>
  private int headTaken_;

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
    private readonly Receiver<TResponse> reading_;

    private int state_;

    internal ReadOp(Receiver<TResponse> reading)
      => reading_ = reading;

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
        reading_.CancelAndDrain();
      }
    }

    internal bool TryWin()
      => Interlocked.CompareExchange(ref state_,
                                     1,
                                     0) == 0;
  }

  /// <summary>The message the last <see cref="MoveNext" /> that answered true decoded.</summary>
  internal TResponse Current { get; private set; } = default!;

  /// <summary>Completes when the terminal has been consumed, which the settlement waits on
  /// before it asks the sender whether anything is still lent.</summary>
  internal Task Settled
    => settled_.Task;

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
        case Phase.Prologue:
          // Waiting for the phase rather than publishing a read behind it is what keeps the
          // arbiter one word, and the token is read here because nothing is published to carry it.
          if (token.IsCancellationRequested)
          {
            CancelAndDrain();
            throw Cancelled();
          }

          // The prologue's end, which is what this read waits for: the ring's arrivals are the
          // prologue's until it lets go. The token ends this wait too, and the check above then
          // reads it.
          await PrologueEndsOr(token)
            .ConfigureAwait(false);

          continue;

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

    DeliveryRing.Slot slot;
    try
    {
      // The transition is the only thing that confers ownership - the signal is a wake-up and
      // grants nothing - and the wait is taken before the claim, so a publication landing
      // between the empty observation and the await sets it. Leaving `waiting` is the
      // reaction's job, not the waiter's: whatever cancels sets the signal, and the claim then
      // answers Lost.
      // TLA: BeginParseEvent against HandoffToDrain
      while (true)
      {
        var arrival = delivered_.NextArrival();
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

#if DEBUG
        if (TestHooks.FoundTheRingEmpty is { } hook)
        {
          await hook(RingConsumer.Reader)
            .ConfigureAwait(false);
        }
#endif
        await arrival.ConfigureAwait(false);
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
    var terminal = slot.Kind == ak_event_kind.AK_EVENT_STATUS;
    var metadata = slot.Kind == ak_event_kind.AK_EVENT_INITIAL_METADATA;
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
        TakeHead(slot);
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
      end = Unreadable(decodeFailure);
      // With no trailers to answer, a head that left the headers to the terminal fails them too.
      FailHead(new RpcException(end.Value));
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

      delivered_.Release();
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

  /// <summary>The one response of a call that declared it, read in as few passes as the events
  /// allow: once the terminal is in, or once the delivery window is full and the engine waits for
  /// the host, the reader takes every event the ring holds and gives their payloads back in one
  /// downcall.</summary>
  /// <remarks>
  ///   It waits outside the reader's phases, so a caller asking for the headers meanwhile starts
  ///   the prologue and is answered by a head that arrives alone; a cancellation hands the ring
  ///   to the drain and wakes it, and it ends with the call's cancellation. Unary and client
  ///   streaming are read this way; a stream is read by <see cref="MoveNext" />.
  /// </remarks>
  internal async Task<TResponse> SingleAsync()
  {
    TResponse? message  = null;
    var        messages = 0;
    Exception? failure  = null;
    while (true)
    {
      var arrival = delivered_.NextArrival();
      var seen    = Volatile.Read(ref reading_);
      switch (seen.Phase)
      {
        case Phase.Finished:
          // Only the drain finishes a call this reader has not read: what it ended with is the
          // answer, and an OK one came without the message the drain discarded.
          Answered();
          throw Cancelled();
        case Phase.Draining:
          throw Cancelled();
        case Phase.Waiting:
        case Phase.Parsing:
          throw new InvalidOperationException("a read is already in flight on this call");
        case Phase.Prologue:
          await PrologueEndsOr(CancellationToken.None)
            .ConfigureAwait(false);
          continue;
      }

      if (!delivered_.HoldsTerminal && delivered_.Count < credits_)
      {
        await arrival.ConfigureAwait(false);
        continue;
      }

      if (Interlocked.CompareExchange(ref reading_,
                                      new Reading(Phase.Parsing,
                                                  new ReadOp(this)),
                                      seen) != seen)
      {
        continue;
      }

      // TLA: BeginParseEvent, then one FinishConsumePayload per event taken
      var     count = delivered_.Count;
      Status? end   = null;
      try
      {
        for (var at = 0; at < count; at++)
        {
          var slot = delivered_.PeekAt(at);
          switch (slot.Kind)
          {
            case ak_event_kind.AK_EVENT_STATUS:
              end = StatusOrUnreadable(slot);
              break;
            case ak_event_kind.AK_EVENT_INITIAL_METADATA:
              TakeHeadOrFailHeaders(slot);
              break;
            default:
              messages++;
              if (messages == 1)
              {
                try
                {
                  message = marshaller_.ContextualDeserializer(new ReceivedMessage(slot.Payload));
                }
                catch (Exception thrown)
                {
                  failure = thrown;
                }
              }

              break;
          }
        }
      }
      finally
      {
        if (end is not null)
        {
          Resolve(end.Value);
        }

        delivered_.ReleaseMany(count);
        PublishIdleOrFinished(end is not null);
      }

      if (end is null)
      {
        continue;
      }

      if (end.Value.StatusCode != StatusCode.OK)
      {
        throw new RpcException(end.Value,
                               trailers_);
      }

      if (failure is not null)
      {
        ExceptionDispatchInfo.Capture(failure)
                             .Throw();
      }

      return messages switch
             {
               0 => throw new RpcException(new Status(StatusCode.Internal,
                                                      "a unary call answered with no message"),
                                           trailers_),
               1 => message!,
               // The engine ends such a call INTERNAL at the second message's first byte; a
               // second one here would be an engine that did not.
               _ => throw new RpcException(new Status(StatusCode.Internal,
                                                      "a unary call answered with more than one message"),
                                           trailers_),
             };
    }
  }

  /// <summary>Moves the reader from waiting to parsing, which is what takes the slot.</summary>
  private Claim TryBeginParse(ReadOp op,
                              out DeliveryRing.Slot slot)
  {
    slot = default;

    var seen = Volatile.Read(ref reading_);
    if (seen.Phase != Phase.Waiting || seen.Op != op)
    {
      // The reaction moved the reader out of `waiting`, or the drain holds the ring.
      return Claim.Lost;
    }

    if (delivered_.IsEmpty)
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

    // Peeked rather than taken: the slot stays at the tail until the parse releases it, which
    // is what leaves a drain something to find if this read never finishes.
    delivered_.TryPeek(out slot);
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
    call_.EndCall();

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
    else if (seen.Phase == Phase.Prologue)
    {
      // The headers have no read to be faulted with, so they are faulted here - the model's
      // `BeginDisposeCall`, which leaves no managed waiter behind. The phase is not moved: the
      // prologue is the one owner of that transition, and it takes it on the signal below.
      FailHead(Cancelled());
    }

    HandoffToDrain();

    // Last, so a waiter that observed an empty ring before any of this wakes and sees the claim
    // lost. The signal grants nothing; it is only what stops the wait.
    delivered_.Wake();
  }

  /// <summary>Gives the ring to a drain, if one is owed and no read holds a slot.</summary>
  private void HandoffToDrain()
  {
    if (Volatile.Read(ref drainOwed_) == 0)
    {
      return;
    }

    var seen = Volatile.Read(ref reading_);
    if (seen.Phase is Phase.Prologue or Phase.Parsing or Phase.Draining or Phase.Finished)
    {
      // The prologue and a parse in flight each own their slot; a drain already holds the ring;
      // a finished reader has consumed the terminal and there is nothing left to collect. The
      // prologue calls this again when it lets go, so a drain owed meanwhile is not lost.
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
  /// <remarks>Nothing may leave this: the drain runs on a task nobody awaits, and `settled_` is
  /// the first thing `SettlingAsync` waits on - so an escape would be a call that never settles
  /// and a channel whose disposal waits for it. Faulted rather than completed, because completing
  /// it says the terminal was consumed.</remarks>
  private async Task DrainAsync()
  {
    try
    {
      await DrainingAsync().ConfigureAwait(false);
    }
    catch (Exception thrown)
    {
      settled_.TrySetException(thrown);
    }
  }

  private async Task DrainingAsync()
  {
    while (true)
    {
      DeliveryRing.Slot slot;
      while (true)
      {
        var arrival = delivered_.NextArrival();
        if (delivered_.TryPeek(out slot))
        {
          break;
        }

#if DEBUG
        if (TestHooks.FoundTheRingEmpty is { } hook)
        {
          await hook(RingConsumer.Drain)
            .ConfigureAwait(false);
        }
#endif
        await arrival.ConfigureAwait(false);
      }

      var terminal = slot.Kind == ak_event_kind.AK_EVENT_STATUS;

      try
      {
        if (terminal)
        {
          Resolve(DecodedStatus(slot));
        }
        else if (slot.Kind == ak_event_kind.AK_EVENT_INITIAL_METADATA)
        {
          TakeHead(slot);
        }
      }
      catch (Exception thrown)
      {
        if (terminal)
        {
          var unreadable = Unreadable(thrown);
          // With no trailers to answer, a head that left the headers to the terminal fails them
          // too.
          FailHead(new RpcException(unreadable));
          Resolve(unreadable);
        }
        else
        {
          // The headers, which the terminal would otherwise answer with an empty set on a call
          // that ended OK - "none" where the truth is "none that could be read".
          FailHead(new RpcException(new Status(StatusCode.Internal,
                                               $"the call's initial metadata could not be read: {thrown.Message}")));
        }
      }
      finally
      {
        delivered_.Release();
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

  /// <summary>The terminal's status, or the one an undecodable terminal reports, which fails the
  /// headers it would have answered.</summary>
  private Status StatusOrUnreadable(in DeliveryRing.Slot slot)
  {
    try
    {
      return DecodedStatus(slot);
    }
    catch (Exception thrown)
    {
      var unreadable = Unreadable(thrown);
      FailHead(new RpcException(unreadable));
      return unreadable;
    }
  }

  /// <summary>Takes the head, or fails the headers with why it could not be read: the rest of the
  /// response is still the reader's.</summary>
  private void TakeHeadOrFailHeaders(in DeliveryRing.Slot slot)
  {
    try
    {
      TakeHead(slot);
    }
    catch (Exception thrown)
    {
      FailHead(new RpcException(new Status(StatusCode.Internal,
                                           "the response metadata could not be read",
                                           thrown)));
    }
  }

  /// <summary>The status a terminal nobody could decode reports, wherever it was consumed.</summary>
  /// <remarks>A terminal always yields one: after the slot is released the trailers are gone, so
  /// a call left without a status would keep every later read, the drain and the settlement
  /// waiting on something no step can produce.</remarks>
  private static Status Unreadable(Exception? thrown)
    => new(StatusCode.Internal,
           $"the call's terminal could not be read: {thrown?.Message}");

  private Status DecodedStatus(in DeliveryRing.Slot slot)
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
    AnswerHeadsAt(ended);
  }

  /// <summary>Takes the head's origin, and answers the headers when the peer's came.</summary>
  /// <remarks>A value this binding does not know is taken for the peer's headers, as the ABI
  /// promises a host that ignores the field.</remarks>
  private void TakeHead(in DeliveryRing.Slot head)
  {
    Volatile.Write(ref headTaken_,
                   1);
    var origin = (ak_head_origin)head.Status;
    headOrigin_ = origin is ak_head_origin.AK_HEAD_TRAILERS_ONLY or ak_head_origin.AK_HEAD_NO_RESPONSE
                    ? origin
                    : ak_head_origin.AK_HEAD_RECEIVED;

    if (headOrigin_ == ak_head_origin.AK_HEAD_RECEIVED)
    {
      headers_.TrySetResult(RawMetadata.Decode(Bytes(head.Payload)));
    }
  }

  /// <summary>The headers the head did not answer, answered at the terminal as grpc-dotnet
  /// answers them.</summary>
  /// <remarks>A response that delivered no head answers them with its trailers, whatever its
  /// status; no response, with the call's status. With no head taken at all, an OK call answers
  /// none and any other its status.</remarks>
  private void AnswerHeadsAt(Status ended)
  {
    if (headOrigin_ == ak_head_origin.AK_HEAD_TRAILERS_ONLY)
    {
      headers_.TrySetResult(trailers_);
    }
    else if (ended.StatusCode == StatusCode.OK)
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


  private async Task PrologueEndsOr(CancellationToken token)
  {
    var cancelled = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    using (token.Register(static source => ((TaskCompletionSource<bool>)source!).TrySetResult(true),
                          cancelled))
    {
      await Task.WhenAny(PrologueFinished,
                         cancelled.Task)
                .ConfigureAwait(false);
    }
  }

  /// <summary>Answers the headers from the terminal the prologue peeked at, which stays the
  /// ring's.</summary>
  private void AnswerHeadsFrom(in DeliveryRing.Slot terminal)
  {
    if (terminal.Kind != ak_event_kind.AK_EVENT_STATUS)
    {
      FailHead(new RpcException(new Status(StatusCode.Internal,
                                           "a message followed a head that said no response body would come")));
      return;
    }

    try
    {
      AnswerHeadsAt(DecodedStatus(terminal));
    }
    catch (Exception thrown)
    {
      FailHead(new RpcException(new Status(StatusCode.Internal,
                                           "the response trailers could not be read",
                                           thrown)));
    }
  }

  /// <summary>Consumes the initial metadata and resolves the headers, on no read's behalf.</summary>
  /// <remarks>The model's <c>ConsumeHeader</c>: what owns slot 0 is the phase and not a read, so
  /// <see cref="ResponseHeadersAsync" /> answers whether or not the caller is pumping the reader.
  /// A task rather than part of <c>Publish</c>, because the header forbids parsing on the
  /// callback's thread.</remarks>
  private async Task PrologueAsync()
  {
    try
    {
      while (true)
      {
        var arrival = delivered_.NextArrival();
        if (!delivered_.IsEmpty)
        {
          break;
        }

        if (call_.Ending.IsCancellationRequested)
        {
          // Cancelled or settled with nothing published. Whatever arrives now is the drain's.
          return;
        }

#if DEBUG
        if (TestHooks.FoundTheRingEmpty is { } hook)
        {
          await hook(RingConsumer.Prologue)
            .ConfigureAwait(false);
        }
#endif
        await arrival.ConfigureAwait(false);
      }

      delivered_.TryPeek(out var slot);
      if (slot.Kind != ak_event_kind.AK_EVENT_INITIAL_METADATA)
      {
        // No head came first, which the ABI does not do. Left for the reader or the drain, and
        // the terminal answers the headers.
        return;
      }

      // The caller hears a head that will not decode from the headers, and the reader still
      // gets the rest of the response.
      TakeHeadOrFailHeaders(slot);

      if (headOrigin_ != ak_head_origin.AK_HEAD_RECEIVED)
      {
        // No headers of the peer's: the terminal comes next and nothing else does, and it
        // answers the headers. Waited for here so that they are answered without a read.
        DeliveryRing.Slot terminal;
        while (true)
        {
          var arrival = delivered_.NextArrival();
          if (delivered_.TryPeekBehind(out terminal))
          {
            break;
          }

          if (call_.Ending.IsCancellationRequested)
          {
            // The call is ending: whoever consumes the ring next takes the head again, and the
            // terminal answers the headers there.
            return;
          }

          await arrival.ConfigureAwait(false);
        }

        AnswerHeadsFrom(terminal);
      }

      delivered_.Release();
    }
    finally
    {
      // Publishing the reader is what releases the tail - the volatile write orders the `tail_`
      // above ahead of any reader that observes the new phase.
      Volatile.Write(ref reading_,
                     new Reading(Phase.Idle,
                                 null));
      HandoffToDrain();
    }
  }

  private static unsafe ReadOnlySpan<byte> Bytes(in ak_bytes payload)
    => UnmanagedMemoryManager.Span(payload.ptr,
                                   payload.len);
}
