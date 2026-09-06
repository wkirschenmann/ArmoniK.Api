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


using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Server streaming: one request, then a message per read.</summary>
[TestFixture]
public class ServerStreamingTests : RuntimeLeaseFixture
{
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

  private static async Task<List<string>> ReadAll(IAsyncStreamReader<EchoReply> replies)
  {
    var seen = new List<string>();
    while (await replies.MoveNext(CancellationToken.None)
                        .ConfigureAwait(false))
    {
      seen.Add(replies.Current.Text);
    }

    return seen;
  }

  [Test]
  public async Task EveryMessageComesBackInOrderAndTheStreamThenEnds()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Fan(new EchoRequest
           {
             Text = "one,two,three",
           });

    var seen = await ReadAll(call.ResponseStream)
                 .ConfigureAwait(false);

    Assert.That(seen,
                Is.EqualTo(new[]
                           {
                             "one",
                             "two",
                             "three",
                           }),
                "one message per read, in order");

    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  /// <summary>A terminal answers the same thing however often it is asked.</summary>
  [Test]
  public async Task AReadPastTheEndKeepsAnsweringFalse()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Fan(new EchoRequest
           {
             Text = "only",
           });

    await ReadAll(call.ResponseStream)
      .ConfigureAwait(false);

    Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                          .ConfigureAwait(false),
                Is.False,
                "the stream ended, and it says so every time");
  }

  [Test]
  public async Task AStreamThatAnswersNothingEndsCleanly()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Fan(new EchoRequest());

    var seen = await ReadAll(call.ResponseStream)
                 .ConfigureAwait(false);

    Assert.That(seen,
                Is.Empty);
    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  /// <summary>A caller that stops reading still lets the channel go.</summary>
  /// <remarks>The drain is what collects the rest: nothing else would, and the channel's
  /// disposal waits for every call to have settled with the engine owed nothing.</remarks>
  [Test]
  public async Task AStreamAbandonedHalfwayStillLetsTheChannelBeDisposed()
  {
    using (var channel = NativeRuntimeFactory.Channel(endpoint_))
    {
      using var call = Client(channel)
        .Fan(new EchoRequest
             {
               Text = "one,two,three",
             });

      Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                            .ConfigureAwait(false),
                  Is.True);
      Assert.That(call.ResponseStream.Current.Text,
                  Is.EqualTo("one"));
    }

    Assert.Pass("the channel disposed without waiting on a stream nobody read to the end");
  }

  [Test]
  public void ACancelledTokenEndsTheReadAsCancelled()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    using var call = Client(channel)
      .Fan(new EchoRequest
           {
             Text = "one,two,three",
           });

    using var cancelled = new CancellationTokenSource();
    cancelled.Cancel();

    var refused = Assert.ThrowsAsync<RpcException>(() => call.ResponseStream.MoveNext(cancelled.Token));

    Assert.That(refused!.Status.StatusCode,
                Is.EqualTo(StatusCode.Cancelled),
                "a cancelled read leaves as the binding's one public rule");
  }
}
