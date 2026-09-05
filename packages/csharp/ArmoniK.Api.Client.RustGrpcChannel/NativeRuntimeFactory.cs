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

public static class NativeRuntimeFactory
{
  private static readonly object Gate = new();

  private static RuntimeDisposeState state_ = RuntimeDisposeState.Absent;
  private static NativeRuntime? current_;
  private static int leases_;
  private static TaskCompletionSource<bool>? destroyed_;
  private static uint workerThreads_;
  private static ulong memoryCeiling_;

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

          default:
            waiting = destroyed_!.Task;
            break;
        }
      }

      waiting.GetAwaiter()
             .GetResult();
    }
  }

  internal static (bool WasLast, Task Destroyed) Release()
  {
    NativeRuntime retiring;
    Task destroyed;

    lock (Gate)
    {
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
        state_ = RuntimeDisposeState.DestroyFailed;
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
