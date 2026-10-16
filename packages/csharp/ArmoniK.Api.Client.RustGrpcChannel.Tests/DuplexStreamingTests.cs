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

using Google.Protobuf;

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

  /// <summary>A write parked against the memory ceiling ends when its call does, with nobody reading.</summary>
  /// <remarks>
  ///   The application awaits its write before it reads, as a duplex caller may, so the terminal
  ///   lands in the ring and nothing consumes it. No lend of the call can succeed any more and no
  ///   BUDGET_WAKE comes for a call that is over, so the wait has to end on the terminal's arrival,
  ///   as the model's <c>BudgetWaitEndsWhenHopeless</c> promises. The ceiling is held by a
  ///   serializer on another channel, and no call reads while it is, so the call's deadline is the
  ///   only terminal that comes.
  /// </remarks>
  [Test]
  public async Task AWriteParkedAgainstTheCeilingEndsWithItsCallUnread()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var holder = runtime.Channel(Endpoint);
    await using var parked = runtime.Channel(Endpoint);

    using var holding = new ManualResetEventSlim(false);
    using var release = new ManualResetEventSlim(false);
    using var serializing = new ManualResetEventSlim(false);

    var wholeCeiling = Requests((request,
                                 context) =>
                                {
                                  context.SetPayloadLength(ceiling);
                                  holding.Set();
                                  release.Wait();
                                  context.Complete(request.ToByteArray());
                                });
    var announces = Requests((request,
                              context) =>
                             {
                               serializing.Set();
                               context.Complete(request.ToByteArray());
                             });

    using var holdsTheCeiling = holder.CreateCallInvoker()
                                      .AsyncClientStreamingCall(Method(MethodType.ClientStreaming,
                                                                       "Collect",
                                                                       wholeCeiling),
                                                                null,
                                                                new CallOptions());
    var lending = Task.Run(() => holdsTheCeiling.RequestStream.WriteAsync(new EchoRequest
                                                                         {
                                                                           Text = "the ceiling",
                                                                         }));
    Assert.That(holding.Wait(TimeSpan.FromSeconds(30)),
                Is.True,
                "the ceiling is lent and held");

    var call = parked.CreateCallInvoker()
                     .AsyncDuplexStreamingCall(Method(MethodType.DuplexStreaming,
                                                      "Chat",
                                                      announces),
                                               null,
                                               new CallOptions(deadline: DateTime.UtcNow + TimeSpan.FromSeconds(2)));
    try
    {
      var parkedWrite = Task.Run(() => call.RequestStream.WriteAsync(new EchoRequest
                                                                    {
                                                                      Text = "no room for this",
                                                                    }));
      Assert.That(serializing.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the call reached its lend, which the ceiling refuses");

      // Awaited through its outcome rather than by an assertion that awaits: what the missing
      // wake-up costs is a wait, and an unbounded assertion would hang the run instead of naming it.
      var outcome = Task.Run(async () =>
                             {
                               try
                               {
                                 await parkedWrite.ConfigureAwait(false);
                                 return null as Exception;
                               }
                               catch (Exception raised)
                               {
                                 return raised;
                               }
                             });

      Assert.That(outcome.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the parked write hears that its call is over, with nobody reading the call");
      Assert.That(outcome.Result,
                  Is.InstanceOf<RpcException>());
    }
    finally
    {
      call.Dispose();
      release.Set();
      await lending.ConfigureAwait(false);
      await holdsTheCeiling.RequestStream.CompleteAsync()
                           .ConfigureAwait(false);
      await holdsTheCeiling.ResponseAsync.ConfigureAwait(false);
    }
  }

  private static Marshaller<EchoRequest> Requests(Action<EchoRequest, SerializationContext> write)
    => Marshallers.Create(write,
                          context => EchoRequest.Parser.ParseFrom(context.PayloadAsNewBuffer()));

  private static Method<EchoRequest, EchoReply> Method(MethodType             type,
                                                       string                 name,
                                                       Marshaller<EchoRequest> requests)
    => new(type,
           "armonik.transport.ffi.test.Echo",
           name,
           requests,
           Marshallers.Create(reply => reply.ToByteArray(),
                              EchoReply.Parser.ParseFrom));
}
