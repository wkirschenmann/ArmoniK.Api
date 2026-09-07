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

// Rooted at ChannelOptions, which reaches every group of the vocabulary, so the whole graph is
// serialized without reflection - which is what lets a trimmed or native-AOT host use this.
[JsonSerializable(typeof(ChannelOptions))]
internal partial class ChannelOptionsJsonContext : JsonSerializerContext
{
}

/// <summary>What the schema does not say: how the document crosses the ABI.</summary>
/// <remarks>
///   The properties, their bounds and their documentation are generated from
///   <c>options.schema.json</c> into ChannelOptions.g.cs. Only what a schema cannot describe is
///   written here.
/// </remarks>
internal sealed partial class ChannelOptions
{
  /// <summary>The document the engine reads, as UTF-8.</summary>
  /// <returns>The options as JSON, without the ones left unset.</returns>
  /// <exception cref="System.ArgumentOutOfRangeException">An option is outside its bounds.</exception>
  /// <remarks>
  ///   Checked before it is written, not after it is refused: the engine answers a bad document
  ///   with a status on `ak_channel_create`, which names neither the option nor the bound.
  /// </remarks>
  internal byte[] Encode()
  {
    Validate();

    return JsonSerializer.SerializeToUtf8Bytes(this,
                                               ChannelOptionsJsonContext.Default.ChannelOptions);
  }
}
