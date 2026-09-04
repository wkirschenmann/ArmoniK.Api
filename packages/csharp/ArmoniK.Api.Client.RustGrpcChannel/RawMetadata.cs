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
///   gRPC metadata in the form the C ABI carries it: a uint32 count, then that many
///   length-prefixed key/value pairs.
/// </summary>
/// <remarks>
///   A flat buffer because the boundary passes a pointer and a length and nothing else. A count
///   with lengths rather than a map because gRPC metadata is a multi-map: keys may repeat and
///   their order is kept, so two entries under one key must not come out as one. Native byte
///   order, as the header says, so neither direction picks an endianness of its own.
///   <para>
///     Whether a value is text or bytes is gRPC's rule and not this format's: a key ending in
///     <c>-bin</c> carries binary, and <see cref="Metadata" /> derives that from the key. What
///     this boundary does choose is that a <c>-bin</c> value crosses it decoded rather than as
///     the base64 it travels as on the wire - the engine converts, so nobody converts twice and
///     a malformed one is refused before it reaches here.
///   </para>
///   <para>
///     Reading is total: a buffer that does not parse yields what could be read rather than
///     throwing. Every one of them was written by the library in this process, so a malformed one
///     is a bug on the other side of the ABI, and losing an answer that is already in hand would
///     be the worse way to report it. Where a length prefix is unreadable the rest is unreachable
///     too - the next field's offset is exactly what was lost - so "what could be read" is a
///     prefix in every case, never a salvaged remainder.
///   </para>
/// </remarks>
internal static class RawMetadata
{
  private const string BinarySuffix = "-bin";

  internal static byte[] Encode(Metadata? metadata)
  {
    if (metadata is null || metadata.Count == 0)
    {
      return Array.Empty<byte>();
    }

    // Each entry is read once, and what it yields is kept. Reading one is not free:
    // `ValueBytes` answers with a defensive copy, so asking a binary entry for its length and
    // then for its bytes copies the value twice - and sizing from `GetByteCount` walks every key
    // and text value that the write below then encodes again.
    //
    // The ternary stays: `ValueBytes` on a text entry is documented as its ASCII bytes, so
    // taking it for both would quietly mangle a value that is not ASCII.
    var chunks = new byte[metadata.Count * 2][];
    var size = 4;
    var index = 0;
    foreach (var entry in metadata)
    {
      var key = Encoding.UTF8.GetBytes(entry.Key);
      var value = entry.IsBinary
                    ? entry.ValueBytes
                    : Encoding.UTF8.GetBytes(entry.Value);
      chunks[index++] = key;
      chunks[index++] = value;
      size           += 8 + key.Length + value.Length;
    }

    var raw = new byte[size];
    var at = 0;
    Write(raw,
          ref at,
          (uint)metadata.Count);
    foreach (var chunk in chunks)
    {
      WriteChunk(raw,
                 ref at,
                 chunk);
    }

    return raw;
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
    // Overwritten below on every payload carrying a reason, which is every terminal; the shared
    // empty one stands in for the path where the length prefix itself is unreadable.
    trailers = Metadata.Empty;
    if (!TryReadChunk(ref payload,
                      out var reason))
    {
      return;
    }

    message  = Text(reason);
    trailers = Decode(payload);
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
