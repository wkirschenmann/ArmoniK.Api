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
/// state: holding none is a message not begun or already sent, and both refuse a write through a
/// capacity of zero.
internal sealed class LentBuffer : SerializationContext, IBufferWriter<byte>, IDisposable
{
  private readonly ulong call_;
  private NativeMethods.AkBuffer buffer_;
  private UnmanagedMemoryManager? block_;
  private int written_;

  internal LentBuffer(ulong call)
    => call_ = call;

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
    var wanted = written_ + Math.Max(sizeHint,
                                     1);
    if (Capacity < wanted)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the marshaller announced {Capacity} bytes and then asked to write {wanted}"));
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

    if (Take(payloadLength) == NativeMethods.AkStatus.BudgetBusy)
    {
      // Serializing onto the managed heap instead would answer backpressure with the very
      // allocation the ceiling exists to refuse, and `bytes_used` would never see those bytes.
      throw new NoRoomYet();
    }
  }

  public override IBufferWriter<byte> GetBufferWriter()
    => this;

  /// <summary>Ends the serialization, and checks the announced length was written.</summary>
  /// <remarks>This binding turns Grpc.Core's optional length hint into a hard contract - the
  /// engine lends a buffer of exactly that size and sends exactly that many bytes - so a
  /// serializer that announces more than it writes ships whatever the arena held as message
  /// bytes. Refused here, where it can still be told apart from a transport failure.</remarks>
  public override void Complete()
  {
    if (written_ != Capacity)
    {
      throw new RpcException(new Status(StatusCode.Internal,
                                        $"the serializer announced {Capacity} bytes and wrote {written_}"));
    }
  }

  /// <summary>Ends the serialization with a payload of its own.</summary>
  /// <remarks>The array replaces whatever was announced, buffer included. The first is given back
  /// before the next is asked for, because the engine lends one at a time.</remarks>
  public override void Complete(byte[] payload)
  {
    if (Holding)
    {
      GiveBack();
    }

    if (Take(payload.Length) == NativeMethods.AkStatus.BudgetBusy)
    {
      throw new NoRoomYet();
    }

    new ReadOnlySpan<byte>(payload).CopyTo(Arena);
    written_ = payload.Length;
  }

  /// <summary>Hands the buffer to the engine, which sends it.</summary>
  /// <remarks>What the engine takes back this stops naming, so a view a serializer kept is a
  /// disposed view rather than an arena lent to the next call. A refusal leaves the buffer here,
  /// and disposal returns it.</remarks>
  internal NativeMethods.AkStatus Commit()
  {
    var status = NativeMethods.ak_call_send_message(call_,
                                                    buffer_);
    if (status == NativeMethods.AkStatus.Ok)
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
  private bool Holding
    => buffer_.Owner != IntPtr.Zero;

  private int Capacity
    => UnmanagedMemoryManager.Length(buffer_.Len);

  private Span<byte> Arena
    => UnmanagedMemoryManager.Span(buffer_.Ptr,
                                   buffer_.Len);

  private UnmanagedMemoryManager Block
    => block_ ??= new UnmanagedMemoryManager(buffer_);

  // Reached holding nothing, both callers having made sure of it, so no view can be naming an
  // older buffer here.
  private NativeMethods.AkStatus Take(int length)
  {
    var status = NativeMethods.ak_get_call_buffer(call_,
                                                  (UIntPtr)length,
                                                  out buffer_);
    switch (status)
    {
      case NativeMethods.AkStatus.Ok:
      case NativeMethods.AkStatus.BudgetBusy:
        return status;

      case NativeMethods.AkStatus.InvalidState:
      case NativeMethods.AkStatus.HandleStale:
        throw new CallEnded(status);

      default:
        throw new RpcException(new Status(status == NativeMethods.AkStatus.MessageTooLarge
                                            ? StatusCode.ResourceExhausted
                                            : StatusCode.Internal,
                                          $"no buffer to serialize into ({status})"));
    }
  }

  private void ReleaseBlock()
  {
    ((IDisposable?)block_)?.Dispose();
    block_ = null;
  }
}
