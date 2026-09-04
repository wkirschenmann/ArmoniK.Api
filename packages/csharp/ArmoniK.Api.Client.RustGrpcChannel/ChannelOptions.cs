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

using System.Globalization;
using System.Text;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   What <c>ak_channel_create</c> is given, as the JSON the engine parses.
/// </summary>
/// <remarks>
///   <para>
///     The engine refuses an option it does not recognise rather than ignoring it, so a name
///     misspelled here fails at channel creation instead of being silently dropped. That is the
///     reason to have a type at all: one place per option, rather than a name that appears once
///     in a concatenation.
///   </para>
///   <para>
///     Hand-written, and only for as long as it is two options. The engine's options are
///     declared in Rust and their JSON schema comes from there, so this class is what a generator
///     replaces - which is why it is a plain carrier with a writer and no behaviour of its own.
///   </para>
/// </remarks>
internal sealed class ChannelOptions
{
  internal string Endpoint { get; set; } = string.Empty;

  internal int DeliveryCredits { get; set; }

  /// <summary>The options as UTF-8 JSON, which is what the ABI takes.</summary>
  internal byte[] Encode()
  {
    var json = new StringBuilder("{");
    Member(json,
           "endpoint",
           Quoted(Endpoint));
    json.Append(',');
    Member(json,
           "delivery_credits",
           DeliveryCredits.ToString(CultureInfo.InvariantCulture));
    return Encoding.UTF8.GetBytes(json.Append('}')
                                      .ToString());
  }

  private static void Member(StringBuilder json,
                             string name,
                             string value)
    => json.Append(Quoted(name))
           .Append(':')
           .Append(value);

  /// <summary>One JSON string, escaped. An endpoint is a URI and may carry either of these.</summary>
  private static string Quoted(string value)
  {
    var quoted = new StringBuilder(value.Length + 2).Append('"');
    foreach (var character in value)
    {
      if (character == '"' || character == '\\')
      {
        quoted.Append('\\')
              .Append(character);
      }
      else if (character < ' ')
      {
        quoted.Append('\\')
              .Append('u')
              .Append(((int)character).ToString("x4",
                                                CultureInfo.InvariantCulture));
      }
      else
      {
        quoted.Append(character);
      }
    }

    return quoted.Append('"')
                 .ToString();
  }
}
