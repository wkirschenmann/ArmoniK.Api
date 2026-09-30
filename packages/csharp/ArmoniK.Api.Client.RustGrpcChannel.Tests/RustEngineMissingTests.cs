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
  [TestCase("http://user:secret@example.test:5000/some/path?q=1",
            "http://example.test:5000",
            TestName = "{m}(userinfo, path and query)")]
  [TestCase("https://user:secret@example.test/",
            "https://example.test",
            TestName = "{m}(no port written)")]
  [TestCase("http://example.test:5000",
            "http://example.test:5000",
            TestName = "{m}(nothing secret)")]
  [TestCase("localhost:5000",
            "localhost:5000",
            TestName = "{m}(no scheme)")]
  [TestCase("localhost:80",
            "localhost:80",
            TestName = "{m}(a port as written)")]
  [TestCase("user:secret@example.test:5000",
            "example.test:5000",
            TestName = "{m}(no scheme, userinfo)")]
  [TestCase("user:12/ab@example.test:5000",
            "example.test:5000",
            TestName = "{m}(no scheme, a slash in the password)")]
  [TestCase("http://user:12/ab@example.test:5000",
            "http://example.test:5000",
            TestName = "{m}(a slash in the password)")]
  [TestCase("user:pa://ss@example.test:5000",
            "example.test:5000",
            TestName = "{m}(a :// in the password)")]
  [TestCase("user:p@ss@example.test:5000",
            "example.test:5000",
            TestName = "{m}(an @ in the password)")]
  [TestCase("http://example.test:5000?token=abc",
            "http://example.test:5000",
            TestName = "{m}(a query)")]
  [TestCase("http://example.test:5000#token",
            "http://example.test:5000",
            TestName = "{m}(a fragment)")]
  [TestCase("http://example.test:5000\\token",
            "http://example.test:5000",
            TestName = "{m}(a backslash)")]
  [TestCase("http://[::1]:5000",
            "http://[::1]:5000",
            TestName = "{m}(an IPv6 host)")]
  [TestCase("http://[::1]",
            "http://[::1]",
            TestName = "{m}(an IPv6 host and no port)")]
  [TestCase("example.test:hunter2",
            "an endpoint that is not a URI",
            TestName = "{m}(a port that is not digits)")]
  [TestCase("dns:///example.test:5000",
            "dns://",
            TestName = "{m}(a scheme and no host)")]
  [TestCase("example.test:5000\n",
            "an endpoint that is not a URI",
            TestName = "{m}(a trailing newline)")]
  [TestCase("http://example.test:5000\n",
            "an endpoint that is not a URI",
            TestName = "{m}(a trailing newline after a scheme)")]
  [TestCase("example.test\u0001:5000",
            "an endpoint that is not a URI",
            TestName = "{m}(a control character)")]
  [TestCase("not an endpoint at all",
            "an endpoint that is not a URI",
            TestName = "{m}(not a URI)")]
  public void AnEndpointInAMessageKeepsItsSchemeHostAndPortAndNothingElse(string endpoint,
                                                                          string shown)
    => Assert.That(NativeChannel.Safely(endpoint),
                   Is.EqualTo(shown));

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
