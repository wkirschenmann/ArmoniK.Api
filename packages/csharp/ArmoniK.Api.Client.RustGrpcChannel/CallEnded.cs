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

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>Raised where the engine refuses a send because the call is already over.</summary>
///
/// It is not the answer, only the signal that the answer is elsewhere: the terminal says why the
/// call ended, and the reader is what carries it. Never leaves the binding.
internal sealed class CallEnded : Exception
{
  internal CallEnded(NativeMethods.AkStatus status)
    : base($"the call was already over ({status})")
  {
  }
}
