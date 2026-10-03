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
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>What a call is to the two halves that drive it.</summary>
///
/// The facts both sides read and neither owns: the handle they downcall on, the end they watch,
/// and the terminal that answers whatever they were waiting for when the call died.
internal interface ICallState
{
  ulong Handle { get; }

  CancellationToken Ending { get; }

  Task<Status> TerminalAsync { get; }

  Metadata Trailers { get; }

  /// <summary>Ends the call, which is what a send that failed does to the rest of it.</summary>
  void EndCall();
}

/// <summary>One call's outbound half: what it has serialized, sent, and not had acquitted.</summary>
///
/// Not generic, because nothing here reads the response type: a request's marshaller arrives with
/// the request. So it is compiled once rather than once per response.
internal sealed class Sender
{
  private readonly ICallState call_;

  private int inFlight_;

  private int holding_;

  private readonly ArrivalSignal handedBack_ = new();

  // Set at each AK_EVENT_BUDGET_WAKE: a release gave bytes back since a send of this call was
  // refused for room.
  private readonly ArrivalSignal woken_ = new();

  // The write waiting for its acquittal, or null between writes, and the claim a writer takes
  // to become the one. A write linearizes at its WRITE_DONE and not at the commit, which is what
  // lets one writer send in a row against a window of one: the emission that completes a write
  // has already freed the slot the next lend asks for, so the window is open at every lend a
  // holder of this claim makes.
  private TaskCompletionSource<bool>? writing_;

  internal Sender(ICallState call)
    => call_ = call;

  /// <summary>What the engine has taken and not yet acquitted.</summary>
  internal int Unacquitted
    => Volatile.Read(ref inFlight_);

  /// <summary>A WRITE_DONE: one send has left, and its writer may return.</summary>
  internal void Acquitted()
  {
    Interlocked.Decrement(ref inFlight_);
    Volatile.Read(ref writing_)
            ?.TrySetResult(true);
  }

  /// <summary>An AK_EVENT_BUDGET_WAKE: a send refused for room may find it now.</summary>
  internal void Woken()
    => woken_.Set();

  /// <summary>Waits until no serializer holds a buffer of the engine's.</summary>
  /// <remarks>What the settlement waits on: a call still holding a buffer is not settled,
  /// whatever its terminal says.</remarks>
  internal async Task HandedEverythingBackAsync()
  {
    while (true)
    {
      var handedBack = handedBack_.Next();
      if (Volatile.Read(ref holding_) == 0)
      {
        return;
      }

      await handedBack.ConfigureAwait(false);
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
  /// <exception cref="InvalidOperationException">
  ///   A write is already waiting for its acquittal. Refused rather than raced: a second writer
  ///   publishing its own acquittal would leave the first waiting on one nothing completes, so the
  ///   write that broke no rule would hang until the call ended. Thrown from here rather than
  ///   through the task, like the closed-stream refusal it sits behind - both name a rule the
  ///   caller broke, and neither is an outcome of the RPC.
  /// </exception>
  internal Task WriteAsync<TRequest>(Marshaller<TRequest> marshaller,
                                     TRequest request)
  {
    var acquitted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);

    if (Interlocked.CompareExchange(ref writing_,
                                    acquitted,
                                    null) is not null)
    {
      throw new InvalidOperationException("a write is already in flight on this call");
    }

    return Writing(marshaller,
                   request,
                   acquitted);
  }

  private async Task Writing<TRequest>(Marshaller<TRequest> marshaller,
                                       TRequest request,
                                       TaskCompletionSource<bool> acquitted)
  {
    try
    {
      await Sent(marshaller,
                 request,
                 halfClose: false)
        .ConfigureAwait(false);

      // The terminal is watched beside the acquittal because a write left pending would hang the
      // caller. Level 1 emits every acquittal before the terminal, so a call that reaches its
      // terminal first is an engine that broke that promise, and the caller hears it as the status.
      var settled = await Task.WhenAny(acquitted.Task,
                                       call_.TerminalAsync)
                              .ConfigureAwait(false);
      if (settled != acquitted.Task)
      {
        throw new RpcException(await call_.TerminalAsync.ConfigureAwait(false),
                               call_.Trailers);
      }
    }
    finally
    {
      // Released for the next write, and only if it is still this one's: exchanging on the value
      // rather than storing null is what keeps a write that gave up its claim from clearing the
      // claim of the write after it.
      Interlocked.CompareExchange(ref writing_,
                                  null,
                                  acquitted);
    }
  }

  /// <summary>Says nothing more is coming.</summary>
  internal unsafe void HalfClose()
  {
    ak_error error = default;
    var closed = NativeMethods.ak_call_end_send(call_.Handle,
                                                &error);
    if (closed == ak_status.AK_STATUS_OK)
    {
      return;
    }

    // Read even when not thrown, because every message is owed back.
    var why = error.Take();

    // The two the engine answers for a call that is already over, which the sender cannot rule
    // out and which the terminal reports anyway.
    if (closed is not (ak_status.AK_STATUS_HANDLE_STALE or ak_status.AK_STATUS_INVALID_STATE))
    {
      throw Failed($"the half-close was refused ({closed}): {why}");
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
      call_.EndCall();
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
    // One whole attempt per turn, serialization included: the lend is asked for at the announced
    // length, before a byte is written, so a ceiling with no room is met there and not halfway
    // through. Serializing again costs a second pass over a message that has not changed, and it
    // is what lets the ceiling be waited on instead of allocated around.
    while (true)
    {
      // Taken before the lend, so a wake-up raised between its refusal and the wait below is not
      // lost: the engine owes one only to a send it has already refused.
      var woken = woken_.Next();

      using var lent = new LentBuffer(call_.Handle);

      ak_status status;
      try
      {
        marshaller.ContextualSerializer(request,
                                        lent);

        // Counted once the engine has taken it, so a refusal leaves nothing to acquit. The
        // WRITE_DONE may land before this returns and drive the count below zero; the reduction
        // reads the count only once this method has returned, which is why it reads the sum and
        // not a moment of it.
        status = lent.Commit();
      }
      catch (NoRoomYet)
      {
        status = ak_status.AK_STATUS_BUDGET_BUSY;
      }

      if (status == ak_status.AK_STATUS_OK)
      {
        Interlocked.Increment(ref inFlight_);
        break;
      }

      if (status is ak_status.AK_STATUS_INVALID_STATE or ak_status.AK_STATUS_HANDLE_STALE)
      {
        throw new CallEnded(status);
      }

      if (status != ak_status.AK_STATUS_BUDGET_BUSY)
      {
        throw Failed($"the message was refused ({status})");
      }

      // Tried again at every wake-up, which the ABI asks of a host woken after a refusal: until
      // the send is served or the call ends, the engine holds reads back for it.
      await WokenOrEndedAsync(woken)
        .ConfigureAwait(false);
    }

    if (halfClose)
    {
      HalfClose();
    }
  }

  private async Task WokenOrEndedAsync(Task woken)
  {
    var ending = call_.Ending;
    var ended  = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    using (ending.Register(static source => ((TaskCompletionSource<bool>)source!).TrySetResult(true),
                           ended))
    {
      await Task.WhenAny(woken,
                         ended.Task)
                .ConfigureAwait(false);
    }

    if (ending.IsCancellationRequested)
    {
      throw new RpcException(new Status(StatusCode.Cancelled,
                                        "the call ended while its send waited for room against the ceiling"));
    }
  }

  private static RpcException Failed(string reason)
    => new(new Status(StatusCode.Internal,
                      reason));
}
