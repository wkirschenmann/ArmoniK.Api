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
using System.Collections.Concurrent;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Calls multiplexed on connections that nginx limits.</summary>
/// <remarks>
///   `keepalive_requests` makes nginx send GOAWAY NO_ERROR with the last request a connection
///   takes: the calls it has taken run to their end there, and later ones need another
///   connection. `http2_max_concurrent_streams` makes it refuse a stream past the limit, and close
///   the connection, every stream on it, past max(limit, 100) refusals in its life. Each test
///   runs on the native engine and on grpc-dotnet, which ArmoniK clients use behind the same nginx.
/// </remarks>
[TestFixture]
public class NginxGoAwayTests : EchoServerFixture
{
  private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

  private NginxProcess? nginx_;

  [TearDown]
  public void StopNginx()
  {
    if (nginx_ is null)
    {
      return;
    }

    try
    {
      TestContext.WriteLine(nginx_.ErrorLog());
    }
    finally
    {
      nginx_.Dispose();
      nginx_ = null;
    }
  }

  /// <summary>
  ///   A call after the GOAWAY goes to another connection, and the two the old one took, one
  ///   finished before the other, both end OK.
  /// </summary>
  [TestCase("native")]
  [TestCase("managed")]
  public async Task CallsTakenBeforeTheGoAwayFinishOneAfterTheOther(string transport)
  {
    await using var channel = Open(transport,
                                   "keepalive_requests 2;");
    var client = new Echo.EchoClient(channel.Invoker);

    using var first = Chat(client,
                           "first");
    await Exchange(first,
                   "first 1")
      .ConfigureAwait(false);
    using var second = Chat(client,
                            "second");
    await Exchange(second,
                   "second 1")
      .ConfigureAwait(false);

    using var third = Chat(client,
                           "third");
    await Exchange(third,
                   "third 1")
      .ConfigureAwait(false);
    await Exchange(first,
                   "first 2")
      .ConfigureAwait(false);
    await Exchange(second,
                   "second 2")
      .ConfigureAwait(false);

    await Finish(first)
      .ConfigureAwait(false);
    await Exchange(second,
                   "second 3")
      .ConfigureAwait(false);
    await Exchange(third,
                   "third 2")
      .ConfigureAwait(false);
    await Finish(second)
      .ConfigureAwait(false);
    await Finish(third)
      .ConfigureAwait(false);

    AssertWhereTheCallsWent();
  }

  /// <summary>Cancelling one of the calls the old connection took leaves the other running.</summary>
  [TestCase("native")]
  [TestCase("managed")]
  public async Task CancellingACallTakenBeforeTheGoAwayLeavesTheOtherRunning(string transport)
  {
    await using var channel = Open(transport,
                                   "keepalive_requests 2;");
    var client = new Echo.EchoClient(channel.Invoker);

    using var first = Chat(client,
                           "first");
    await Exchange(first,
                   "first 1")
      .ConfigureAwait(false);
    using var second = Chat(client,
                            "second");
    await Exchange(second,
                   "second 1")
      .ConfigureAwait(false);

    first.Dispose();

    await Exchange(second,
                   "second 2")
      .ConfigureAwait(false);
    using var third = Chat(client,
                           "third");
    await Exchange(third,
                   "third 1")
      .ConfigureAwait(false);
    await Exchange(second,
                   "second 3")
      .ConfigureAwait(false);
    await Finish(second)
      .ConfigureAwait(false);
    await Finish(third)
      .ConfigureAwait(false);

    var calls = nginx_!.Calls();
    Assert.That(calls.Single(call => call.Call == "second")
                     .Connection,
                Is.EqualTo(calls.Single(call => call.Call == "first")
                                .Connection),
                "both calls before the GOAWAY on one connection");
    Assert.That(calls.Single(call => call.Call == "third")
                     .Connection,
                Is.Not.EqualTo(calls.Single(call => call.Call == "first")
                                    .Connection),
                "the call after it on another");
  }

  /// <summary>
  ///   A call past the concurrency limit of a connection whose calls go on takes another
  ///   connection rather than waiting for one of them to end.
  /// </summary>
  /// <remarks>
  ///   Each call answered before the next starts, so that nginx's SETTINGS are in: the calls a new
  ///   connection takes before they arrive are the stress test's subject.
  /// </remarks>
  [TestCase("native")]
  [TestCase("managed")]
  public async Task ACallPastTheConcurrencyLimitStillRuns(string transport)
  {
    await using var channel = Open(transport,
                                   "http2_max_concurrent_streams 2;");
    var client = new Echo.EchoClient(channel.Invoker);

    using var first = Chat(client,
                           "first");
    await Exchange(first,
                   "first 1")
      .ConfigureAwait(false);
    using var second = Chat(client,
                            "second");
    await Exchange(second,
                   "second 1")
      .ConfigureAwait(false);
    using var third = Chat(client,
                           "third");
    await Exchange(third,
                   "third 1")
      .ConfigureAwait(false);

    foreach (var call in new[]
                         {
                           first,
                           second,
                           third,
                         })
    {
      await Finish(call)
        .ConfigureAwait(false);
    }

    // The three overlap, so the calls a connection took are the calls it carried at once.
    var calls = nginx_!.Calls();
    Assert.That(calls,
                Has.Count.EqualTo(3),
                "nginx logged the three calls");
    Assert.That(calls.GroupBy(call => call.Connection)
                     .Select(connection => connection.Count()),
                Is.All.AtMost(2),
                "no connection took more calls than nginx allows it");
  }

