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
using System.Linq;
using System.Threading.Tasks;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The test server started with <c>--plain-say</c>, as the benchmark starts it.</summary>
[TestFixture]
public class PlainSayTests : RuntimeFixture
{
  private EchoServerProcess? server_;

  [OneTimeSetUp]
  public void StartThePlainServer()
    => server_ = EchoServerProcess.Start(plainSay: true);

  [OneTimeTearDown]
  public void StopThePlainServer()
    => server_?.Dispose();

  [Test]
  public async Task SayAnswersWithNothingButTheReplyAndXAnswered()
  {
    await using var channel = Runtime.Channel(server_!.Endpoint);
    using var call = new Echo.EchoClient(channel.CreateCallInvoker()).SayAsync(new EchoRequest
                                                                                {
                                                                                  Text = "x",
                                                                                });
    var reply = await call.ResponseAsync.ConfigureAwait(false);
    var head  = await call.ResponseHeadersAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("x"));
    Assert.That(head.Select(entry => entry.Key)
                    .Where(key => key.StartsWith("x-",
                                                 StringComparison.Ordinal)),
                Is.EqualTo(new[]
                           {
                             "x-answered",
                           }));
    Assert.That(call.GetTrailers()
                    .Select(entry => entry.Key)
                    .Where(key => key.StartsWith("x-",
                                                 StringComparison.Ordinal)),
                Is.Empty);
  }
}
