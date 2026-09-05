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

  /// <summary>Opens a channel and takes a lease on the runtime, creating it if there is none.
  /// Disposing the channel gives the lease back.</summary>
  public static NativeChannel Channel(string endpoint,
                                      int deliveryCredits = 1)
  {
    if (deliveryCredits < 1)
    {
      throw new ArgumentOutOfRangeException(nameof(deliveryCredits),
                                            "a window of zero admits no delivery at all");
    }

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
                               deliveryCredits);
    }
    catch
    {
      Release();
      throw;
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
