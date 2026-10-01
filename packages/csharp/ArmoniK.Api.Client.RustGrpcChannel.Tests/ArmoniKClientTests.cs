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
using System.Diagnostics;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Events;
using ArmoniK.Api.gRPC.V1.Results;

using Grpc.Core;

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

  /// <summary><c>WaitForResultsAsync</c> gives up on a server that does not answer.</summary>
  [Test]
  public async Task WaitForResultsGivesUpOnAServerThatDoesNotAnswer()
  {
    await using var channel = Runtime.Channel(ClosedPort.Endpoint());

    Assert.That(await WaitForResultsEnd(channel,
                                        "session-id")
                  .ConfigureAwait(false),
                Is.InstanceOf<RpcException>());
  }

  /// <summary><c>WaitForResultsAsync</c> gives up on a subscription the server refuses.</summary>
  /// <remarks>Refused before any event, the response is Trailers-Only, and its headers answer as
  /// grpc-dotnet's do: only an event shows the subscription holds.</remarks>
  [Test]
  public async Task WaitForResultsGivesUpOnASubscriptionTheServerRefuses()
  {
    await using var channel = Runtime.Channel(endpoint_);

    // ArmoniK.Api.Mock's Events.RefusedSessionId.
    var thrown = await WaitForResultsEnd(channel,
                                         "refused-session-id")
                   .ConfigureAwait(false);

    Assert.That((thrown as RpcException)?.StatusCode,
                Is.EqualTo(StatusCode.PermissionDenied));
  }

  /// <summary><c>WaitForResultsAsync</c> outlasts a subscription dropped after each event.</summary>
  /// <remarks>Seven drops in a row, each after an event, then the result completes: more drops than
  /// it tolerates with no event between them.</remarks>
  [Test]
  public async Task WaitForResultsOutlastsDropsBetweenEvents()
  {
    await using var channel = Runtime.Channel(endpoint_);

    // ArmoniK.Api.Mock's Events.DroppedSessionId.
    Assert.That(await WaitForResultsEnd(channel,
                                        "dropped-session-id",
                                        "dropped-at-once")
                  .ConfigureAwait(false),
                Is.Null);
  }

  /// <summary>Each new subscription waits for its bound, which the multiplier grows up to the
  /// maximum, and the third refusal ends the wait.</summary>
  /// <remarks>With no jitter the delays are the bounds: 100 then 300 ms, 0.4 s in all. An uncapped
  /// bound would wait 1.1 s, and six attempts 1.3 s.</remarks>
  [Test]
  public async Task WaitForResultsWaitsItsBackoffBetweenSubscriptions()
  {
    await using var channel = await Warmed()
                                .ConfigureAwait(false);

    var watch = Stopwatch.StartNew();
    var thrown = await WaitForResultsEnd(channel,
                                         "refused-session-id",
                                         retry: Backoff(3,
                                                        10,
                                                        300,
                                                        0))
                   .ConfigureAwait(false);
    watch.Stop();

    Assert.That((thrown as RpcException)?.StatusCode,
                Is.EqualTo(StatusCode.PermissionDenied));
    // A little under the bounds' sum, because a timer may fire just early.
    Assert.That(watch.Elapsed,
                Is.GreaterThanOrEqualTo(TimeSpan.FromMilliseconds(360)));
    Assert.That(watch.Elapsed,
                Is.LessThan(TimeSpan.FromMilliseconds(900)),
                "the bound is capped and the attempts counted");
  }

  /// <summary>With full jitter each delay is drawn up to its bound.</summary>
  /// <remarks>Eleven delays whose bounds - 100, 400, then 600 ms - add up to 5.9 s, and whose draws
  /// add up to 2.95 s on average. Even with 300 ms spent on timers and subscriptions, the wait
  /// reaches 5.6 s less than once in a million runs.</remarks>
  [Test]
  public async Task WaitForResultsDrawsItsDelaysAtRandom()
  {
    await using var channel = await Warmed()
                                .ConfigureAwait(false);

    var watch = Stopwatch.StartNew();
    var thrown = await WaitForResultsEnd(channel,
                                         "refused-session-id",
                                         retry: Backoff(12,
                                                        4,
                                                        600,
                                                        1))
                   .ConfigureAwait(false);
    watch.Stop();

    Assert.That((thrown as RpcException)?.StatusCode,
                Is.EqualTo(StatusCode.PermissionDenied));
    Assert.That(watch.Elapsed,
                Is.LessThan(TimeSpan.FromMilliseconds(5600)));
  }

  /// <summary>An event resets the bound along with the count.</summary>
  /// <remarks>Seven drops, each after an event, so each waits the first bound: 0.7 s, where a bound
  /// that kept growing would wait 1.9 s.</remarks>
  [Test]
  public async Task WaitForResultsStartsItsBackoffAgainAfterAnEvent()
  {
    await using var channel = await Warmed()
                                .ConfigureAwait(false);

    var watch = Stopwatch.StartNew();
    Assert.That(await WaitForResultsEnd(channel,
                                        "dropped-session-id",
                                        "dropped-with-backoff",
                                        Backoff(3,
                                                10,
                                                300,
                                                0))
                  .ConfigureAwait(false),
                Is.Null);
    watch.Stop();

    // Well under the bounds' sum, because each of seven timers may fire just early.
    Assert.That(watch.Elapsed,
                Is.GreaterThanOrEqualTo(TimeSpan.FromMilliseconds(500)));
    Assert.That(watch.Elapsed,
                Is.LessThan(TimeSpan.FromMilliseconds(1500)),
                "the bound started again");
  }

  /// <summary>The first bound is capped as well, as gRFC A6 caps every bound.</summary>
  /// <remarks>One delay, of 100 ms rather than the 1 s InitialBackOff names.</remarks>
  [Test]
  public async Task WaitForResultsCapsItsFirstBound()
  {
    await using var channel = await Warmed()
                                .ConfigureAwait(false);

    var watch = Stopwatch.StartNew();
    var thrown = await WaitForResultsEnd(channel,
                                         "refused-session-id",
                                         retry: new SubscriptionRetry
                                                {
                                                  MaxAttempts    = 2,
                                                  InitialBackOff = TimeSpan.FromSeconds(1),
                                                  MaxBackOff     = TimeSpan.FromMilliseconds(100),
                                                  Jitter         = 0,
                                                })
                   .ConfigureAwait(false);
    watch.Stop();

    Assert.That((thrown as RpcException)?.StatusCode,
                Is.EqualTo(StatusCode.PermissionDenied));
    Assert.That(watch.Elapsed,
                Is.GreaterThanOrEqualTo(TimeSpan.FromMilliseconds(80)));
    Assert.That(watch.Elapsed,
                Is.LessThan(TimeSpan.FromMilliseconds(700)));
  }

  /// <summary>A retry whose values are outside what they admit is refused before any subscription.</summary>
  /// <remarks>Each case breaks one check alone, and would otherwise end another way: a refusal after
  /// the first attempt, a Task.Delay that refuses its own argument, or no delay at all.</remarks>
  [TestCaseSource(nameof(OutOfRange))]
  public async Task WaitForResultsRefusesARetryOutsideWhatItAdmits(SubscriptionRetry retry)
  {
    await using var channel = Runtime.Channel(endpoint_);

    Assert.That(await WaitForResultsEnd(channel,
                                        "refused-session-id",
                                        retry: retry)
                  .ConfigureAwait(false),
                Is.InstanceOf<ArgumentOutOfRangeException>()
                  .And.Property(nameof(ArgumentException.ParamName))
                  .EqualTo("retry"));
  }

  [Test]
  public async Task WaitForResultsRefusesANullRetry()
  {
    await using var channel = Runtime.Channel(endpoint_);

    var thrown = Assert.ThrowsAsync<ArgumentNullException>(() => new gRPC.V1.Events.Events.EventsClient(channel).WaitForResultsAsync("refused-session-id",
                                                                                                                                       new[]
                                                                                                                                       {
                                                                                                                                         "result-id",
                                                                                                                                       },
                                                                                                                                       null!));
    Assert.That(thrown!.ParamName,
                Is.EqualTo("retry"));
  }

  private static IEnumerable<TestCaseData> OutOfRange()
  {
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    MaxAttempts = 0,
                                  }).SetArgDisplayNames("MaxAttempts 0");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    InitialBackOff = TimeSpan.FromMilliseconds(-1),
                                    Jitter         = 0,
                                  }).SetArgDisplayNames("InitialBackOff -1 ms");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    InitialBackOff = TimeSpan.FromMilliseconds(int.MaxValue) + TimeSpan.FromMilliseconds(1),
                                    MaxBackOff     = TimeSpan.FromMilliseconds(int.MaxValue),
                                    Jitter         = 0,
                                  }).SetArgDisplayNames("InitialBackOff past Task.Delay");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    MaxBackOff = TimeSpan.FromSeconds(-1),
                                  }).SetArgDisplayNames("MaxBackOff -1 s");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    InitialBackOff    = TimeSpan.FromMilliseconds(1),
                                    MaxBackOff        = TimeSpan.MaxValue,
                                    BackoffMultiplier = 1e12,
                                    Jitter            = 0,
                                  }).SetArgDisplayNames("MaxBackOff past Task.Delay");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    BackoffMultiplier = double.NaN,
                                  }).SetArgDisplayNames("BackoffMultiplier NaN");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    BackoffMultiplier = 0,
                                  }).SetArgDisplayNames("BackoffMultiplier 0");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    BackoffMultiplier = -1,
                                  }).SetArgDisplayNames("BackoffMultiplier -1");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    BackoffMultiplier = double.PositiveInfinity,
                                  }).SetArgDisplayNames("BackoffMultiplier infinite");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    Jitter = 1.5,
                                  }).SetArgDisplayNames("Jitter 1.5");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    Jitter = -0.5,
                                  }).SetArgDisplayNames("Jitter -0.5");
    yield return new TestCaseData(new SubscriptionRetry
                                  {
                                    Jitter = double.NaN,
                                  }).SetArgDisplayNames("Jitter NaN");
  }

  /// <summary>A first bound of 100 ms.</summary>
  private static SubscriptionRetry Backoff(int    maxAttempts,
                                           double multiplier,
                                           int    maxMilliseconds,
                                           double jitter)
    => new()
       {
         MaxAttempts       = maxAttempts,
         InitialBackOff    = TimeSpan.FromMilliseconds(100),
         BackoffMultiplier = multiplier,
         MaxBackOff        = TimeSpan.FromMilliseconds(maxMilliseconds),
         Jitter            = jitter,
       };

  /// <summary>A channel that has made its first call.</summary>
  /// <remarks>A channel connects on its first call, and on .NET Framework that connection alone can
  /// take most of a timed test's margin, so a stopwatch started on this one counts the backoff and
  /// not the connection. The call is a refusal the server answers at once, under a result no timed
  /// test names, so it changes nothing the test after it reads.</remarks>
  private async Task<NativeChannel> Warmed()
  {
    var channel = Runtime.Channel(endpoint_);
    var thrown = await WaitForResultsEnd(channel,
                                         "refused-session-id",
                                         "warm-up",
                                         new SubscriptionRetry
                                         {
                                           MaxAttempts = 1,
                                         })
                   .ConfigureAwait(false);
    Assert.That((thrown as RpcException)?.StatusCode,
                Is.EqualTo(StatusCode.PermissionDenied),
                "the warm-up call is answered");
    return channel;
  }

  /// <summary>How <c>WaitForResultsAsync</c> ends, which it must within 30 s: the exception it
  /// fails with, or null when it completes.</summary>
  /// <remarks>With no <paramref name="retry" />, through the overload that takes none.</remarks>
  private static async Task<Exception?> WaitForResultsEnd(NativeChannel      channel,
                                                          string             sessionId,
                                                          string             resultId = "result-id",
                                                          SubscriptionRetry? retry    = null)
  {
    var client = new gRPC.V1.Events.Events.EventsClient(channel);
    var results = new[]
                  {
                    resultId,
                  };
    var waiting = retry is null
                    ? client.WaitForResultsAsync(sessionId,
                                                 results,
                                                 100,
                                                 1,
                                                 CancellationToken.None)
                    : client.WaitForResultsAsync(sessionId,
                                                 results,
                                                 retry,
                                                 100,
                                                 1,
                                                 CancellationToken.None);
    using var bound = new CancellationTokenSource();
    var settled = await Task.WhenAny(waiting,
                                     Task.Delay(TimeSpan.FromSeconds(30),
                                                bound.Token))
                            .ConfigureAwait(false);
    bound.Cancel();

    Assert.That(settled,
                Is.SameAs(waiting),
                "it retries for good");
    return waiting.Exception?.InnerException;
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
