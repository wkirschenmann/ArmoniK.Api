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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>A view of one delivered payload, for as long as the deserializer is running.</summary>
///
/// The bytes belong to the engine until `ak_event_consumed`, which the reader calls once the
/// deserializer has returned. Nothing here copies them, so what a deserializer keeps a reference
/// to it must have copied itself - which is what protobuf's parser does with every field it reads.
internal sealed class ReceivedMessage : DeserializationContext
{
  private readonly IntPtr start_;
  private readonly int length_;

  private UnmanagedMemoryManager? view_;

  internal ReceivedMessage(in NativeMethods.AkBytes payload)
  {
    start_  = payload.Ptr;
    length_ = UnmanagedMemoryManager.Length(payload.Len);
  }

  public override int PayloadLength
    => length_;

  public override byte[] PayloadAsNewBuffer()
  {
    if (length_ == 0)
    {
      return Array.Empty<byte>();
    }

    var bytes = new byte[length_];
    Marshal.Copy(start_,
                 bytes,
                 0,
                 length_);
    return bytes;
  }

  public override ReadOnlySequence<byte> PayloadAsReadOnlySequence()
  {
    if (length_ == 0)
    {
      return ReadOnlySequence<byte>.Empty;
    }

    // Kept, because a deserializer may ask more than once and the manager is the only thing this
    // allocates. It is not disposed: the memory is the engine's, `Dispose` has nothing to do, and
    // `DeserializationContext` gives the caller no moment at which to say so.
    view_ ??= new UnmanagedMemoryManager(start_,
                                         length_);
    return new ReadOnlySequence<byte>(view_.Memory);
  }
}
