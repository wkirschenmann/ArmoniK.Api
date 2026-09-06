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
using System.Runtime.InteropServices;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class RustEngineMissingTests
{
  /// <summary>The message is the whole point of this type, so it is asserted rather than assumed.</summary>
  [Test]
  public void TheMessageNamesWhatSomeoneWouldGoAndCheck()
  {
    var raised = RustEngineMissingException.For(new DllNotFoundException("under test"));

    Assert.Multiple(() =>
                    {
                      Assert.That(raised.Message,
                                  Does.Contain(NativeMethods.Library),
                                  "which library");
                      Assert.That(raised.Message,
                                  Does.Contain(RuntimeInformation.ProcessArchitecture.ToString()),
                                  "which architecture, since 64-bit alone does not tell x64 from Arm64");
                      Assert.That(raised.Message,
                                  Does.Contain(AppDomain.CurrentDomain.BaseDirectory),
                                  "where to go and look");
                      Assert.That(raised.InnerException,
                                  Is.TypeOf<DllNotFoundException>(),
                                  "and what the runtime actually said");
                    });
  }
}
