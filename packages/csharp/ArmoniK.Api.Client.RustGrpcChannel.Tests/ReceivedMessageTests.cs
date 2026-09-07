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

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;
using ArmoniK.Api.Client.RustGrpcChannel.Calls;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class ReceivedMessageTests
{
  private static void Over(byte[] bytes,
                           Action<ReceivedMessage> read)
  {
    var pin = GCHandle.Alloc(bytes,
                             GCHandleType.Pinned);
    try
    {
      read(new ReceivedMessage(new NativeMethods.AkBytes
                               {
                                 Ptr = bytes.Length == 0
                                         ? IntPtr.Zero
                                         : pin.AddrOfPinnedObject(),
                                 Len = (UIntPtr)bytes.Length,
                               }));
    }
    finally
    {
      pin.Free();
    }
  }

  [Test]
  public void BothViewsShowTheBytesTheEngineDelivered()
    => Over(new byte[]
            {
              1, 2, 3, 255,
            },
            message => Assert.Multiple(() =>
                                       {
                                         Assert.That(message.PayloadLength,
                                                     Is.EqualTo(4));
                                         Assert.That(message.PayloadAsNewBuffer(),
                                                     Is.EqualTo(new byte[]
                                                                {
                                                                  1, 2, 3, 255,
                                                                }));
                                         Assert.That(message.PayloadAsReadOnlySequence()
                                                            .ToArray(),
                                                     Is.EqualTo(new byte[]
                                                                {
                                                                  1, 2, 3, 255,
                                                                }));
                                       }));

  /// <summary>Asked twice, because a deserializer may and the second answer is the cached view.</summary>
  [Test]
  public void TheSequenceReadsTheSameEveryTimeItIsAsked()
    => Over(new byte[]
            {
              7, 8,
            },
            message => Assert.That(message.PayloadAsReadOnlySequence()
                                          .ToArray(),
                                   Is.EqualTo(message.PayloadAsReadOnlySequence()
                                                     .ToArray())));

  /// <summary>An empty message is a message: the engine owns its payload and expects it back.</summary>
  [Test]
  public void AnEmptyPayloadIsEmptyAndAllocatesNothing()
    => Over(Array.Empty<byte>(),
            message => Assert.Multiple(() =>
                                       {
                                         Assert.That(message.PayloadLength,
                                                     Is.EqualTo(0));
                                         Assert.That(message.PayloadAsNewBuffer(),
                                                     Is.SameAs(Array.Empty<byte>()));
                                         Assert.That(message.PayloadAsReadOnlySequence()
                                                            .IsEmpty);
                                       }));
}
