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

internal sealed class ReceivedMessage : DeserializationContext
{
  private readonly IntPtr start_;
  private readonly int length_;

  internal ReceivedMessage(in NativeMethods.AkBytes payload)
  {
    start_  = payload.Ptr;
    length_ = UnmanagedMemoryManager.Length(payload.Len);
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
    => new(new UnmanagedMemoryManager(start_,
                                      length_).Memory);
}
