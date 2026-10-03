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
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Bidi streaming: sends and answers interleaved on one call.</summary>
[TestFixture]
public class DuplexStreamingTests : EchoServerFixture
{
  private static readonly string[] Sent =
  {
    "one",
    "two",
    "three",
  };

  /// <summary>The headers resolve with no read, on the cardinality that reads and writes at once.</summary>
  [Test]
  public async Task TheResponseHeadArrivesWithoutAnyRead()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .HeadThenChat();

    var head = call.ResponseHeadersAsync;
    var settled = await Task.WhenAny(head,
                                     Task.Delay(TimeSpan.FromSeconds(10)))
                            .ConfigureAwait(false);

    Assert.That(settled,
                Is.SameAs(head),
                "the headers resolved with no read to carry them, and with nothing sent yet");
    Assert.That((await head.ConfigureAwait(false)).GetValue("x-answered"),
                Is.EqualTo("yes"));
  }

  /// <summary>Each answer read before the next message is sent.</summary>
  /// <remarks>Interleaved and not batched, because that is what the cardinality is for and what
  /// a single send window has to allow.</remarks>
  [Test]
  public async Task EachMessageIsAnsweredBeforeTheNextIsSent()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Chat();

    foreach (var text in Sent)
    {
      await call.RequestStream.WriteAsync(new EchoRequest
                                          {
                                            Text = text,
                                          })
                .ConfigureAwait(false);

      Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                            .ConfigureAwait(false),
                  Is.True,
                  $"an answer to `{text}`");
      Assert.That(call.ResponseStream.Current.Text,
                  Is.EqualTo(text));
    }

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);

    Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                          .ConfigureAwait(false),
                Is.False,
                "the half-close ends the answers too");
    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  /// <summary>Everything sent first, then everything read.</summary>
  [Test]
  public async Task EverythingSentBeforeAnythingIsReadStillComesBackInOrder()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Chat();

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

    var seen = new List<string>();
    while (await call.ResponseStream.MoveNext(CancellationToken.None)
                     .ConfigureAwait(false))
    {
      seen.Add(call.ResponseStream.Current.Text);
    }

    Assert.That(seen,
                Is.EqualTo(Sent));
  }

  /// <summary>A call that sends nothing ends as soon as it says so.</summary>
  [Test]
  public async Task AConversationWithNothingToSayEndsCleanly()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Chat();

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);

    Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                          .ConfigureAwait(false),
                Is.False);
    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  /// <summary>A read cancelled while the response head has not arrived ends as cancelled.</summary>
  /// <remarks>Nothing is sent, so the server writes no head and the read waits in the prologue,
  /// where no published read carries the token: the token has to wake that wait itself.</remarks>
  [Test]
  public async Task AReadCancelledBeforeTheHeadArrivesEndsAsCancelled()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Chat();

    using var cancelled = new CancellationTokenSource();
    var read = call.ResponseStream.MoveNext(cancelled.Token);
    cancelled.CancelAfter(TimeSpan.FromMilliseconds(100));

    var ended = await Task.WhenAny(read,
                                   Task.Delay(TimeSpan.FromSeconds(10)))
                          .ConfigureAwait(false);

    Assert.That(ended,
                Is.SameAs(read),
                "the read outlived its token");
    var refused = Assert.ThrowsAsync<RpcException>(async () => await read.ConfigureAwait(false));
    Assert.That(refused!.Status.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }
}
