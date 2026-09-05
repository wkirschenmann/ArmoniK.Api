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
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal enum ChannelDisposeState
{
  Active,
  Disposing,

  Released,

  ReleasedLast,
  Disposed,
}

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
                         int deliveryCredits)
    : base(endpoint)
  {
    runtime_         = runtime;
    deliveryCredits_ = deliveryCredits;

    var json = new ChannelOptions
               {
                 Endpoint        = endpoint,
                 DeliveryCredits = deliveryCredits,
               }.Encode();

    unsafe
    {
      fixed (byte* pinned = json)
      {
        var config = NativeMethods.AkBytesIn.Borrow(pinned,
                                                    json.Length);

        var status = NativeMethods.ak_channel_create(runtime.Handle,
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
    Track(call,
          call.Drained);
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

    if (Volatile.Read(ref disposing_) != 0)
    {
      call.Cancel();
    }
  }

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

  public void Dispose()
    => DisposeAsync()
      .AsTask()
      .GetAwaiter()
      .GetResult();

  protected override Task ShutdownAsyncCore()
    => DisposeAsync()
      .AsTask();
}
