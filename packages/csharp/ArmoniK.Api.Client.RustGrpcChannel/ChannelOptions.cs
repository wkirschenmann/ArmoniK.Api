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

using System.Text.Json;
using System.Text.Json.Serialization;

namespace ArmoniK.Api.Client.RustGrpcChannel;

// The names are the schema's, which are the Rust field names in PascalCase, so the default
// policy is the right one and there is nothing to configure. An option written any other way is
// refused by the engine rather than ignored.
[JsonSerializable(typeof(ChannelOptions))]
internal partial class ChannelOptionsJsonContext : JsonSerializerContext
{
}

/// <summary>What a caller sets on one channel, as the engine's schema describes it.</summary>
/// <remarks>Hand-written until T3.3 generates it from that schema. The endpoint is not here: it
/// crosses the ABI as its own argument, which is what lets every option have a default.</remarks>
internal sealed class ChannelOptions
{
  public int DeliveryCredits { get; set; }

  internal byte[] Encode()
    => JsonSerializer.SerializeToUtf8Bytes(this,
                                           ChannelOptionsJsonContext.Default.ChannelOptions);
}
