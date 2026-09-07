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

using ArmoniK.Api.Common.Utils;

using Microsoft.Extensions.Configuration;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal enum RuntimeDisposeState
{
  Absent,
  Active,

  ShutdownPending,
  Destroying,

  DestroyFailed,
}

/// <summary>The one native runtime a process may hold, and the channels leased from it.</summary>
public static class NativeRuntimeFactory
{
  private static readonly object Gate = new();

  private static RuntimeDisposeState state_ = RuntimeDisposeState.Absent;
  private static NativeRuntime? current_;
  private static int leases_;
  private static TaskCompletionSource<bool>? destroyed_;
  private static Exception? refused_;
  private static uint workerThreads_;
  private static ulong memoryCeiling_;

  /// <summary>Sets what the next runtime is created with. Refused while one exists.</summary>
  public static void Configure(uint workerThreads = 0,
                               ulong memoryCeiling = 0)
  {
    lock (Gate)
    {
      if (state_ != RuntimeDisposeState.Absent)
      {
        throw new InvalidOperationException($"the runtime is {state_}; its options are fixed while it exists");
      }

      workerThreads_ = workerThreads;
      memoryCeiling_ = memoryCeiling;
    }
  }

  /// <summary>The deepest delivery window a channel may ask for.</summary>
  /// <remarks>Every call of the channel allocates a ring of the next power of two above it, so a
  /// window is paid per call in memory whether or not the peer ever fills it: this one is a
  /// megabyte of slots. There is no answer here for what a host should want - the bound exists
  /// because the ABI's own is `Semaphore::MAX_PERMITS`, which is 2^61 and sizes nothing.</remarks>
  public const int MaxDeliveryCredits = 1 << 15;

  /// <summary>The delivery window a channel gets when its options name none.</summary>
  /// <remarks>
  ///   Resolved into the document a channel sends, so the engine is never left to apply its own -
  ///   which is what keeps the ring this side sizes and the credits that side grants the same
  ///   number. One, because a host that asks for nothing gets a channel it can drive without ever
  ///   holding two of anything.
  /// </remarks>
  public const int DefaultDeliveryCredits = 1;

  /// <summary>The section a channel's options are read from when a caller names none.</summary>
  public const string SettingSection = "RustGrpcChannel";

  /// <summary>Opens a channel with the options a configuration carries.</summary>
  /// <param name="endpoint">Where the channel connects, as the engine's own argument.</param>
  /// <param name="configuration">What the options are read from.</param>
  /// <param name="key">The section holding them.</param>
  /// <returns>The channel, holding a lease on the runtime.</returns>
  /// <exception cref="ArgumentNullException"><paramref name="configuration" /> is null.</exception>
  /// <exception cref="InvalidOperationException"><paramref name="key" /> names no section.</exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside what is admitted.</exception>
  /// <remarks>
  ///   Required rather than optional: a caller who names a section meant to configure this, and a
  ///   misspelled name that quietly gave the engine's defaults would be a channel nobody
  ///   configured. <see cref="Channel(string,int)" /> is how to ask for the defaults.
  /// </remarks>
  public static NativeChannel Channel(string endpoint,
                                      IConfiguration configuration,
                                      string key = SettingSection)
  {
    if (configuration is null)
    {
      throw new ArgumentNullException(nameof(configuration));
    }

    return Channel(endpoint,
                   configuration.GetRequiredValue<ChannelOptions>(key));
  }

  /// <summary>Opens a channel with a delivery window, and the engine's defaults elsewhere.</summary>
  /// <param name="endpoint">Where the channel connects.</param>
  /// <param name="deliveryCredits">How many events the engine may hold for an unread call.</param>
  /// <returns>The channel, holding a lease on the runtime.</returns>
  /// <exception cref="ArgumentOutOfRangeException">The window is outside what is admitted.</exception>
  public static NativeChannel Channel(string endpoint,
                                      int deliveryCredits = DefaultDeliveryCredits)
    => Channel(endpoint,
               new ChannelOptions
               {
                 DeliveryCredits = deliveryCredits,
               });

