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

internal sealed class ArrivalSignal
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

    arrived.TrySetResult(true);
  }
}
