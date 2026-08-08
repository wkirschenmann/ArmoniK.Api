using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   The ABI's key/value encoding: a uint32 count, then that many length-prefixed pairs.
/// </summary>
/// <remarks>
///   Native byte order, as the header says, so this reads and writes with <see cref="BitConverter" />
///   rather than choosing an endianness of its own. Keys may repeat and their order is kept: HTTP
///   allows both, and a response with two `set-cookie` headers must not come out as one.
/// </remarks>
internal static class Blob
{
  internal static byte[] Encode(IReadOnlyList<KeyValuePair<string, string>> pairs)
  {
    using var buffer = new MemoryStream();
    using var writer = new BinaryWriter(buffer);

    writer.Write((uint)pairs.Count);
    foreach (var pair in pairs)
    {
      WriteChunk(writer,
                 Encoding.UTF8.GetBytes(pair.Key));
      WriteChunk(writer,
                 Encoding.UTF8.GetBytes(pair.Value));
    }

    writer.Flush();
    return buffer.ToArray();
  }

  private static void WriteChunk(BinaryWriter writer,
                                 byte[]       data)
  {
    writer.Write((uint)data.Length);
    writer.Write(data);
  }

  internal static List<KeyValuePair<string, string>> Decode(byte[] blob)
  {
    var pairs = new List<KeyValuePair<string, string>>();
    if (blob.Length == 0)
    {
      return pairs;
    }

    var offset = 0;
    var count  = (int)BitConverter.ToUInt32(blob,
                                            offset);
    offset += 4;

    for (var index = 0; index < count; index++)
    {
      var key   = ReadChunk(blob, ref offset);
      var value = ReadChunk(blob, ref offset);
      pairs.Add(new KeyValuePair<string, string>(key,
                                                 value));
    }

    return pairs;
  }

  private static string ReadChunk(byte[]   blob,
                                  ref int  offset)
  {
    var length = (int)BitConverter.ToUInt32(blob,
                                            offset);
    offset += 4;
    var text = Encoding.UTF8.GetString(blob,
                                       offset,
                                       length);
    offset += length;
    return text;
  }
}
