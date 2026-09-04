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
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   The ABI's key/value encoding: a uint32 count, then that many length-prefixed pairs.
/// </summary>
/// <remarks>
///   Native byte order, as the header says, so neither direction chooses an endianness of its own.
///   Keys may repeat and their order is kept: gRPC metadata is a multi-map, and two entries under
///   one key must not come out as one.
///   <para>
///     A key ending in <c>-bin</c> carries raw bytes on both sides of this boundary: the library
///     hands over the decoded value, not its base64 form.
///   </para>
///   <para>
///     Reading is total: a blob that does not parse yields what could be read rather than
///     throwing. Every one of them was written by the library in this process, so a malformed one
///     is a bug on the other side of the ABI, and losing an answer that is already in hand would
///     be the worse way to report it. Where a length prefix is unreadable the rest is unreachable
///     too - the next field's offset is exactly what was lost - so "what could be read" is a
///     prefix in every case, never a salvaged remainder.
///   </para>
/// </remarks>
internal static class Blob
{
  private const string BinarySuffix = "-bin";

  internal static byte[] Encode(Metadata? metadata)
  {
    if (metadata is null || metadata.Count == 0)
    {
      return Array.Empty<byte>();
    }

    var size = 4;
    foreach (var entry in metadata)
    {
      size += 8 + Encoding.UTF8.GetByteCount(entry.Key) + (entry.IsBinary
                                                             ? entry.ValueBytes.Length
                                                             : Encoding.UTF8.GetByteCount(entry.Value));
    }

    var blob = new byte[size];
    var at = 0;
    Write(blob,
          ref at,
          (uint)metadata.Count);
    foreach (var entry in metadata)
    {
      WriteChunk(blob,
                 ref at,
                 Encoding.UTF8.GetBytes(entry.Key));
      WriteChunk(blob,
                 ref at,
                 entry.IsBinary
                   ? entry.ValueBytes
                   : Encoding.UTF8.GetBytes(entry.Value));
    }

    return blob;
  }

  internal static Metadata Decode(ReadOnlySpan<byte> blob)
  {
    var metadata = new Metadata();
    if (blob.Length < 4)
    {
      return metadata;
    }

    var count = Read(ref blob);
    for (var index = 0; index < count; index++)
    {
      if (!TryReadChunk(ref blob,
                        out var key) || !TryReadChunk(ref blob,
                                                      out var value))
      {
        break;
      }

      var name = Text(key);
      if (name.EndsWith(BinarySuffix,
                        StringComparison.OrdinalIgnoreCase))
      {
        metadata.Add(name,
                     value.ToArray());
      }
      else
      {
        metadata.Add(name,
                     Text(value));
      }
    }

    return metadata;
  }

  /// <summary>
  ///   The terminal's payload: a length-prefixed reason, then the trailing metadata as a blob. The
  ///   status code itself travels beside it, in the event.
  /// </summary>
  internal static void DecodeStatus(ReadOnlySpan<byte> payload,
                                    out string message,
                                    out Metadata trailers)
  {
    message  = string.Empty;
    trailers = new Metadata();
    if (!TryReadChunk(ref payload,
                      out var reason))
    {
      return;
    }

    message  = Text(reason);
    trailers = Decode(payload);
  }

  /// <summary>
  ///   The channel config the engine parses, as JSON.
  /// </summary>
  /// <remarks>
  ///   Here beside the other encoding this ABI asks for, and not in the channel: the header lists
  ///   more options than these two and refuses one it does not know rather than ignoring it, so
  ///   the next option added wants one obvious home. Written by hand because a netstandard2.0
  ///   target would need a package to do it any other way, and the shape is two fields.
  /// </remarks>
  internal static byte[] ChannelConfig(string endpoint,
                                       int deliveryCredits)
    => Encoding.UTF8.GetBytes("{"
                              + Quote("endpoint")
                              + ":"
                              + Quote(endpoint)
                              + ","
                              + Quote("delivery_credits")
                              + ":"
                              + deliveryCredits.ToString(CultureInfo.InvariantCulture)
                              + "}");

  /// <summary>One JSON string, escaped. An endpoint is a URI and may carry either of these.</summary>
  private static string Quote(string value)
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

  private static void Write(byte[] into,
                            ref int at,
                            uint value)
  {
    // `BitConverter` rather than `MemoryMarshal.Write`, which would be the reader's exact
    // mirror: its second parameter is `ref` on netstandard2.0 and `in` on net8.0, so one call
    // cannot satisfy both targets below C# 12, and native byte order is what matters here.
    BitConverter.GetBytes(value)
                .CopyTo(into,
                        at);
    at += 4;
  }

  private static void WriteChunk(byte[] into,
                                 ref int at,
                                 byte[] chunk)
  {
    Write(into,
          ref at,
          (uint)chunk.Length);
    chunk.CopyTo(into,
                 at);
    at += chunk.Length;
  }

  private static uint Read(ref ReadOnlySpan<byte> from)
  {
    var value = MemoryMarshal.Read<uint>(from);
    from = from.Slice(4);
    return value;
  }

  private static bool TryReadChunk(ref ReadOnlySpan<byte> from,
                                   out ReadOnlySpan<byte> chunk)
  {
    chunk = default;
    if (from.Length < 4)
    {
      return false;
    }

    var length = Read(ref from);
    if (length > (uint)from.Length)
    {
      return false;
    }

    chunk = from.Slice(0,
                       (int)length);
    from  = from.Slice((int)length);
    return true;
  }

  private static unsafe string Text(ReadOnlySpan<byte> bytes)
  {
    if (bytes.IsEmpty)
    {
      return string.Empty;
    }

    fixed (byte* start = bytes)
    {
      return Encoding.UTF8.GetString(start,
                                     bytes.Length);
    }
  }
}
