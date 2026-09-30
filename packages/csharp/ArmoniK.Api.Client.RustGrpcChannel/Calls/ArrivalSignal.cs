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

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>Wakes every wait taken before it, and keeps nothing for a wait taken after.</summary>
/// <remarks>
///   A waiter takes <see cref="Next" /> before it looks at what it waits for, and awaits it only
///   when the look found nothing: a change after the look sets the wait already taken, and one
///   before it the look saw. With nothing kept, no waiter can take a wake-up another needed.
/// </remarks>
internal sealed class ArrivalSignal
{
  private readonly object gate_ = new();

  private TaskCompletionSource<bool> arrived_ = Pending();

  // Whether a wait was taken on `arrived_`, so a set with no wait taken since the last one
  // allocates nothing.
  private bool waited_;

  private static TaskCompletionSource<bool> Pending()
    => new(TaskCreationOptions.RunContinuationsAsynchronously);

  internal Task Next()
  {
    lock (gate_)
    {
      waited_ = true;
      return arrived_.Task;
    }
  }

  internal void Set()
  {
    TaskCompletionSource<bool> arrived;
    lock (gate_)
    {
      if (!waited_)
      {
        return;
      }

      waited_  = false;
      arrived  = arrived_;
      arrived_ = Pending();
    }

    arrived.TrySetResult(true);
  }
}
