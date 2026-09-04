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

internal sealed class ChannelOptions
{
  internal string Endpoint { get; set; } = string.Empty;

  internal int DeliveryCredits { get; set; }

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
