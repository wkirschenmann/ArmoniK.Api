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
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Events;
using ArmoniK.Api.gRPC.V1.Results;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The only tests that drive ArmoniK's own generated stubs over this invoker.</summary>
/// <remarks>They start their own ArmoniK.Api.Mock, the way the echo tests start their own server,
/// so they run wherever the suite runs rather than only where something else has already put a
/// server on a port and named it in the environment.</remarks>
[TestFixture]
public class ArmoniKClientTests : RuntimeFixture
{
  private MockServerProcess? server_;
  private string             endpoint_ = string.Empty;

  [OneTimeSetUp]
  public void StartServer()
  {
    server_   = MockServerProcess.Start();
    endpoint_ = server_.Endpoint;
  }

  [OneTimeTearDown]
  public void StopServer()
    => server_?.Dispose();

  /// <summary>The pattern ArmoniK's own <c>WaitForResultsAsync</c> uses, over this invoker.</summary>
  /// <remarks><c>EventsClientExt.WaitForResultsAsync</c> awaits <c>ResponseHeadersAsync</c> on the
  /// event stream and only then reads it. Driven here against the real service rather than the echo
  /// server, because it is that extension's shape and not a contrivance that has to work.</remarks>
  [Test]
  public async Task TheEventStreamAnswersItsHeadBeforeItIsRead()
  {
    await using var channel = Runtime.Channel(endpoint_);

    using var events = new gRPC.V1.Events.Events.EventsClient(channel).GetEvents(new EventSubscriptionRequest
                                                                                 {
                                                                                   SessionId = "session-id",
                                                                                 });

    var head = events.ResponseHeadersAsync;
    var settled = await Task.WhenAny(head,
                                     Task.Delay(TimeSpan.FromSeconds(10)))
                            .ConfigureAwait(false);

    Assert.That(settled,
                Is.SameAs(head),
                "WaitForResultsAsync awaits this before its first MoveNext");

    Assert.That(await events.ResponseStream.MoveNext(CancellationToken.None)
                            .ConfigureAwait(false),
                Is.True,
                "and the stream still reads afterwards");
  }

  [Test]
  public async Task AGeneratedArmoniKStubAnswersOverThisInvoker()
  {
    await using var channel = Runtime.Channel(endpoint_);

    var results = new Results.ResultsClient(channel);

    Assert.That(() => results.GetServiceConfiguration(new Empty()),
                Throws.Nothing);
  }

  [Test]
  public async Task TheSameStubAnswersAsynchronously()
  {
    await using var channel = Runtime.Channel(endpoint_);

    var configuration = await new Results.ResultsClient(channel).GetServiceConfigurationAsync(new Empty())
                                                                .ConfigureAwait(false);

    Assert.That(configuration.DataChunkMaxSize,
                Is.GreaterThan(0),
                "the answer came from the service and not from a default");
  }
}
