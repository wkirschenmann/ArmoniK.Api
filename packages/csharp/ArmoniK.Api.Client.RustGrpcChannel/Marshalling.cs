using System;
using System.Buffers;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   Where a request message serializes to.
/// </summary>
/// <remarks>
///   The contextual half of a <see cref="Marshaller{T}" /> is the only half a generated stub
///   implements: its <c>Serializer</c> throws, so this is not an optimisation but the entry point.
/// </remarks>
internal sealed class SerializedMessage : SerializationContext, IBufferWriter<byte>
{
  private byte[] bytes_ = Array.Empty<byte>();
  private int written_;

  /// <summary>The message's bytes, and nothing past them.</summary>
  internal byte[] Bytes
  {
    get
    {
      if (written_ == bytes_.Length)
      {
        return bytes_;
      }

      var exact = new byte[written_];
      Array.Copy(bytes_,
                 exact,
                 written_);
      return exact;
    }
  }

  /// <inheritdoc />
  public void Advance(int count)
  {
    if (count < 0 || written_ + count > bytes_.Length)
    {
      throw new ArgumentOutOfRangeException(nameof(count),
                                            $"{count} bytes do not fit the {bytes_.Length - written_} left");
    }

    written_ += count;
  }

  /// <inheritdoc />
  public Memory<byte> GetMemory(int sizeHint = 0)
  {
    Reserve(written_ + Math.Max(sizeHint,
                                1));
    return new Memory<byte>(bytes_,
                            written_,
                            bytes_.Length - written_);
  }

  /// <inheritdoc />
  public Span<byte> GetSpan(int sizeHint = 0)
    => GetMemory(sizeHint)
      .Span;

  /// <inheritdoc />
  public override void SetPayloadLength(int payloadLength)
    => Reserve(payloadLength);

  /// <inheritdoc />
  public override IBufferWriter<byte> GetBufferWriter()
    => this;

  /// <inheritdoc />
  public override void Complete()
  {
  }

  /// <inheritdoc />
  public override void Complete(byte[] payload)
  {
    bytes_   = payload;
    written_ = payload.Length;
  }

  private void Reserve(int total)
  {
    if (bytes_.Length >= total)
    {
      return;
    }

    var grown = new byte[Math.Max(total,
                                  bytes_.Length * 2)];
    Array.Copy(bytes_,
               grown,
               written_);
    bytes_ = grown;
  }
}

/// <summary>What a response message deserializes from.</summary>
internal sealed class ReceivedMessage : DeserializationContext
{
  private readonly byte[] bytes_;

  internal ReceivedMessage(byte[] bytes)
    => bytes_ = bytes;

  /// <inheritdoc />
  public override int PayloadLength
    => bytes_.Length;

  /// <inheritdoc />
  // Already the copy the trampoline made out of the library's memory, for this call alone.
  public override byte[] PayloadAsNewBuffer()
    => bytes_;

  /// <inheritdoc />
  public override ReadOnlySequence<byte> PayloadAsReadOnlySequence()
    => new(bytes_);
}
