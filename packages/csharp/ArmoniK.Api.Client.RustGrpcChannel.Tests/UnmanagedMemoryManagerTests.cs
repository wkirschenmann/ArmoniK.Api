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

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class UnmanagedMemoryManagerTests
{
  [Test]
  public void EveryLengthASpanCanHoldIsHeld()
    => Assert.Multiple(() =>
                       {
                         Assert.That(UnmanagedMemoryManager.Length(UIntPtr.Zero),
                                     Is.EqualTo(0));
                         Assert.That(UnmanagedMemoryManager.Length((UIntPtr)int.MaxValue),
                                     Is.EqualTo(int.MaxValue));
                       });

  /// <summary>One past what a span addresses, which is where a narrowing truncates.</summary>
  [Test]
  public void ALengthPastWhatASpanHoldsIsRefused()
    => Assert.Throws<OverflowException>(() => UnmanagedMemoryManager.Length((UIntPtr)((ulong)int.MaxValue + 1)));
}
