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

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>Raised where the runtime-wide ceiling has no room for a lend yet.</summary>
///
/// The ceiling is backpressure and not a refusal, so the answer is to wait. It travels as an
/// exception because the method that meets it is a `SerializationContext` override and cannot
/// await: the send loop catches this, waits for room, and serializes again. Never leaves the
/// binding.
internal sealed class NoRoomYet : Exception
{
  internal NoRoomYet()
    : base("the send ceiling has no room for this message yet")
  {
  }
}
