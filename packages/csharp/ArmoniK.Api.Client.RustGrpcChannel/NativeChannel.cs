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
using System.Collections.Concurrent;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>How far along a channel's disposal is. The names are the model's own.</summary>
internal enum ChannelDisposeState
{
  Active,
  Disposing,

  /// <summary>The lease is back. This one was not the last, so it owes nothing further.</summary>
  Released,

  /// <summary>The lease is back and this one emptied the set, so it owes the generation's destroy.</summary>
  ReleasedLast,
  Disposed,
}

/// <summary>
///   A channel on the shared native runtime: calls, over one HTTP/2 session to one endpoint.
/// </summary>
/// <remarks>
///   A channel holds one lease on the runtime and is the unit of borrowing - a
///   <see cref="CallInvoker" /> over it is a stateless view. Disposing settles this channel's own
///   calls before releasing the native half, because a channel is not reclaimed while one of its
///   calls still owes a payload or a buffer; then the lease goes back, and the release that
///   empties the set is the one that awaits the runtime's destroy.
/// </remarks>
public sealed class NativeChannel : ChannelBase, IAsyncDisposable, IDisposable
{
  private readonly NativeRuntime runtime_;
  private readonly ulong handle_;
  private readonly int deliveryCredits_;

  /// <summary>
  ///   The calls this channel started that have not settled, keyed by identity.
  /// </summary>
  /// <remarks>
  ///   Keyed by what the dispose needs of them rather than by <c>object</c>: the cast that would
  ///   otherwise stand here could silently do nothing, where this cannot compile without it.
  /// </remarks>
  private readonly ConcurrentDictionary<ICallSink, Task> live_ = new();

  private readonly TaskCompletionSource<bool> disposed_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private int disposing_;
  private ChannelDisposeState state_ = ChannelDisposeState.Active;

  internal NativeChannel(NativeRuntime runtime,
                         string endpoint,
                         int deliveryCredits)
    : base(endpoint)
  {
    runtime_         = runtime;
    deliveryCredits_ = deliveryCredits;

    var json = Blob.ChannelConfig(endpoint,
                                  deliveryCredits);
    var pin = GCHandle.Alloc(json,
                             GCHandleType.Pinned);
    try
    {
      var config = new NativeMethods.AkBytesIn
                   {
                     Ptr = pin.AddrOfPinnedObject(),
                     Len = (UIntPtr)json.Length,
                   };

      var status = NativeMethods.ak_channel_create(runtime.Handle,
                                                   config,
                                                   out handle_);
      if (status != NativeMethods.AkStatus.Ok)
      {
        throw new InvalidOperationException($"`{endpoint}` was refused ({status})");
      }
    }
    finally
    {
      pin.Free();
    }
  }

  /// <summary>The state the model calls <c>channel_dispose_state</c>, for tests and assertions.</summary>
  public string DisposeState
    => state_.ToString();

  /// <summary>
  ///   What the engine says of its half, which is the model's <c>channel_state</c>.
  /// </summary>
  /// <remarks>
  ///   Read rather than remembered, so <c>ChannelStateMatchesNative</c> can be checked against
  ///   the two sides instead of asserted of one.
  /// </remarks>
  public string NativeState
    => NativeMethods.ak_channel_status(handle_)
                    .ToString();

  /// <inheritdoc />
  public override CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(this);

  /// <summary>
  ///   Starts a call on this channel.
  /// </summary>
  /// <remarks>
  ///   Here and not at the invoker: the handle, the delivery window and the runtime that owns the
  ///   ceiling are all the channel's, and an invoker that forwarded them would be a courier for a
  ///   sentence it has no part in.
  /// </remarks>
  internal NativeCall<TResponse> StartCall<TResponse>(string method,
                                                      Metadata? metadata,
                                                      Marshaller<TResponse> marshaller)
    where TResponse : class
  {
    // The model's guard on starting a call is an active channel, and it is also what bounds the
    // wait in `DisposeAsync`: without it a caller could keep starting calls into a channel that
    // is trying to settle them all.
    if (Volatile.Read(ref disposing_) != 0)
    {
      throw new RpcException(new Status(StatusCode.Unavailable,
                                        "the channel is being disposed and takes no new calls"));
    }

    var call = NativeCall<TResponse>.Start(runtime_,
                                           handle_,
                                           deliveryCredits_,
                                           method,
                                           metadata,
                                           marshaller);
    Track(call,
          call.Drained);
    return call;
  }

