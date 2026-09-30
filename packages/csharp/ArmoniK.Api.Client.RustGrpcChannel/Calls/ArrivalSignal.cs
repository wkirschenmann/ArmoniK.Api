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

using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>Wakes every wait taken before it, and keeps nothing for a wait taken after.</summary>
/// <remarks>
///   A waiter takes <see cref="Next" /> before it looks at what it waits for, and awaits it only
///   when the look found nothing: a change after the look sets the wait already taken, and one
///   before it the look saw. With nothing kept, no waiter can take a wake-up another needed.
///   <see cref="Set" /> never blocks and takes no lock: the engine's callback thread is among the
///   threads that run it, and it must not wait on a host thread.
/// </remarks>
internal sealed class ArrivalSignal
{
  private TaskCompletionSource<bool> arrived_ = Pending();

  // 1 once a wait may have been taken on `arrived_`, so a set with none taken since the last one
  // allocates nothing. A waiter that read the task just before a swap raises it against the old
  // one, which costs the next set an allocation for nobody.
  private int waited_;

  private static TaskCompletionSource<bool> Pending()
    => new(TaskCreationOptions.RunContinuationsAsynchronously);

  internal Task Next()
  {
    // The task, then the flag, the flag with a full fence. A set that swaps the task after it
    // was read completes it; one that finds no flag came before the flag went up, so the look
    // that follows this sees whatever that set was for.
    var arrived = Volatile.Read(ref arrived_);
    Interlocked.Exchange(ref waited_,
                         1);
    return arrived.Task;
  }

  internal void Set()
  {
    if (Interlocked.Exchange(ref waited_,
                             0) == 0)
    {
      return;
    }

    Interlocked.Exchange(ref arrived_,
                         Pending())
               .TrySetResult(true);
  }
}
