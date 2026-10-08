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
using System.Buffers;

using Grpc.Core;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>One message, serialized into a buffer the engine lends.</summary>
///
/// An instance serves that message and is then thrown away, which is what lets the buffer be asked
/// for at the announced length and the written count be checked against it. The buffer is the
/// state: holding none is a message not begun, with no length announced or an empty one, and a
/// write then lends the first buffer.
///
/// A message that turns out longer than it announced is not refused: the buffer is exchanged for a
/// larger one, with what was written carried over. Memory the serializer holds then is a disposed
/// view; a span is a pointer into an arena the engine may lend again, which the IBufferWriter
/// contract allows: nothing obtained before a request for more room is written through after it.
/// A request for room the ceiling has none of yet is answered by an exception that leaves the
/// serializer, so a serializer lets what `GetMemory` throws out.
internal sealed class LentBuffer : SerializationContext, IBufferWriter<byte>, IDisposable
{
  private const int Page = 4096;

  /// <summary>The engine's exchange of a lent buffer for another.</summary>
  internal delegate ak_status Exchange(ak_buffer buffer,
                                       nuint     length,
                                       nuint     keep,
                                       out ak_buffer resized);

  /// <summary>What exchanges the buffer, which a test replaces to make the allocator fail where a
  /// real one will not within the ceiling.</summary>
  internal static Exchange Exchanger = Engine;

  private readonly int atLeast_;
  private readonly ulong call_;
  private int announced_;
  private ak_buffer buffer_;
  private UnmanagedMemoryManager? block_;
  private int written_;

  /// <param name="call">The call the buffer is lent by.</param>
  /// <param name="atLeast">The length a lend is made at when the announcement is shorter: the room
  /// the message is known to need.</param>
  internal LentBuffer(ulong call,
                      int   atLeast = 0)
  {
    call_    = call;
    atLeast_ = atLeast;
  }

  /// <summary>The least length the engine's ceiling had no room for yet, or zero: what the next
  /// attempt at this message lends at, so that the wait is for the room the message has been found
  /// to need and not for the room the announcement did.</summary>
  internal int Needed { get; private set; }

  /// <summary>Whether the ceiling's refusal was of an exchange, which records no wait: the engine
  /// owes this call no wake-up for it, and the way to wait is to lend at <see cref="Needed" />.</summary>
  internal bool RefusedExchange { get; private set; }

  public void Advance(int count)
  {
    if (count < 0 || written_ + count > Capacity)
    {
      throw new ArgumentOutOfRangeException(nameof(count),
                                            $"{count} bytes do not fit the {Capacity - written_} left");
    }

    written_ += count;
  }

  public Memory<byte> GetMemory(int sizeHint = 0)
  {
    var wanted = (long)written_ + Math.Max(sizeHint,
                                           1);
    if (Capacity < wanted)
    {
      Grow(wanted);
    }

    return Block.Memory.Slice(written_);
  }

  public Span<byte> GetSpan(int sizeHint = 0)
    => GetMemory(sizeHint)
      .Span;

  public override void SetPayloadLength(int payloadLength)
  {
    if (payloadLength < 0)
    {
      throw new ArgumentOutOfRangeException(nameof(payloadLength),
                                            payloadLength,
                                            "a message is not a negative number of bytes");
    }

    // A second announcement is the serializer's mistake, not the call's: the engine lends one
    // buffer at a time and refuses the second ask with the status it also uses for a call that
    // has ended, so left to it the caller reads that its call was cancelled.
    if (Holding)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the serializer announced a length twice: {Capacity} bytes, then {payloadLength}"));
    }

    // An empty message needs no buffer: the engine sends it with none, and refuses a lend of no
    // bytes.
    announced_ = payloadLength;
    if (payloadLength == 0)
    {
      return;
    }

