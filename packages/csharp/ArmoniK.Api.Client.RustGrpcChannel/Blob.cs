using System;
using System.Collections.Generic;
using System.Text;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   The ABI's key/value encoding: a uint32 count, then that many length-prefixed pairs.
/// </summary>
/// <remarks>
///   Native byte order, as the header says, so this reads and writes with <see cref="BitConverter" />
///   rather than choosing an endianness of its own. Keys may repeat and their order is kept: gRPC
///   metadata is a multi-map, and two entries under one key must not come out as one.
///   <para>
///     A key ending in <c>-bin</c> carries raw bytes on both sides of this boundary: the library
///     hands over the decoded value, not its base64 form.
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

    var pairs = new List<KeyValuePair<byte[], byte[]>>(metadata.Count);
    foreach (var entry in metadata)
    {
      var value = entry.IsBinary
                    ? entry.ValueBytes
                    : Encoding.UTF8.GetBytes(entry.Value);
      pairs.Add(new KeyValuePair<byte[], byte[]>(Encoding.UTF8.GetBytes(entry.Key),
                                                 value));
    }

    var size = 4;
    foreach (var pair in pairs)
    {
      size += 8 + pair.Key.Length + pair.Value.Length;
    }

    var blob = new byte[size];
    var at = 0;
    WriteUInt32(blob,
                ref at,
                (uint)pairs.Count);
    foreach (var pair in pairs)
    {
      WriteChunk(blob,
                 ref at,
                 pair.Key);
      WriteChunk(blob,
                 ref at,
                 pair.Value);
    }

    return blob;
  }

  /// <summary>
  ///   Reads a blob the library produced. A truncated one yields what could be read rather than
  ///   throwing: this runs while an answer is already in hand, and losing the answer over a
  ///   malformed header would be the worse outcome.
  /// </summary>
  internal static Metadata Decode(byte[] blob)
  {
    var metadata = new Metadata();
    if (blob.Length < 4)
    {
      return metadata;
    }

    var at = 0;
    var count = ReadUInt32(blob,
                           ref at);
    for (var index = 0; index < count; index++)
    {
      if (!TryReadChunk(blob,
                        ref at,
                        out var key) || !TryReadChunk(blob,
                                                      ref at,
                                                      out var value))
      {
        break;
      }

      var name = Encoding.UTF8.GetString(key);
      if (name.EndsWith(BinarySuffix,
                        StringComparison.OrdinalIgnoreCase))
      {
        metadata.Add(name,
                     value);
      }
      else
      {
        metadata.Add(name,
                     Encoding.UTF8.GetString(value));
      }
    }

    return metadata;
  }

  /// <summary>
  ///   The terminal's payload: a length-prefixed reason, then the trailing metadata as a blob. The
  ///   status code itself travels beside it, in the event.
  /// </summary>
  internal static void DecodeStatus(byte[] payload,
                                    out string message,
                                    out Metadata trailers)
  {
    message  = string.Empty;
    trailers = new Metadata();
    if (payload.Length < 4)
    {
      return;
    }

    var at = 0;
    if (!TryReadChunk(payload,
                      ref at,
                      out var reason))
    {
      return;
    }

    message = Encoding.UTF8.GetString(reason);
    var rest = new byte[payload.Length - at];
    Buffer.BlockCopy(payload,
                     at,
                     rest,
                     0,
                     rest.Length);
    trailers = Decode(rest);
  }

  private static void WriteUInt32(byte[] into,
                                  ref int at,
                                  uint value)
  {
    var bytes = BitConverter.GetBytes(value);
    Buffer.BlockCopy(bytes,
                     0,
                     into,
                     at,
                     4);
    at += 4;
  }

  private static void WriteChunk(byte[] into,
                                 ref int at,
                                 byte[] chunk)
  {
    WriteUInt32(into,
                ref at,
                (uint)chunk.Length);
    Buffer.BlockCopy(chunk,
                     0,
                     into,
                     at,
                     chunk.Length);
    at += chunk.Length;
  }

  private static uint ReadUInt32(byte[] from,
                                 ref int at)
  {
    var value = BitConverter.ToUInt32(from,
                                      at);
    at += 4;
    return value;
  }

  private static bool TryReadChunk(byte[] from,
                                   ref int at,
                                   out byte[] chunk)
  {
    chunk = Array.Empty<byte>();
    if (from.Length - at < 4)
    {
      return false;
    }

    var length = ReadUInt32(from,
                            ref at);
    if (length > (uint)(from.Length - at))
    {
      return false;
    }

    chunk = new byte[length];
    Buffer.BlockCopy(from,
                     at,
                     chunk,
                     0,
                     (int)length);
    at += (int)length;
    return true;
  }
}
