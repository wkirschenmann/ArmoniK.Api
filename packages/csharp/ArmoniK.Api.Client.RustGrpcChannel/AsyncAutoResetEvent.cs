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

using System.Collections.Generic;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   A latched signal whose <see cref="Set" /> never runs a waiter inline.
/// </summary>
/// <remarks>
///   That is the point of it, not an implementation detail: <see cref="Set" /> is called from the
///   library's own thread, and a continuation running there would hold the native actor up behind
///   whatever the application does next.
/// </remarks>
internal sealed class AsyncAutoResetEvent
{
  private readonly Queue<TaskCompletionSource<bool>> waiting_ = new();
  private bool signalled_;

  internal Task WaitAsync()
  {
    lock (waiting_)
    {
      if (signalled_)
      {
        signalled_ = false;
        return Task.CompletedTask;
      }

      var waiter = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
      waiting_.Enqueue(waiter);
      return waiter.Task;
    }
  }

  internal void Set()
  {
    TaskCompletionSource<bool>? released = null;
    lock (waiting_)
    {
      if (waiting_.Count > 0)
      {
        released = waiting_.Dequeue();
      }
      else
      {
        signalled_ = true;
      }
    }

    released?.TrySetResult(true);
  }
}