  /// <summary>
  ///   Many short calls above the concurrency limit, some cancelled: every call that is not
  ///   cancelled ends OK.
  /// </summary>
  [TestCase("native")]
  [TestCase("managed")]
  public async Task ManyCallsAboveTheConcurrencyLimitAllEnd(string transport)
  {
    const int calls       = 400;
    const int parallelism = 8;
    await using var channel = Open(transport,
                                   "http2_max_concurrent_streams 2;");
    var client = new Echo.EchoClient(channel.Invoker);

    // One deadline for all, so that calls that hang fail the test once rather than each in turn.
    var deadline = DateTime.UtcNow + Patience;
    using var gate = new SemaphoreSlim(parallelism);
    var failures = new ConcurrentQueue<string>();
    var cancelled = 0;
    await Task.WhenAll(Enumerable.Range(0,
                                        calls)
                                 .Select(async index =>
                                         {
                                           await gate.WaitAsync()
                                                     .ConfigureAwait(false);
                                           using var cancellation = new CancellationTokenSource();
                                           try
                                           {
                                             if (index % 5 == 0)
                                             {
                                               cancellation.CancelAfter(index % 3);
                                             }

                                             var reply = await client.SayAsync(new EchoRequest
                                                                               {
                                                                                 Text = index.ToString(),
                                                                               },
                                                                               deadline: deadline,
                                                                               cancellationToken: cancellation.Token)
                                                                     .ConfigureAwait(false);
                                             if (reply.Text != index.ToString())
                                             {
                                               failures.Enqueue($"{index}: answered {reply.Text}");
                                             }
                                           }
                                           catch (RpcException error) when (error.StatusCode == StatusCode.Cancelled && cancellation.IsCancellationRequested)
                                           {
                                             Interlocked.Increment(ref cancelled);
                                           }
                                           catch (RpcException error)
                                           {
                                             failures.Enqueue($"{index}: {error.StatusCode} {error.Status.Detail}");
                                           }
                                           finally
                                           {
                                             gate.Release();
                                           }
                                         }))
              .ConfigureAwait(false);

    TestContext.WriteLine($"{cancelled} cancelled, {failures.Count} failed");
    Assert.That(failures,
                Is.Empty);
  }

  private void AssertWhereTheCallsWent()
  {
    var calls = nginx_!.Calls();
    TestContext.WriteLine(string.Join(Environment.NewLine,
                                      calls));
    var first = calls.Single(call => call.Call == "first");
    var second = calls.Single(call => call.Call == "second");
    var third = calls.Single(call => call.Call == "third");
    Assert.Multiple(() =>
                    {
                      Assert.That(second.Connection,
                                  Is.EqualTo(first.Connection),
                                  "both calls before the GOAWAY on one connection");
                      Assert.That(third.Connection,
                                  Is.Not.EqualTo(first.Connection),
                                  "the call after it on another");
                      Assert.That(calls.Select(call => call.Status),
                                  Is.All.EqualTo("200"));
                    });
  }

  private static AsyncDuplexStreamingCall<EchoRequest, EchoReply> Chat(Echo.EchoClient client,
                                                                       string          name)
    => client.Chat(new Metadata
                   {
                     {
                       "x-call", name
                     },
                   },
                   DateTime.UtcNow + Patience);

  private static async Task Exchange(AsyncDuplexStreamingCall<EchoRequest, EchoReply> call,
                                     string                                            text)
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

  private static async Task Finish(AsyncDuplexStreamingCall<EchoRequest, EchoReply> call)
  {
    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);
    Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None)
                          .ConfigureAwait(false),
                Is.False);
    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  /// <summary>An nginx with <paramref name="directives" />, and a channel of the transport through it.</summary>
  /// <remarks>The test is ignored before nginx starts when it cannot run here.</remarks>
  private Transport Open(string transport,
                         string directives)
  {
    if (NginxProcess.Executable is null)
    {
      Assert.Ignore($"{NginxProcess.Variable} names no nginx");
    }

#if !NET
    if (transport == "managed")
    {
      Assert.Ignore("grpc-dotnet speaks plain HTTP/2 on .NET only");
    }
#endif

    nginx_ = NginxProcess.Start(Endpoint,
                                directives);
    switch (transport)
    {
      case "native":
        var channel = Runtime.Channel(nginx_!.Endpoint);
        return new Transport(channel.CreateCallInvoker(),
                             channel.DisposeAsync);
#if NET
      case "managed":
        var managed = Grpc.Net.Client.GrpcChannel.ForAddress(nginx_!.Endpoint);
        return new Transport(managed.CreateCallInvoker(),
                             () =>
                             {
                               managed.Dispose();
                               return default;
                             });
#endif
      default:
        throw new ArgumentOutOfRangeException(nameof(transport),
                                              transport,
                                              "a transport is native or managed");
    }
  }

  private sealed class Transport : IAsyncDisposable
  {
    private readonly Func<ValueTask> dispose_;

    internal Transport(CallInvoker     invoker,
                       Func<ValueTask> dispose)
    {
      Invoker  = invoker;
      dispose_ = dispose;
    }

    internal CallInvoker Invoker { get; }

    public ValueTask DisposeAsync()
      => dispose_();
  }
}
