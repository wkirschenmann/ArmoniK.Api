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

#if DEBUG
using System;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>Points where a test holds a call's own task, to reach an interleaving the scheduler
/// reaches only by chance. Debug builds only.</summary>
internal static class TestHooks
{
  /// <summary>Awaited by a consumer of a call's ring each time it finds the ring empty, between
  /// taking its wait and awaiting it.</summary>
  /// <remarks>Awaited rather than run: the prologue's first pass is on the thread that starts the
  /// call, which a hook that blocked would stop before the call is handed out.</remarks>
  internal static Func<RingConsumer, Task>? FoundTheRingEmpty;
}

internal enum RingConsumer
{
  Prologue,
  Reader,
  Drain,
}
#endif
