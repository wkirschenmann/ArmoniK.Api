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
using System.Text;
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

  /// <summary>The calls this channel started that have not settled. Keyed by identity.</summary>
  private readonly ConcurrentDictionary<object, Task> live_ = new();

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

    var json = Encoding.UTF8.GetBytes($"{{\"endpoint\":{Quote(endpoint)},\"delivery_credits\":{deliveryCredits}}}");
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
  internal ChannelDisposeState DisposeState
    => state_;

  /// <inheritdoc />
  public override CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(this);

  internal ulong Handle
    => handle_;

  internal ulong Runtime
    => runtime_.Handle;

  internal int DeliveryCredits
    => deliveryCredits_;

  /// <summary>
  ///   Records a call as this channel's until it settles.
  /// </summary>
  /// <remarks>
  ///   The task is the call's drain, and its completion is what "settled" means: past the
  ///   terminal, every payload consumed and every buffer given back.
  /// </remarks>
  internal void Track(object call,
                      Task settled)
  {
    live_[call] = settled;
    // The discard is typed: TryRemove's out is unannotated in the netstandard2.0 reference
    // assembly, so an inferred one reads as a null assignment to a non-nullable Task.
    _ = settled.ContinueWith(_ => live_.TryRemove(call,
                                                  out Task? _),
                             TaskContinuationOptions.ExecuteSynchronously);
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
      foreach (var call in live_.Keys.ToArray())
      {
        (call as IDisposable)?.Dispose();
      }

      await Task.WhenAll(live_.Values.ToArray()
                              .Select(Settled))
                .ConfigureAwait(false);

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

  /// <summary>A call that ended badly still settled, which is all this waits for.</summary>
  private static Task Settled(Task drain)
    => drain.ContinueWith(static _ =>
                          {
                          },
                          TaskContinuationOptions.ExecuteSynchronously);

  private static string Quote(string value)
  {
    var quoted = new StringBuilder(value.Length + 2).Append('"');
    foreach (var character in value)
    {
      switch (character)
      {
        case '"':
          quoted.Append("\\\"");
          break;

        case '\\':
          quoted.Append("\\\\");
          break;

        default:
          if (character < ' ')
          {
            quoted.Append("\\u")
                  .Append(((int)character).ToString("x4"));
          }
          else
          {
            quoted.Append(character);
          }

          break;
      }
    }

    return quoted.Append('"')
                 .ToString();
  }
}
