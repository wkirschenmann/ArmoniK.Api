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

internal static class RawMetadata
{
  private const string BinarySuffix = "-bin";

  internal static byte[] Encode(Metadata? metadata)
  {
    if (metadata is null || metadata.Count == 0)
    {
      return Array.Empty<byte>();
    }

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

  internal static void DecodeStatus(ReadOnlySpan<byte> payload,
                                    out string message,
                                    out Metadata trailers)
  {
    message  = string.Empty;
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
