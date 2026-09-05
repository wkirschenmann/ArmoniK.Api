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

[JsonSourceGenerationOptions(PropertyNamingPolicy = JsonKnownNamingPolicy.SnakeCaseLower)]
[JsonSerializable(typeof(ChannelOptions))]
internal partial class ChannelOptionsJsonContext : JsonSerializerContext
{
}

internal sealed class ChannelOptions
{
  public string Endpoint { get; set; } = string.Empty;

  public int DeliveryCredits { get; set; }

  internal byte[] Encode()
    => JsonSerializer.SerializeToUtf8Bytes(this,
                                           ChannelOptionsJsonContext.Default.ChannelOptions);
}
