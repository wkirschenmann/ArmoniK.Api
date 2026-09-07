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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class RustEngineMissingTests
{
  /// <summary>An endpoint reaches a message without whatever it carried before its host.</summary>
  /// <remarks>
  ///   The engine refuses an endpoint that carries `user:password@`, so that refusal is precisely
  ///   the one whose message a caller reads - and the endpoint as given would put the password in
  ///   their log. Asserted on what must be absent as well as on what must be there: a redaction
  ///   that keeps the host is worth nothing if it keeps the userinfo too.
  /// </remarks>
  [Test]
  public void AnEndpointInAMessageKeepsItsSchemeHostAndPortAndNothingElse()
  {
    Assert.Multiple(() =>
                    {
                      Assert.That(NativeChannel.Safely("http://user:secret@example.test:5000/some/path?q=1"),
                                  Is.EqualTo("http://example.test:5000"));
                      Assert.That(NativeChannel.Safely("https://user:secret@example.test/"),
                                  Is.EqualTo("https://example.test"),
                                  "a default port is not spelled out");
                      Assert.That(NativeChannel.Safely("http://example.test:5000"),
                                  Is.EqualTo("http://example.test:5000"),
                                  "an endpoint carrying nothing secret is unchanged");
                      Assert.That(NativeChannel.Safely("user:secret@example.test:5000"),
                                  Does.Not.Contain("secret"),
                                  "a string no Uri parses is named rather than echoed");
                    });
  }

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
