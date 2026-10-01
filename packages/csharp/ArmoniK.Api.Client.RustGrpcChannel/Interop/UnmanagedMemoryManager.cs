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

namespace ArmoniK.Api.Client.RustGrpcChannel.Interop;

/// <summary>A <see cref="Memory{T}" /> over memory of the engine's, for as long as it is lent.</summary>
///
/// The only reason this type exists: a `Span` can be built from a pointer and a `Memory` cannot -
/// it may outlive the frame that made it, so it names its store through a manager that answers
/// `GetSpan` and `Pin`. For memory outside the GC both answers are trivial, which is why the ones
/// below are so short.
///
/// It takes the record the ABI handed over rather than a pointer and a length, so the two cannot
/// be paired with anything but each other, and disposal is what says the engine has it back.
internal sealed unsafe class UnmanagedMemoryManager : MemoryManager<byte>
{
  private readonly byte* start_;
  private readonly int length_;

  private bool returned_;

  internal UnmanagedMemoryManager(in ak_buffer buffer)
    : this(buffer.ptr,
           buffer.len)
  {
  }

  internal UnmanagedMemoryManager(in ak_bytes payload)
    : this(payload.ptr,
           payload.len)
  {
  }

  private UnmanagedMemoryManager(byte*   start,
                                 UIntPtr length)
  {
    length_ = Length(length);
    if (start == null && length_ != 0)
    {
      throw new ArgumentException($"{length_} bytes at no address",
                                  nameof(start));
    }

    start_ = start;
  }

  public override Span<byte> GetSpan()
    => Span(Lent,
            length_);

  public override MemoryHandle Pin(int elementIndex = 0)
  {
    if (elementIndex < 0 || elementIndex > length_)
    {
      throw new ArgumentOutOfRangeException(nameof(elementIndex),
                                            elementIndex,
                                            $"outside the {length_} bytes this names");
    }

    return new MemoryHandle(Lent + elementIndex);
  }

  public override void Unpin()
  {
  }

  /// <summary>The one place an ABI length is narrowed to what .NET counts with.</summary>
  ///
  /// Checked, because the ABI counts in `size_t` and a span counts in `int`. A length past what a
  /// span addresses is not a length this side can hold, and truncating it lands either on a
  /// negative one - which `Span` refuses with nothing to say about why - or on a smaller positive
  /// one, which is a short read of a payload nobody notices is short.
  internal static int Length(UIntPtr length)
    => checked((int)length);

  internal static Span<byte> Span(byte*   start,
                                  UIntPtr length)
    => Span(start,
            Length(length));

  internal static Span<byte> Span(byte* start,
                                  int   length)
    => new(start,
           length);

  /// <summary>Ends every view this handed out.</summary>
  /// <remarks>Nothing is freed here - the memory is the engine's - but a `Memory` a serializer
  /// kept would otherwise still read an address the engine has taken back and reused. Disposal is
  /// the moment that stops being true, so it is the moment this stops answering.</remarks>
  protected override void Dispose(bool disposing)
    => returned_ = true;

  private byte* Lent
    => returned_
         ? throw new ObjectDisposedException(nameof(UnmanagedMemoryManager),
                                            "this names memory the engine has taken back")
         : start_;
}
