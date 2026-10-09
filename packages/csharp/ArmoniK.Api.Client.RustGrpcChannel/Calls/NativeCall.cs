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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

internal interface ICallSink
{
  void TerminalReturned();

  void Cancel();

  /// <summary>Takes one event of a callback, and answers whether returning its payload is now the
  /// consumer's obligation.</summary>
  bool Publish(ak_event_kind kind,
               in ak_bytes payload,
               int statusCode);

  /// <summary>The callback has published every event it carries.</summary>
  void Arrived();
}

internal sealed class NativeCall<TResponse> : ICallSink, ICallState
  where TResponse : class
{
  private readonly Receiver<TResponse> receiving_;

  private readonly Sender sending_;

  private GCHandle self_;
  private ulong handle_;
  private int reduced_;

  private readonly CancellationTokenSource ending_ = new();

  private static readonly uint StartOptionsSize = (uint)Marshal.SizeOf<ak_call_start_options>();

  private static readonly ConditionalWeakTable<string, byte[]> MethodNames = new();

  private readonly object disarm_ = new();

  private CancellationTokenRegistration cancellation_;

  /// <summary>The settlement, kept so its completion is owned rather than dropped.</summary>
  private Task? settling_;

  private NativeCall(int deliveryCredits,
                     Marshaller<TResponse> marshaller,
                     bool oneRequest)
  {
    receiving_ = new Receiver<TResponse>(this,
                                        deliveryCredits,
                                        marshaller);
    sending_ = new Sender(this,
                          oneRequest);

    self_ = GCHandle.Alloc(this);
  }

  internal Task<Metadata> ResponseHeadersAsync
    => receiving_.ResponseHeadersAsync;

  public Task<Status> TerminalAsync
    => receiving_.TerminalAsync;

  public Metadata Trailers
    => receiving_.Trailers;

  public ulong Handle
    => handle_;

  public CancellationToken Ending
    => ending_.Token;

  public void EndCall()
    => ending_.Cancel();

  internal static NativeCall<TResponse> Start(ulong channel,
                                              int deliveryCredits,
                                              string method,
                                              Metadata? metadata,
                                              Marshaller<TResponse> marshaller,
                                              DateTime? deadline,
                                              bool oneRequest,
                                              bool oneResponse)
  {
    // Encoded before the call exists: the encoding refuses a reserved key by throwing, and a call
    // built first would already hold the handle that roots it, with no terminal to free it.
    var methodBytes = MethodNames.GetValue(method,
                                           static name => Encoding.UTF8.GetBytes(name));
    var metadataBytes = RawMetadata.Encode(metadata);
    var (flags, timeoutNs) = TimeoutOf(deadline);
    if (oneRequest)
    {
      flags |= NativeMethods.AK_CALL_ONE_REQUEST;
    }

    if (oneResponse)
    {
      flags |= NativeMethods.AK_CALL_ONE_RESPONSE;
    }

    var call = new NativeCall<TResponse>(deliveryCredits,
                                         marshaller,
                                         oneRequest);

    // The engine copies both before it answers, so the pin lasts exactly the call.
    unsafe
    {
      fixed (byte* methodPinned = methodBytes)
      fixed (byte* metadataPinned = metadataBytes)
      {
        var options = new ak_call_start_options
                      {
                        struct_size = StartOptionsSize,
                        flags       = flags,
                        method = ak_bytes_in.Borrow(methodPinned,
                                                    methodBytes.Length),
                        metadata = ak_bytes_in.Borrow(metadataPinned,
                                                      metadataBytes.Length),
                        timeout_ns = timeoutNs,
                      };

        ak_status status;
        ak_error  error = default;
        fixed (ulong* started = &call.handle_)
        {
          status = NativeMethods.ak_call_start(channel,
                                               &options,
                                               (void*)GCHandle.ToIntPtr(call.self_),
                                               started,
                                               &error);
        }
        Probe.Mark(1);

        if (status != ak_status.AK_STATUS_OK)
        {
          call.self_.Free();
          var why = error.Take();

          // Three answers to three questions. A channel that has begun closing, or a handle
          // whose generation is spent, is the channel going away under a call that raced its
          // disposal - which is the same answer `NativeChannel.StartCall` gives when it sees the
          // disposal first. `InvalidArg` is what the caller handed over: a method that is not a
          // path, or metadata the engine reserves, refused before anything left this process, and
          // gRPC's own table maps a caller's error to INVALID_ARGUMENT. Anything else is this
          // binding's fault to own.
          throw new RpcException(new Status(status switch
                                            {
                                              ak_status.AK_STATUS_INVALID_STATE or ak_status.AK_STATUS_HANDLE_STALE => StatusCode.Unavailable,
                                              ak_status.AK_STATUS_INVALID_ARG => StatusCode.InvalidArgument,
                                              _ => StatusCode.Internal,
                                            },
                                            $"the call could not be started ({status}): {why}"));
        }
      }
    }

    call.ending_.Token.Register(call.EndNative);
    call.settling_ = call.SettlingAsync();

    return call;
  }

  /// <summary>
  ///   A call's deadline as the ABI takes it: the nanoseconds left of it, zero once it has
  ///   passed.
  /// </summary>
  /// <remarks>
  ///   Read as grpc-dotnet reads one: <see cref="DateTime.MaxValue" /> is none,
  ///   <see cref="DateTime.MinValue" /> one already passed, and any other has to be UTC, which is
  ///   refused rather than guessed at - a local time read as UTC is a deadline hours away from the
  ///   one the caller meant.
  /// </remarks>
  private static (uint Flags, ulong TimeoutNs) TimeoutOf(DateTime? deadline)
  {
    if (deadline is null || deadline.Value == DateTime.MaxValue)
    {
      return (0, 0);
    }

    var at = deadline.Value;

    if (at != DateTime.MinValue && at.Kind != DateTimeKind.Utc)
    {
      throw new InvalidOperationException("Deadline must have a kind DateTimeKind.Utc or be equal to DateTime.MaxValue or DateTime.MinValue.");
    }

    var left = at - DateTime.UtcNow;
    if (left <= TimeSpan.Zero)
    {
      return (NativeMethods.AK_CALL_HAS_DEADLINE, 0);
    }

    // A tick is a hundred nanoseconds; past what a ulong holds is a deadline no call reaches.
    return (NativeMethods.AK_CALL_HAS_DEADLINE, left.Ticks > (long)(ulong.MaxValue / 100)
                                                  ? ulong.MaxValue
                                                  : (ulong)left.Ticks * 100);
  }

  public bool Publish(ak_event_kind kind,
                      in ak_bytes payload,
                      int statusCode)
  {
    if (kind == ak_event_kind.AK_EVENT_WRITE_DONE)
    {
      sending_.Acquitted();
      return false;
    }

    if (kind == ak_event_kind.AK_EVENT_BUDGET_WAKE)
    {
      sending_.Woken();
      return false;
    }

    return receiving_.Store(kind,
                           payload,
                           statusCode);
  }

  public void Arrived()
    => receiving_.Arrived();

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
    => sending_.SendUnaryAsync(marshaller,
                               request);

  internal Task WriteAsync<TRequest>(Marshaller<TRequest> marshaller,
                                     TRequest request)
    => sending_.WriteAsync(marshaller,
                           request);

  internal void HalfClose()
    => sending_.HalfClose();

  internal TResponse Current
    => receiving_.Current;

  internal Task<bool> MoveNext(CancellationToken token)
    => receiving_.MoveNext(token);

  /// <summary>Completes when the call owes the engine nothing, which is what a channel's
  /// disposal waits for.</summary>
  internal Task Settled
    => settling_ ?? throw new InvalidOperationException("the call was not started");

  /// <returns>The response, or a faulted task carrying an <see cref="RpcException" />.</returns>
  /// <exception cref="InvalidOperationException">
  ///   The call is already being read as a single response. Thrown rather than returned in a
  ///   faulted task, because a second reduction is a fault of this assembly and not an outcome
  ///   of the RPC - a caller has nothing to handle, and one has something to fix.
  /// </exception>
  /// <remarks>
  ///   Unary, client streaming and any other cardinality that answers once are read by the
  ///   receiver's single-pass reader: one message, then a terminal, which the engine holds the
  ///   server to. A stream is read by <c>MoveNext</c>, on the same ring.
  ///   <para>
  ///     Whoever wants one message asks for it, because only the caller knows which shape it
  ///     asked the server for. Two readers on one ring would race, so exactly one call to this is
  ///     what a cardinality answering once owes; a stream owes none, and its holder reads the ring
  ///     itself.
  ///   </para>
  /// </remarks>
  internal Task<TResponse> SingleAsync()
  {
    // Refused rather than raced: a second reduction reads the same ring as the first, and what
    // the two would divide between them is one message and one terminal.
    //
    // Checked here and not in `Reduced` below, because an `async` method hands even a throw to
    // the task it returns - and a caller that discards the task would then observe nothing at
    // the line that was wrong.
    if (Interlocked.Exchange(ref reduced_,
                             1) != 0)
    {
      throw new InvalidOperationException("the call is already being read as a single response");
    }

    return Reduced();
  }

  private async Task<TResponse> Reduced()
  {
    TResponse response;
    try
    {
      // The reader rethrows a decode failure as it stands. This surface owes the binding's one
      // public rule instead: whatever went wrong, a caller of a single-response cardinality reads
      // it as an RpcException.
      try
      {
        response = await receiving_.SingleAsync()
                                   .ConfigureAwait(false);
      }
      catch (Exception thrown) when (thrown is not RpcException)
      {
        throw new RpcException(new Status(StatusCode.Internal,
                                          $"the call's events could not be read: {thrown.Message}"),
                               receiving_.Trailers);
      }
    }
    finally
    {
      // A call that still holds a buffer is not settled, whatever its terminal says: the
      // channel's drain waits on this, so completing early would let `ak_channel_release` and
      // then the runtime's shutdown run while the engine is still owed what the serializer has.
      await receiving_.Settled.ConfigureAwait(false);
    }

    // Reachable now, and read where the sender is known to have finished: it raises the count
    // and only then lets `holding_` fall, and `Settled` waits on that. Read from the reader the
    // count could be -1 - a WRITE_DONE that landed while the sender was off the CPU between its
    // P/Invoke returning and its own increment - and a call the server answered OK would fail.
    var unacquitted = sending_.Unacquitted;
    if (unacquitted != 0)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the call reached its terminal with {unacquitted} send(s) unacquitted"),
                             receiving_.Trailers);
    }

    Probe.Mark(10);
    return response;
  }

  private async Task SettlingAsync()
  {
    await receiving_.Settled.ConfigureAwait(false);

    // Before the wait below and not after. A send parked against the memory ceiling watches this
    // token, and what it waits for is room to serialize a message into a call that is over - room
    // other calls have to give up, on a schedule this one does not control. Cancelled afterwards,
    // the wait for that sender would be waiting on the sender it is there to release.
    ending_.Cancel();

    await sending_.HandedEverythingBackAsync()
                  .ConfigureAwait(false);

    // Cancelled before the claim, so an invoker that publishes after this reads the cancel and
    // disposes its own copy: in either order the registration is disposed exactly once.
    CancellationTokenRegistration registration;
    lock (disarm_)
    {
      registration  = cancellation_;
      cancellation_ = default;
    }

    registration.Dispose();

    // This terminates: a terminal cannot be consumed until the prologue has let go, and the
    // cancel above is what ends one still waiting on a call that published nothing.
    await receiving_.PrologueFinished.ConfigureAwait(false);
  }

  /// <summary>Ends this call when the caller's token is cancelled.</summary>
  /// <remarks>Registered on <c>ending_</c> rather than on the token directly, so it is set up
  /// before the call is started and cannot race the reader disposing it: a call the engine ends
  /// at once - a channel released underneath it - reaches its terminal and disposes
  /// <c>cancellation_</c> while the invoker is still on its way here, and the registration would
  /// then outlive the call for as long as the caller's token source does. Reading that state and
  /// publishing are one step under <c>disarm_</c>, so no terminal lands between them. A token
  /// already cancelled cancels immediately, which is what a caller passing one expects.</remarks>
  internal void CancelWith(CancellationToken token)
  {
    if (!token.CanBeCanceled)
    {
      return;
    }

    var registration = token.Register(Cancel);

    lock (disarm_)
    {
      if (!ending_.IsCancellationRequested)
      {
        cancellation_ = registration;
        return;
      }
    }

    registration.Dispose();
  }

  /// <summary>Ends the call, and makes sure what it already published is collected.</summary>
  /// <remarks>The drain is the whole point: a caller that disposes a response stream half read
  /// leaves events behind, and nothing else would take them - the channel's disposal waits for
  /// every call to have settled, so a cancel that only ended the call would hang it.</remarks>
  public void Cancel()
    => receiving_.CancelAndDrain();

  private unsafe void EndNative()
  {
    if (!TerminalAsync.IsCompleted)
    {
      NativeMethods.ak_call_cancel(handle_,
                                   null);
    }
  }
}