  /// <summary>Opens a channel and takes a lease on the runtime, creating it if there is none.
  /// Disposing the channel gives the lease back.</summary>
  /// <param name="endpoint">Where the channel connects.</param>
  /// <param name="options">What the channel is opened with, read once and never written to.</param>
  /// <returns>The channel, holding a lease on the runtime.</returns>
  /// <exception cref="ArgumentNullException"><paramref name="options" /> is null.</exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside what is admitted.</exception>
  public static NativeChannel Channel(string endpoint,
                                      ChannelOptions options)
  {
    if (options is null)
    {
      throw new ArgumentNullException(nameof(options));
    }

    // One read of the caller's instance: what a channel sizes its rings from and what it sends
    // the engine are the same number only if nothing can set it in between.
    var settled = new ChannelOptions(options)
                  {
                    DeliveryCredits = options.DeliveryCredits ?? DefaultDeliveryCredits,
                  };

    // The schema's bounds, then this binding's own tighter one. Both are checked here rather
    // than left to the engine, which answers a bad document with a status naming no option.
    settled.Validate();
    RefuseAWindowNoRingCanHold(settled.DeliveryCredits);

    NativeRuntime runtime;
    try
    {
      runtime = Lease();
    }
    catch (DllNotFoundException absent)
    {
      throw RustEngineMissingException.For(absent);
    }

    try
    {
      return new NativeChannel(runtime,
                               endpoint,
                               settled);
    }
    catch
    {
      Release();
      throw;
    }
  }

  // Checked wherever a window arrives, so which door a caller came through does not decide
  // which bound applies.
  private static void RefuseAWindowNoRingCanHold(int? deliveryCredits)
  {
    if (deliveryCredits > MaxDeliveryCredits)
    {
      throw new ArgumentOutOfRangeException(nameof(ChannelOptions.DeliveryCredits),
                                            deliveryCredits,
                                            $"a delivery window is at most {MaxDeliveryCredits} here: every call of the channel allocates a ring of the next power of two above it");
    }
  }

  internal static int LibraryAbiVersion
  {
    get
    {
      try
      {
        return NativeMethods.ak_abi_version();
      }
      catch (DllNotFoundException absent)
      {
        throw RustEngineMissingException.For(absent);
      }
    }
  }

  internal static RuntimeDisposeState State
  {
    get
    {
      lock (Gate)
      {
        return state_;
      }
    }
  }

  private static NativeRuntime Lease()
  {
    while (true)
    {
      Task waiting;
      lock (Gate)
      {
        switch (state_)
        {
          case RuntimeDisposeState.Absent:
            current_   = NativeRuntime.Create(workerThreads_,
                                              memoryCeiling_);
            destroyed_ = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
            state_     = RuntimeDisposeState.Active;
            leases_    = 1;
            return current_;

          case RuntimeDisposeState.Active:
            leases_++;
            return current_!;

          case RuntimeDisposeState.DestroyFailed:
            // Absorbing, and honestly so: the engine admits one runtime per process and gives
            // that claim back only on a destroy that succeeded, so no other can be created here.
            // Said afresh rather than by re-throwing the task's exception, which would reach a
            // caller minutes later as though its own call had just timed out.
            throw new InvalidOperationException("the native runtime did not shut down, and the engine admits one runtime per process, so this one cannot be replaced",
                                                refused_);

          default:
            waiting = destroyed_!.Task;
            break;
        }
      }

      // Waited on for the state to move, not for what it moved to: the outcome is read above on
      // the next turn, where every caller reads the same thing whether it waited or not.
      try
      {
        waiting.GetAwaiter()
               .GetResult();
      }
      catch
      {
      }
    }
  }

  internal static (bool WasLast, Task Destroyed) Release()
  {
    NativeRuntime retiring;
    Task destroyed;

    lock (Gate)
    {
      // A lease is given back once. Letting the count go negative would send a second release
      // down the teardown path with `current_` already null, and the state machine would be
      // wrong from then on rather than at the call that broke it.
      if (leases_ <= 0)
      {
        throw new InvalidOperationException($"the runtime is {state_} and has no lease to give back");
      }

      if (--leases_ > 0)
      {
        return (false, Task.CompletedTask);
      }

      state_    = RuntimeDisposeState.ShutdownPending;
      retiring  = current_!;
      destroyed = destroyed_!.Task;
    }

    _ = TearDownAsync(retiring);
    return (true, destroyed);
  }

  private static async Task TearDownAsync(NativeRuntime retiring)
  {
    lock (Gate)
    {
      state_ = RuntimeDisposeState.Destroying;
    }

    Exception? failure = null;
    try
    {
      await retiring.RetireAsync()
                    .ConfigureAwait(false);
    }
    catch (Exception raised)
    {
      failure = raised;
    }

    TaskCompletionSource<bool> waiting;
    lock (Gate)
    {
      waiting = destroyed_!;

      if (failure is null)
      {
        current_ = null;
        state_   = RuntimeDisposeState.Absent;
      }
      else
      {
        state_   = RuntimeDisposeState.DestroyFailed;
        refused_ = failure;
      }
    }

    if (failure is null)
    {
      waiting.TrySetResult(true);
    }
    else
    {
      waiting.TrySetException(failure);
    }
  }
}
