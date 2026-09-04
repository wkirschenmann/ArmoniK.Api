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
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   Wakes one waiter, and remembers a signal that arrives when nobody is waiting.
/// </summary>
/// <remarks>
///   <para>
///     The remembering is what makes "look, then wait" safe. The drain finds the ring empty and
///     only then calls <see cref="WaitAsync" />; a callback publishing in between would find no
///     waiter, and without the latch its signal would be dropped and the drain would park on an
///     event that had already arrived.
///   </para>
///   <para>
///     One signal, not a count: several events published while the drain is busy collapse into
///     one wake-up, and that is enough because the drain re-reads the ring and takes everything
///     there. The cost of collapsing is a wake-up that finds nothing, which is why the caller
///     loops rather than waiting once.
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

  /// <summary>
  ///   The one waiter, if there is one.
  /// </summary>
  /// <remarks>
  ///   One field and not a queue: a call has one drain and one only, and it can be inside one
  ///   await at a time, so a second waiter would be a bug rather than something to serve.
  /// </remarks>
  private TaskCompletionSource<bool>? waiter_;

  private bool signalled_;

  internal Task WaitAsync()
  {
    lock (gate_)
    {
      if (signalled_)
      {
        signalled_ = false;
        return Task.CompletedTask;
      }

      if (waiter_ is not null)
      {
        throw new InvalidOperationException("this signal has one waiter, and it is already waiting");
      }

      waiter_ = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
      return waiter_.Task;
    }
  }

  internal void Set()
  {
    TaskCompletionSource<bool>? released;
    lock (gate_)
    {
      released = waiter_;
      waiter_  = null;
      if (released is null)
      {
        signalled_ = true;
      }
    }

    // Outside the lock: the completion is asynchronous, but scheduling it is not, and there is
    // no reason for a publisher to hold the gate while it happens.
    released?.TrySetResult(true);
  }
}
