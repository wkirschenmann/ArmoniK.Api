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
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;
using ArmoniK.Api.Client.RustGrpcChannel.Calls;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal enum ChannelDisposeState
{
  Active,
  Disposing,

  Released,

  ReleasedLast,
  Disposed,
}

/// <summary>A gRPC channel served by the native engine.</summary>
public sealed class NativeChannel : ChannelBase, IAsyncDisposable, IDisposable
{
  private readonly NativeRuntime runtime_;
  private readonly ulong handle_;
  private readonly int deliveryCredits_;

  private readonly ConcurrentDictionary<ICallSink, Task> live_ = new();

  private readonly TaskCompletionSource<bool> disposed_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private int disposing_;
  private volatile ChannelDisposeState state_ = ChannelDisposeState.Active;

  internal NativeChannel(NativeRuntime runtime,
                         string endpoint,
                         ChannelOptions options)
    : base(endpoint)
  {
    runtime_ = runtime;
    // Resolved by the factory, so this and the engine size from one number.
    deliveryCredits_ = options.DeliveryCredits!.Value;

    // The endpoint is its own argument and never an option: it is the one value a channel cannot
    // be created without, so every option of the document has a default and `{}` would do.
    var named = Encoding.UTF8.GetBytes(endpoint);
    var json = options.Encode();

    unsafe
    {
      fixed (byte* pinnedEndpoint = named)
      fixed (byte* pinned = json)
      {
        var where = NativeMethods.AkBytesIn.Borrow(pinnedEndpoint,
                                                   named.Length);
        var config = NativeMethods.AkBytesIn.Borrow(pinned,
                                                    json.Length);

        var status = NativeMethods.ak_channel_create(runtime.Handle,
                                                     where,
                                                     config,
                                                     out handle_);
        if (status != NativeMethods.AkStatus.Ok)
        {
          throw new InvalidOperationException($"`{endpoint}` was refused ({status})");
        }
      }
    }
  }

  internal ChannelDisposeState DisposeState
    => state_;

  internal NativeMethods.AkChannelState NativeState
    => NativeMethods.ak_channel_status(handle_);

  /// <summary>The invoker a generated client calls through.</summary>
  public override CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(this);

  internal NativeCall<TResponse> StartCall<TResponse>(string method,
                                                      Metadata? metadata,
                                                      Marshaller<TResponse> marshaller)
    where TResponse : class
  {
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
    // The settlement and not the response: a server stream has no single response, and what the
    // drain has to wait for is the terminal consumed with nothing owed either way.
    Track(call,
          call.Settled);
    return call;
  }

  private void Track(ICallSink call,
                     Task settled)
  {
    live_[call] = settled;
    _ = settled.ContinueWith(static (_,
                                     state) =>
                             {
                               var (tracked, key) = ((ConcurrentDictionary<ICallSink, Task>, ICallSink))state!;
                               tracked.TryRemove(key,
                                                 out Task? _);
                             },
                             (live_, call),
                             TaskContinuationOptions.ExecuteSynchronously);

    // Read again, because `StartCall`'s read and this line straddle the start: a disposal that
    // began between them drained a `live_` this call was not in yet.
    if (Volatile.Read(ref disposing_) != 0)
    {
      call.Cancel();
    }
  }

  /// <summary>Cancels what is still running, gives the lease back, and waits for the engine to
  /// be destroyed if this was the last channel.</summary>
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
      while (!live_.IsEmpty)
      {
        // One snapshot, because `Keys` and `Values` are two of them and a call may leave between.
        var live = live_.ToArray();
        var settling = new Task[live.Length];
        for (var index = 0; index < live.Length; index++)
        {
          live[index]
            .Key.Cancel();
          settling[index] = live[index]
            .Value;
        }

        try
        {
          await Task.WhenAll(settling)
                    .ConfigureAwait(false);
        }
        catch
        {
        }

        // Removed here rather than left to the continuation `Track` registered. A task runs its
        // continuations in order, on the thread that completed it; when `Track` was preempted
        // between the add and the ContinueWith, the removal is registered behind this loop's own
        // await and cannot run while this loop is on that thread - and the loop would spin on a
        // `live_` nothing can empty. Exact because a key is added once and its task never
        // changes: what was just awaited is what is being removed.
        foreach (var entry in live)
        {
          live_.TryRemove(entry.Key,
                          out Task? _);
        }
      }

      NativeMethods.ak_channel_release(handle_);

      var (wasLast, destroyed) = NativeRuntimeFactory.Release();
      state_ = wasLast
                 ? ChannelDisposeState.ReleasedLast
                 : ChannelDisposeState.Released;

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

  /// <summary><see cref="DisposeAsync" />, awaited. It blocks until the engine has let go, so
  /// prefer the asynchronous one wherever there is a choice.</summary>
  public void Dispose()
    => DisposeAsync()
      .AsTask()
      .GetAwaiter()
      .GetResult();

  /// <summary><see cref="ChannelBase" />'s shutdown, which is this channel's disposal.</summary>
  protected override Task ShutdownAsyncCore()
    => DisposeAsync()
      .AsTask();
}
