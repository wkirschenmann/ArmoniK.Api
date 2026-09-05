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

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal sealed class LentBuffer : SerializationContext, IBufferWriter<byte>, IDisposable
{
  private readonly ulong call_;
  private NativeMethods.AkBuffer buffer_;
  private UnmanagedMemoryManager? block_;
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
    return spilled_ is not null
             ? new Memory<byte>(spilled_,
                                written_,
                                spilled_.Length - written_)
             : Block.Memory.Slice(written_);
  }

  public Span<byte> GetSpan(int sizeHint = 0)
    => GetMemory(sizeHint)
      .Span;

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
    ReleaseBlock();
    if (!lent_)
    {
      return;
    }

    lent_ = false;
    NativeMethods.ak_return_call_buffer(buffer_);
  }

  private int Capacity
    => spilled_?.Length ?? Arena.Length;

  private Span<byte> Arena
    => UnmanagedMemoryManager.Span(buffer_.Ptr,
                                   buffer_.Len);

  private UnmanagedMemoryManager Block
    => block_ ??= new UnmanagedMemoryManager(buffer_.Ptr,
                                             Capacity);

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
        // The manager names the buffer that has just been replaced.
        ReleaseBlock();
        lent_ = true;
        return status;

      case NativeMethods.AkStatus.BudgetBusy:
        spilled_ ??= new byte[length];
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