  /// <summary>
  ///   Records a call as this channel's until it settles.
  /// </summary>
  /// <remarks>
  ///   The task is the call's drain, and its completion is what "settled" means: past the
  ///   terminal, every payload consumed and every buffer given back.
  ///   <para>
  ///     Private, and called by <see cref="StartCall" /> before the call is handed to anyone.
  ///     A caller that registered the call as a later step left a window in which this channel
  ///     owned a live call it did not know about, so a concurrent dispose released the native
  ///     half without awaiting that call's drain - which is the one thing the dispose promises
  ///     not to do.
  ///   </para>
  /// </remarks>
  private void Track(ICallSink call,
                     Task settled)
  {
    live_[call] = settled;
    // The state is passed rather than captured, so the continuation costs no closure; and the
    // discard is typed, TryRemove's out being unannotated in the netstandard2.0 reference
    // assembly, where an inferred one reads as a null assignment to a non-nullable Task.
    _ = settled.ContinueWith(static (_,
                                     state) =>
                             {
                               var (tracked, key) = ((ConcurrentDictionary<ICallSink, Task>, ICallSink))state!;
                               tracked.TryRemove(key,
                                                 out Task? _);
                             },
                             (live_, call),
                             TaskContinuationOptions.ExecuteSynchronously);

    // Published, and only now asked whether the channel is still taking calls. The check before
    // the insert cannot stand alone: a dispose that latches in between snapshots `live_` without
    // this call in it, so nothing would settle it. Re-reading afterwards closes that from the
    // other side - either the dispose's snapshot holds this call, or its latch preceded this read
    // and this disposes the call itself.
    if (Volatile.Read(ref disposing_) != 0)
    {
      call.Cancel();
    }
  }

  /// <inheritdoc />
  public async ValueTask DisposeAsync()
  {
    if (Interlocked.Exchange(ref disposing_,
                             1) != 0)
    {
      await disposed_.Task.ConfigureAwait(false);
      return;
    }

    state_ = ChannelDisposeState.Disposing;

    try
    {
      // Every call of this channel and no other: settled before the native half closes, since a
      // call that still owes something keeps the channel from being reclaimed.
      // Re-read rather than snapshotted once. `Track` publishes a call and only then checks
      // the latch, so a call that arrived after a snapshot cancels itself but would not be
      // awaited here - and what this method promises, and what the native release below rests
      // on, is that it returns with every call of this channel settled. The guard in
      // `StartCall` is what makes this terminate: no call joins after the latch.
      while (!live_.IsEmpty)
      {
        // One snapshot per round for both loops, where two could disagree.
        var live = live_.ToArray();
        // Asked to end, not disposed: the engine cancels this channel's calls itself when the
        // native half is released - the header is emphatic about it, a call parked on a delivery
        // credit being the case that needs it - so what is left for this side is waking the
        // managed waiters of each.
        foreach (var call in live)
        {
          call.Key.Cancel();
        }

        try
        {
          await Task.WhenAll(live.Select(settling => settling.Value))
                    .ConfigureAwait(false);
        }
        catch
        {
          // A call that ended badly still settled, which is all this waits for.
        }
      }

      NativeMethods.ak_channel_release(handle_);

      var (wasLast, destroyed) = NativeRuntimeFactory.Release();
      state_ = wasLast
                 ? ChannelDisposeState.ReleasedLast
                 : ChannelDisposeState.Released;

      // What this channel's task promised: the one that emptied the set completes only once the
      // generation IT released is gone.
      if (wasLast)
      {
        await destroyed.ConfigureAwait(false);
      }

      state_ = ChannelDisposeState.Disposed;
      disposed_.TrySetResult(true);
    }
    catch (Exception raised)
    {
      disposed_.TrySetException(raised);
      throw;
    }
  }

  /// <summary>
  ///   Blocks on <see cref="DisposeAsync" />.
  /// </summary>
  /// <remarks>
  ///   <see cref="ChannelBase" /> and <see cref="IDisposable" /> both want a synchronous close,
  ///   and the last channel's has a native destroy behind it. Prefer
  ///   <see cref="DisposeAsync" /> wherever the caller can await.
  /// </remarks>
  public void Dispose()
    => DisposeAsync()
      .AsTask()
      .GetAwaiter()
      .GetResult();

  /// <inheritdoc />
  protected override Task ShutdownAsyncCore()
    => DisposeAsync()
      .AsTask();
}