    if (Take(Math.Max(payloadLength,
                      atLeast_)) == ak_status.AK_STATUS_BUDGET_BUSY)
    {
      // Serializing onto the managed heap instead would answer backpressure with the very
      // allocation the ceiling exists to refuse, and `bytes_used` would never see those bytes.
      throw new NoRoomYet();
    }
  }

  public override IBufferWriter<byte> GetBufferWriter()
    => this;

  /// <summary>Ends the serialization, and checks the announced length was written.</summary>
  /// <remarks>This binding turns Grpc.Core's optional length hint into a hard contract: a
  /// serializer that writes less than it announced computed its size from one message and wrote
  /// another, and is refused here, where it can still be told apart from a transport failure.
  /// One that writes more has asked for the room and been given it, so the message is what it
  /// wrote.</remarks>
  public override void Complete()
  {
    if (written_ < announced_)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the serializer announced {announced_} bytes and wrote {written_}"));
    }
  }

  /// <summary>Ends the serialization with a payload of its own.</summary>
  /// <remarks>The array replaces whatever was announced: the buffer is exchanged for one of its
  /// length, which the ceiling sees as the difference alone, with no moment at which another call
  /// can take the room the buffer held.</remarks>
  public override void Complete(byte[] payload)
  {
    if (payload.Length == 0)
    {
      if (Holding)
      {
        GiveBack();
      }

      return;
    }

    if (Holding)
    {
      if (Capacity != payload.Length)
      {
        Settle(Resize(payload.Length,
                      0),
               payload.Length);
      }
    }
    else if (Take(payload.Length) == ak_status.AK_STATUS_BUDGET_BUSY)
    {
      throw new NoRoomYet();
    }

    new ReadOnlySpan<byte>(payload).CopyTo(Arena);
    written_ = payload.Length;
  }

  /// <summary>Hands the buffer to the engine, which sends the bytes written.</summary>
  /// <remarks>What the engine takes back this stops naming, so a view a serializer kept is a
  /// disposed view rather than an arena lent to the next call. An overrun is taken back too, and
  /// shuts the runtime down. Any other refusal leaves the buffer here, and disposal returns
  /// it.</remarks>
  internal unsafe ak_status Commit()
  {
    var status = NativeMethods.ak_call_send_message(call_,
                                                    buffer_,
                                                    (nuint)written_,
                                                    null);
    if (status is ak_status.AK_STATUS_OK or ak_status.AK_STATUS_CORRUPTED)
    {
      ReleaseBlock();
      buffer_ = default;
    }

    return status;
  }

  public void Dispose()
  {
    if (Holding)
    {
      GiveBack();
    }
  }

  private void GiveBack()
  {
    ReleaseBlock();
    NativeMethods.ak_return_call_buffer(buffer_);
    buffer_  = default;
    written_ = 0;
  }

  /// <summary>The token a lend hands over and a return consumes, so it is the lend.</summary>
  private unsafe bool Holding
    => buffer_.owner != null;

  private int Capacity
    => UnmanagedMemoryManager.Length(buffer_.len);

  private unsafe Span<byte> Arena
    => UnmanagedMemoryManager.Span(buffer_.ptr,
                                   buffer_.len);

  private UnmanagedMemoryManager Block
    => block_ ??= new UnmanagedMemoryManager(buffer_);

  // Reached holding nothing, all callers having made sure of it, so no view can be naming an
  // older buffer here.
  private unsafe ak_status Take(int length)
  {
    ak_status status;
    fixed (ak_buffer* lent = &buffer_)
    {
      status = NativeMethods.ak_get_call_buffer(call_,
                                                (nuint)length,
                                                lent,
                                                null);
    }

    switch (status)
    {
      case ak_status.AK_STATUS_OK:
        return status;

      case ak_status.AK_STATUS_BUDGET_BUSY:
        Needed = length;
        return status;

      case ak_status.AK_STATUS_INVALID_STATE:
      case ak_status.AK_STATUS_HANDLE_STALE:
        throw new CallEnded(status);

      // The send window is full, which means another send of this call is unacquitted. The header
      // calls it backpressure and names the next WRITE_DONE as its wake-up, which is what a host
      // pipelining deeper than one message waits on; this binding admits one writer and has it
      // wait for the acquittal, so reaching this is its own bookkeeping being wrong rather than a
      // resource to wait for.
      case ak_status.AK_STATUS_SLOT_BUSY:
        throw new RpcException(new Status(StatusCode.Internal,
                                          "a send was begun while this call still had one unacquitted"));

      default:
        throw new RpcException(new Status(status == ak_status.AK_STATUS_MESSAGE_TOO_LARGE
                                            ? StatusCode.ResourceExhausted
                                            : StatusCode.Internal,
                                          $"no buffer to serialize into ({status})"));
    }
  }

  /// <summary>Makes room for <paramref name="wanted" /> bytes, with what was written kept.</summary>
  /// <remarks>Twice the capacity when that is more, so a serializer that asks for a byte at a time
  /// is not a copy of the message for each. The ceiling's refusal of that, for room or for the
  /// size itself, is not the answer while less would do: a quarter more than the buffer holds, and
  /// never less than a page, then halves of the distance down to what was asked. A refusal costs a
  /// downcall that allocates nothing, and a message sized against a tight ceiling takes the
  /// largest size that fits, so the attempts grow with the logarithm of the size and not with its
  /// length.</remarks>
  private void Grow(long wanted)
  {
    if (wanted > int.MaxValue)
    {
      throw new RpcException(new Status(StatusCode.ResourceExhausted,
                                        $"a message of {wanted} bytes does not fit one buffer"));
    }

    // Nothing is lent when the serializer announced no length, or zero: the first bytes are the
    // first lend, whose refusal for room is a wait the engine wakes.
    if (!Holding)
    {
      if (Take(Math.Max((int)wanted,
                        atLeast_)) == ak_status.AK_STATUS_BUDGET_BUSY)
      {
        throw new NoRoomYet();
      }

      return;
    }

    var length = (int)Math.Min(Math.Max(wanted,
                                        2L * Capacity),
                               int.MaxValue);
    var least = (int)Math.Min(length,
                              Math.Max(wanted,
                                       (long)Capacity + Math.Max(Page,
                                                                 Capacity / 4)));

    // A smaller size is refused for room only when every larger one was, so what is left of the
    // refusal after the last try is the room what was asked for needs. An allocator that fails
    // on a larger size may serve what was asked for, so that is the one retry it gets.
    var status = Resize(length,
                        written_);
    while (status != ak_status.AK_STATUS_OK && length > wanted)
    {
      length = status == ak_status.AK_STATUS_INTERNAL
                 ? (int)wanted
                 : length > least
                   ? least
                   : (int)(wanted + (length - wanted) / 2);
      status = Resize(length,
                      written_);
    }

    Settle(status,
           (int)wanted);
  }

  /// <summary>What the exchange of the buffer for one of <paramref name="length" /> bytes came
  /// to: nothing to say when it was made, the wait for room when the ceiling has none yet, and a
  /// refusal of the message when no room will ever do.</summary>
  private void Settle(ak_status status,
                      int       length)
  {
    switch (status)
    {
      case ak_status.AK_STATUS_OK:
        Needed          = 0;
        RefusedExchange = false;
        return;

      case ak_status.AK_STATUS_BUDGET_BUSY:
        Needed          = length;
        RefusedExchange = true;
        throw new NoRoomYet();

      case ak_status.AK_STATUS_INTERNAL:
        throw new RpcException(new Status(StatusCode.Internal,
                                          $"no buffer of {length} bytes to serialize into ({status})"));

      default:
        throw new RpcException(new Status(StatusCode.ResourceExhausted,
                                          $"a message of {length} bytes does not fit the ceiling"));
    }
  }

  /// <summary>Exchanges the buffer for one of <paramref name="length" /> bytes, keeping the first
  /// <paramref name="keep" /> of what was written, unless the ceiling has no room for it, now or
  /// ever, or the allocator failed, each of which is a status of its own.</summary>
  /// <remarks>Whatever else is refused leaves the buffer lent, and disposal returns it. What
  /// succeeds disposes the memory the serializer was handed, which names the old arena.</remarks>
  private ak_status Resize(int length,
                           int keep)
  {
    var status = Exchanger(buffer_,
                           (nuint)length,
                           (nuint)keep,
                           out var resized);
    switch (status)
    {
      case ak_status.AK_STATUS_OK:
        ReleaseBlock();
        buffer_ = resized;
        return status;

      // An allocator failure leaves the buffer lent too, and a smaller size may be one the
      // allocator serves.
      case ak_status.AK_STATUS_BUDGET_BUSY:
      case ak_status.AK_STATUS_MESSAGE_TOO_LARGE:
      case ak_status.AK_STATUS_INTERNAL:
        return status;

      case ak_status.AK_STATUS_INVALID_STATE:
      case ak_status.AK_STATUS_HANDLE_STALE:
        throw new CallEnded(status);

      // The engine has taken the buffer back, unfreed, and is shutting down: nothing is left to
      // give back, and the serializer wrote past what it was lent.
      case ak_status.AK_STATUS_CORRUPTED:
        ReleaseBlock();
        buffer_  = default;
        written_ = 0;
        throw new RpcException(new Status(StatusCode.Internal,
                                          "the serializer wrote past the buffer it was lent"));

      default:
        throw new RpcException(new Status(StatusCode.Internal,
                                          $"no buffer to serialize into ({status})"));
    }
  }

  internal static unsafe ak_status Engine(ak_buffer     buffer,
                                          nuint         length,
                                          nuint         keep,
                                          out ak_buffer resized)
  {
    ak_buffer made = default;
    var status = NativeMethods.ak_resize_call_buffer(buffer,
                                                     length,
                                                     keep,
                                                     &made,
                                                     null);
    resized = made;
    return status;
  }

  private void ReleaseBlock()
  {
    ((IDisposable?)block_)?.Dispose();
    block_ = null;
  }
}
