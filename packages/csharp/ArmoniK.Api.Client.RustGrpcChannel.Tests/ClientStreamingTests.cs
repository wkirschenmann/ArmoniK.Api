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


using System.Threading.Tasks;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Client streaming: several messages, then one reply.</summary>
[TestFixture]
public class ClientStreamingTests : RuntimeLeaseFixture
{
  private static readonly string[] Sent =
  {
    "one",
    "two",
    "three",
  };

  private EchoServerProcess? server_;
  private string             endpoint_ = string.Empty;

  [OneTimeSetUp]
  public void StartServer()
  {
    server_   = EchoServerProcess.Start();
    endpoint_ = server_.Endpoint;
  }

  [OneTimeTearDown]
  public void StopServer()
    => server_?.Dispose();

  private Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());

  [Test]
  public async Task EveryMessageReachesTheServerInOrderAndTheReplyNamesThemAll()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Collect();

    foreach (var text in Sent)
    {
      await call.RequestStream.WriteAsync(new EchoRequest
                                          {
                                            Text = text,
                                          })
                .ConfigureAwait(false);
    }

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);

    var reply = await call.ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("3:one,two,three"),
                "the server read every message, in order, and answered once");
  }

  [Test]
  public async Task AStreamThatSendsNothingStillReachesItsReply()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Collect();

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);

    var reply = await call.ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("0:"));
  }

  [Test]
  public void AWriteAfterTheStreamIsClosedIsRefused()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Collect();

    call.RequestStream.CompleteAsync()
        .GetAwaiter()
        .GetResult();

    Assert.That(() => call.RequestStream.WriteAsync(new EchoRequest
                                                    {
                                                      Text = "late",
                                                    }),
                Throws.InstanceOf<System.InvalidOperationException>(),
                "the engine would refuse it anyway, but the writer says which rule was broken");
  }
}
