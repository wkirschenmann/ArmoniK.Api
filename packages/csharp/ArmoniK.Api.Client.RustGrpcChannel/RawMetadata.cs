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
using System.Buffers.Binary;
using System.Collections.Generic;
using System.Globalization;
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

    // Sized, then written. Text is measured and later encoded straight into the answer, which
    // walks it twice and allocates nothing; a binary value is taken once and kept, because
    // `ValueBytes` answers with a fresh copy every time it is asked and asking twice would copy
    // the value twice. Metadata is almost always text, and then this allocates once.
    List<byte[]>? binaries = null;
    var size = 4;
    foreach (var entry in metadata)
    {
      size += 8 + Encoding.UTF8.GetByteCount(entry.Key);

      if (entry.IsBinary)
      {
        var bytes = entry.ValueBytes;
        (binaries ??= new List<byte[]>()).Add(bytes);
        size += bytes.Length;
      }
      else
      {
        size += Encoding.UTF8.GetByteCount(entry.Value);
      }
    }

    var raw = new byte[size];
    var at = 0;
    Write(raw,
          ref at,
          (uint)metadata.Count);

    var taken = 0;
    foreach (var entry in metadata)
    {
      WriteText(raw,
                ref at,
                entry.Key);

      if (entry.IsBinary)
      {
        WriteChunk(raw,
                   ref at,
                   binaries![taken++]);
      }
      else
      {
        WriteText(raw,
                  ref at,
                  entry.Value);
      }
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

  // Native order, which is what the header says these integers are in and what the engine
  // writes. This blob crosses a process boundary and not a wire, so both sides read what the
  // machine they share writes - fixing it to one endianness would make them disagree on the
  // machine that has the other. `BitConverter.GetBytes` would say the same thing and allocate
  // four bytes to say it, once per length written.
  private static void Write(byte[] into,
                            ref int at,
                            uint value)
  {
    var head = new Span<byte>(into,
                              at,
                              4);
    if (BitConverter.IsLittleEndian)
    {
      BinaryPrimitives.WriteUInt32LittleEndian(head,
                                               value);
    }
    else
    {
      BinaryPrimitives.WriteUInt32BigEndian(head,
                                            value);
    }

    at += 4;
  }

  private static void WriteText(byte[] into,
                                ref int at,
                                string value)
  {
    // Encoded over the four bytes its length will occupy, because the length is what the encoder
    // answers and reserving it first would mean counting the value a third time.
    var length = Encoding.UTF8.GetBytes(value,
                                        0,
                                        value.Length,
                                        into,
                                        at + 4);
    Write(into,
          ref at,
          (uint)length);
    at += length;
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
    var value = BitConverter.IsLittleEndian
                  ? BinaryPrimitives.ReadUInt32LittleEndian(from)
                  : BinaryPrimitives.ReadUInt32BigEndian(from);
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
