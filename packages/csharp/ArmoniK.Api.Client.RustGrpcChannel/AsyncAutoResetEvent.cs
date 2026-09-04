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

using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   Wakes one waiter, and remembers a signal that arrives when nobody is waiting.
/// </summary>
/// <remarks>
///   <para>
///     The remembering is what makes "look at the ring, then wait" safe. The drain finds the ring
///     empty and only then calls <see cref="WaitAsync" />; a callback publishing in between would
///     find nobody to wake, and a signal dropped there would park the drain on an event that had
///     already arrived.
///   </para>
///   <para>
///     One task carries both halves of that: <see cref="Set" /> completes it, and completed is
///     what "a signal is pending" means, whether or not anyone was waiting when it happened.
///     <see cref="WaitAsync" /> consumes a pending signal by putting a fresh task in its place -
///     that exchange is the reset. So the state is one object rather than a waiter and a flag
///     that have to agree.
///   </para>
///   <para>
///     One signal, not a count: several events published while the drain is busy collapse into
///     one completion, and that is enough because the drain re-reads the ring and takes
///     everything there. Collapsing costs a wake-up that finds nothing, and consuming a
///     completion costs one turn of the caller's loop - which is why the caller loops rather
///     than waiting once.
///   </para>
///   <para>
///     Continuations are asynchronous because <see cref="Set" /> runs inside the FFI callback, on
///     the engine's own thread. Completing a waiter inline would run the drain's continuation
///     there - parsing a message, handing it to the application - so the engine's callback could
///     not return until the application was done with it.
///   </para>
/// </remarks>
internal sealed class AsyncAutoResetEvent
{
  private readonly object gate_ = new();

  private TaskCompletionSource<bool> arrived_ = Pending();

  private static TaskCompletionSource<bool> Pending()
    => new(TaskCreationOptions.RunContinuationsAsynchronously);

  internal Task WaitAsync()
  {
    lock (gate_)
    {
      var arrived = arrived_;
      if (arrived.Task.IsCompleted)
      {
        // Taken, so the next wait blocks again. Returning the completed task rather than
        // `Task.CompletedTask` keeps the caller on the one it was handed.
        arrived_ = Pending();
      }

      return arrived.Task;
    }
  }

  internal void Set()
  {
    TaskCompletionSource<bool> arrived;
    lock (gate_)
    {
      arrived = arrived_;
    }

    // Outside the lock: scheduling the continuation does not need it, and a publisher holding
    // the gate through it would block the next wait for nothing. A second `Set` on an already
    // completed task answers false, which is the collapsing above.
    arrived.TrySetResult(true);
  }
}
