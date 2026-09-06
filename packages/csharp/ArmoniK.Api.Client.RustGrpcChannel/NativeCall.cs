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

    call.drained_ = call.RunAsync();

    return call;
  }

  public bool Publish(NativeMethods.AkEventKind kind,
                      in NativeMethods.AkBytes payload,
                      int statusCode)
  {
    if (kind == NativeMethods.AkEventKind.WriteDone)
    {
      Interlocked.Decrement(ref inFlight_);
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
      ending_.Cancel();
      throw;
    }
  }

  private async Task SendingAsync<TRequest>(Marshaller<TRequest> marshaller,
                                            TRequest request)
  {
    Interlocked.Increment(ref holding_);
    try
    {
      await HoldingABufferAsync(marshaller,
                                request)
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
                                                   TRequest request)
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

    // The two the engine answers for a call that is already over, which the sender cannot rule
    // out and which the terminal reports anyway.
    var closed = NativeMethods.ak_call_end_send(handle_);
    if (closed is not (NativeMethods.AkStatus.Ok or NativeMethods.AkStatus.HandleStale
                                                 or NativeMethods.AkStatus.InvalidState))
    {
      throw Failed($"the half-close was refused ({closed})");
    }
  }

  private async Task<TResponse> RunAsync()
  {
    try
    {
      return await ReadAsync()
               .ConfigureAwait(false);
    }
    finally
    {
      // A call that still holds a buffer is not settled, whatever its terminal says. The channel's
      // drain waits on this task, so completing it early would let `ak_channel_release` and then
      // the runtime's shutdown run while the engine is still owed what the serializer has.
      while (Volatile.Read(ref holding_) != 0)
      {
        await handedBack_.WaitAsync()
                         .ConfigureAwait(false);
      }
    }

    // Here and not in the reader, because here the sender is known to have finished: it raises
    // the count and only then lets `holding_` fall, and the wait above is on that. Read from the
    // reader, the count could be -1 - a WRITE_DONE that landed while the sender was off the CPU
    // between its P/Invoke returning and its own increment - and a call the server answered OK
    // would fail with Internal.
    var unacquitted = Volatile.Read(ref inFlight_);
    if (unacquitted != 0)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the call reached its terminal with {unacquitted} send(s) unacquitted"),
                             trailers_);
    }
  }

  private async Task<TResponse> ReadAsync()
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
            response = marshaller_.ContextualDeserializer(new ReceivedMessage(slot.Payload));
            break;

          case NativeMethods.AkEventKind.Status:
            Settle(slot);
            break;
        }
      }
      catch (Exception thrown)
      {
        refused ??= thrown;

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

    ending_.Cancel();
    cancellation_.Dispose();

    if (refused is not null)
    {
      throw refused is RpcException rpc
              ? rpc
              : new RpcException(new Status(StatusCode.Internal,
                                            $"the call's events could not be read: {refused.Message}"),
                                 trailers_);
    }


    var status = await terminal_.Task.ConfigureAwait(false);
    if (status.StatusCode != StatusCode.OK)
    {
      throw new RpcException(status,
                             trailers_);
    }

    if (seen != 1)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"a unary call answered with {seen} messages"),
                             trailers_);
    }

    return response!;
  }

  private void Settle(in Slot slot)
  {
    RawMetadata.DecodeStatus(Bytes(slot.Payload),
                      out var reason,
                      out var trailers);
    trailers_ = trailers;

    var ended = new Status((StatusCode)slot.Status,
                           reason);
    terminal_.TrySetResult(ended);

    if (ended.StatusCode == StatusCode.OK)
    {
      headers_.TrySetResult(Metadata.Empty);
    }
    else
    {
      FailHead(new RpcException(ended,
                                trailers));
    }
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

  public void Cancel()
    => ending_.Cancel();

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
