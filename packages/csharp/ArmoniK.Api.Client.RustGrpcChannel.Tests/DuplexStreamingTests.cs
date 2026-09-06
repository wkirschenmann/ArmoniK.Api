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

/// <summary>Bidi streaming: sends and answers interleaved on one call.</summary>
[TestFixture]
public class DuplexStreamingTests : RuntimeLeaseFixture
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

  /// <summary>Each answer read before the next message is sent.</summary>
  /// <remarks>Interleaved and not batched, because that is what the cardinality is for and what
  /// a single send window and one held event per call have to allow.</remarks>
  [Test]
  public async Task EachMessageIsAnsweredBeforeTheNextIsSent()
  {
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
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
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
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
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
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
}
