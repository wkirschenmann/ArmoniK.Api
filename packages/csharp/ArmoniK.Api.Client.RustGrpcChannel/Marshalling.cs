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

/// <summary>
///   Where a request message serializes to: the buffer the engine lends for it.
/// </summary>
/// <remarks>
///   The contextual half of a <see cref="Marshaller{T}" /> is the only half a generated stub
///   implements: its <c>Serializer</c> throws, so this is not an optimisation but the entry point.
///   <para>
///     The lend needs the length, and <see cref="SetPayloadLength" /> is where the marshaller
///     first says it, so that is where it happens. A refusal there cannot wait for room, being a
///     synchronous callback, so it serializes into managed memory instead and leaves the wait to
///     the caller: the arena path stays copy-free, and the copy appears only under a ceiling
///     that is already refusing work.
///   </para>
///   <para>
///     Disposing gives back a buffer that was lent and not handed over. That is the whole of the
///     host's half of the contract, and it covers a throwing marshaller as well as a refusal.
///   </para>
/// </remarks>
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
    // The legacy path never announced a length, so nothing was lent and this is already managed.
    spilled_ = payload;
    written_ = payload.Length;
  }

  /// <summary>
  ///   Hands the message to the engine. Answers what the ABI answered, so a refusal that only
  ///   means "not now" stays distinguishable from one that means "never".
  /// </summary>
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

  /// <summary>Checks that what the marshaller is about to write still fits.</summary>
  /// <remarks>
  ///   A check, and never a growth. The buffer is lent for the length the marshaller announced
  ///   through <see cref="SetPayloadLength" /> - Grpc.Core's protobuf marshaller announces
  ///   <c>CalculateSize()</c> and then writes exactly that much - so asking past it is a
  ///   marshaller contradicting itself, and there is nothing sensible to serialize into.
  ///   <para>
  ///     Growing instead meant returning the lent buffer half way through serializing and
  ///     carrying on in managed memory, which leaves a serializing writer holding no buffer at
  ///     all - a state <c>SerializingWriterHoldsTheBuffer</c> forbids, reachable only by that
  ///     contradiction.
  ///   </para>
  /// </remarks>
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

      // The byte ceiling is a wait, so the message is serialized into managed memory and
      // `Commit` lends again. SLOT_BUSY is not: its wake-up is this call's next WRITE_DONE, and
      // `ManagedWriterNeverObservesSlotBusy` says a single writer with a window of one never
      // reaches it, so it falls through to the refusal below.
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

/// <summary>What a response message deserializes from, in the library's own memory.</summary>
/// <remarks>
///   Valid only for the callback that delivered it, which is why nothing here outlives the drain
///   step that builds it.
/// </remarks>
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

/// <summary>A <see cref="Memory{T}" /> over memory the garbage collector does not know about.</summary>
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
