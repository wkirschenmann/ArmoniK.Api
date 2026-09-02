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

/// <summary>
///   How far along the shared runtime's teardown is. One generation at a time.
/// </summary>
/// <remarks>
///   The names are the model's own (<c>runtime_dispose_state</c>), so a reader can hold the code
///   and <c>DotNetBinding.tla</c> side by side.
/// </remarks>
internal enum RuntimeDisposeState
{
  /// <summary>No generation is materialized. A fresh one may follow.</summary>
  Absent,
  Active,

  /// <summary>The last lease is gone and the latch is set: no further lease may be taken.</summary>
  ShutdownPending,
  Destroying,
  Destroyed,
}

/// <summary>
///   The shared runtime, by generation: the first channel materializes it, every later one takes a
///   lease, and the release that empties the set tears it down.
/// </summary>
/// <remarks>
///   A channel is the unit of borrowing, which is why the count is of channels and not of calls.
///   The release decides whether it was the last one under the same lock that latches the state to
///   <see cref="RuntimeDisposeState.ShutdownPending" />, so no lease can be taken between the zero
///   and the destroy and resurrect a generation on its way out.
///   <para>
///     The runtime's own options belong here rather than to a channel: there is one generation for
///     the process, so its worker threads and its byte ceiling are the process's. They may be set
///     only while no generation exists.
///   </para>
/// </remarks>
public static class NativeRuntimeFactory
{
  private static readonly object Gate = new();

  private static RuntimeDisposeState state_ = RuntimeDisposeState.Absent;
  private static NativeRuntime? current_;
  private static int leases_;
  private static TaskCompletionSource<bool>? destroyed_;
  private static uint workerThreads_;
  private static ulong memoryCeiling_;

  /// <summary>What the next generation is created with.</summary>
  /// <param name="workerThreads">Zero leaves the choice to the runtime.</param>
  /// <param name="memoryCeiling">Bytes lent buffers may occupy at once. Zero is no ceiling.</param>
  /// <exception cref="InvalidOperationException">A generation exists, so these would not apply to it.</exception>
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

  /// <summary>Opens a channel, materializing the shared runtime if this is the first one.</summary>
  /// <param name="endpoint">Where to dial, as a plain HTTP/2 URI.</param>
  /// <param name="deliveryCredits">
  ///   How many payloads of one call of this channel may be outstanding at once. The host is what
  ///   holds them, so the host is what chooses; each call sizes its queue from it.
  /// </param>
  public static NativeChannel Channel(string endpoint,
                                      int deliveryCredits = 1)
  {
    if (deliveryCredits < 1)
    {
      throw new ArgumentOutOfRangeException(nameof(deliveryCredits),
                                            "a window of zero admits no delivery at all");
    }

    var runtime = Lease();
    try
    {
      return new NativeChannel(runtime,
                               endpoint,
                               deliveryCredits);
    }
    catch
    {
      // A refused creation is local - a bad endpoint may not take down the generation every
      // other channel leases - so the lease goes back and the teardown follows only if it was
      // the last one, exactly as a release would.
      Release();
      throw;
    }
  }

  /// <summary>What ABI the loaded library speaks. Opening a channel refuses a mismatch.</summary>
  public static int LibraryAbiVersion
    => NativeRuntime.LibraryAbiVersion;

  /// <summary>The state the model calls <c>runtime_dispose_state</c>, for tests and assertions.</summary>
  public static string State
  {
    get
    {
      lock (Gate)
      {
        return state_.ToString();
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
            // The generation on its way out cannot take a lease, and the next one does not
            // exist yet. Waiting for the teardown is what re-arms the factory.
            waiting = destroyed_!.Task;
            break;
        }
      }

      waiting.GetAwaiter()
             .GetResult();
    }
  }

  /// <summary>
  ///   Gives a lease back. Answers what the caller must still wait for: the last release owes the
  ///   destroy of the generation it released, and nobody else owes anything.
  /// </summary>
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

      // The latch and the decision in one step, under the lock the lease is taken under.
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

    retiring.BeginShutdown();

    Exception? failure = null;
    try
    {
      await retiring.ReleasedAsync()
                    .ConfigureAwait(false);
      retiring.Destroy();
    }
    catch (Exception raised)
    {
      // Kept and handed to whoever awaits the destroy: a runtime that will not quiesce is not
      // something to discover from a hung DisposeAsync.
      failure = raised;
    }

    TaskCompletionSource<bool> waiting;
    lock (Gate)
    {
      state_ = RuntimeDisposeState.Destroyed;
      waiting = destroyed_!;

      if (failure is null)
      {
        // The root outlives every callback, and destroy returning is what says there are none.
        retiring.FreeRoot();
        current_ = null;
        state_   = RuntimeDisposeState.Absent;
        leases_  = 0;
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
