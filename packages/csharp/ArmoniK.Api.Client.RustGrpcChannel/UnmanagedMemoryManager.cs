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

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal sealed class UnmanagedMemoryManager : MemoryManager<byte>
{
  private readonly IntPtr start_;
  private readonly int length_;

  internal UnmanagedMemoryManager(IntPtr start,
                                  int length)
  {
    start_  = start;
    length_ = length;
  }

  public override Span<byte> GetSpan()
    => Span(start_,
            length_);

  public override unsafe MemoryHandle Pin(int elementIndex = 0)
    => new((byte*)start_ + elementIndex);

  public override void Unpin()
  {
  }

  /// <summary>The one place an ABI length is narrowed to what .NET counts with.</summary>
  internal static int Length(UIntPtr length)
    => (int)length;

  internal static Span<byte> Span(IntPtr start,
                                  UIntPtr length)
    => Span(start,
            Length(length));

  internal static unsafe Span<byte> Span(IntPtr start,
                                         int length)
    => new((void*)start,
           length);

  protected override void Dispose(bool disposing)
  {
  }
}
