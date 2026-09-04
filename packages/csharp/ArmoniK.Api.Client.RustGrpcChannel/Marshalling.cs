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
using System.Runtime.InteropServices;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal sealed class LentBuffer : SerializationContext, IBufferWriter<byte>, IDisposable
{
  private readonly ulong call_;
  private NativeMethods.AkBuffer buffer_;
  private UnmanagedBlock? block_;
  private byte[]? spilled_;
  private int written_;
  private bool lent_;

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
    Reserve(sizeHint);
    if (spilled_ is not null)
    {
      return new Memory<byte>(spilled_,
                              written_,
                              spilled_.Length - written_);
    }

    block_ ??= new UnmanagedBlock(buffer_.Ptr,
                                  (int)buffer_.Len);
    return block_.Memory.Slice(written_);
  }

  public Span<byte> GetSpan(int sizeHint = 0)
  {
    Reserve(sizeHint);
    return spilled_ is not null
             ? new Span<byte>(spilled_,
                              written_,
                              spilled_.Length - written_)
             : Arena.Slice(written_);
  }

  public override void SetPayloadLength(int payloadLength)
    => Take(payloadLength);

  public override IBufferWriter<byte> GetBufferWriter()
    => this;

  public override void Complete()
  {
  }

  public override void Complete(byte[] payload)
  {
    spilled_ = payload;
    written_ = payload.Length;
  }

  internal NativeMethods.AkStatus Commit()
  {
    if (spilled_ is not null)
    {
      var taken = Take(written_);
      if (taken != NativeMethods.AkStatus.Ok)
      {
        return taken;
      }

      new ReadOnlySpan<byte>(spilled_,
                             0,
                             written_).CopyTo(Arena);
      spilled_ = null;
    }

    var status = NativeMethods.ak_call_send_message(call_,
                                                    buffer_);
    if (status == NativeMethods.AkStatus.Ok)
    {
      lent_ = false;
    }

    return status;
  }

  public void Dispose()
  {
    ((IDisposable?)block_)?.Dispose();
    block_ = null;
    if (!lent_)
    {
      return;
    }

    lent_ = false;
    NativeMethods.ak_return_call_buffer(buffer_);
  }

  private int Capacity
    => spilled_?.Length ?? (int)buffer_.Len;

  private unsafe Span<byte> Arena
    => new((void*)buffer_.Ptr,
           (int)buffer_.Len);

  private void Reserve(int sizeHint)
  {
    var wanted = written_ + Math.Max(sizeHint,
                                     1);
    if (Capacity >= wanted)
    {
      return;
    }

    throw new RpcException(new Status(StatusCode.Internal,
                                      $"the marshaller announced {Capacity} bytes and then asked to write {wanted}"));
  }

  private NativeMethods.AkStatus Take(int length)
  {
    var status = NativeMethods.ak_get_call_buffer(call_,
                                                  (UIntPtr)length,
                                                  out buffer_);
    switch (status)
    {
      case NativeMethods.AkStatus.Ok:
        lent_ = true;
        return status;

      case NativeMethods.AkStatus.BudgetBusy:
        spilled_ ??= new byte[length];
        return status;

      default:
        throw new RpcException(new Status(status == NativeMethods.AkStatus.MessageTooLarge
                                            ? StatusCode.ResourceExhausted
                                            : StatusCode.Internal,
                                          $"no buffer to serialize into ({status})"));
    }
  }
}

internal sealed class ReceivedMessage : DeserializationContext
{
  private readonly IntPtr start_;
  private readonly int length_;

  internal ReceivedMessage(IntPtr start,
                           int length)
  {
    start_  = start;
    length_ = length;
  }

  public override int PayloadLength
    => length_;

  public override byte[] PayloadAsNewBuffer()
  {
    var bytes = new byte[length_];
    Marshal.Copy(start_,
                 bytes,
                 0,
                 length_);
    return bytes;
  }

  public override ReadOnlySequence<byte> PayloadAsReadOnlySequence()
    => new(new UnmanagedBlock(start_,
                              length_).Memory);
}

internal sealed class UnmanagedBlock : MemoryManager<byte>
{
  private readonly IntPtr start_;
  private readonly int length_;

  internal UnmanagedBlock(IntPtr start,
                          int length)
  {
    start_  = start;
    length_ = length;
  }

  public override unsafe Span<byte> GetSpan()
    => new((void*)start_,
           length_);

  public override unsafe MemoryHandle Pin(int elementIndex = 0)
    => new((byte*)start_ + elementIndex);

  public override void Unpin()
  {
  }

  protected override void Dispose(bool disposing)
  {
  }
}
