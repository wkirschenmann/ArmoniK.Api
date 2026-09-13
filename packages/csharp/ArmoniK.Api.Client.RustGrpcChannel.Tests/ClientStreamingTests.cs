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

using Google.Protobuf;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Client streaming: several messages, then one reply.</summary>
[TestFixture]
public class ClientStreamingTests : EchoServerFixture
{
  private static readonly string[] Sent =
  {
    "one",
    "two",
    "three",
  };

  [Test]
  public async Task EveryMessageReachesTheServerInOrderAndTheReplyNamesThemAll()
  {
    await using var channel = Runtime.Channel(Endpoint);
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
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Collect();

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);

    var reply = await call.ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("0:"));
  }

  /// <summary>A write into a call the server ended answers with the call's status.</summary>
  /// <remarks>The one path that reaches for the binding's internal <c>CallEnded</c>: the unary
  /// invoker catches it, and nothing did on this side, so application code saw a type it cannot
  /// name. Over grpc-dotnet a caller hears an <c>RpcException</c> here.</remarks>
  [Test]
  public async Task AWriteAfterTheCallEndedIsAnRpcException()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .CollectRefused();

    // Awaited first, so the terminal is observed before the write below rather than racing it.
    try
    {
      await call.ResponseAsync.ConfigureAwait(false);
      Assert.Fail("the server refused the call");
    }
    catch (RpcException)
    {
    }

    Assert.That(() => call.RequestStream.WriteAsync(new EchoRequest
                                                     {
                                                       Text = "late",
                                                     }),
                Throws.InstanceOf<RpcException>(),
                "the call's status, not a type of the binding's own");
  }

  /// <summary>A second write while one is in flight is refused, and the first still answers.</summary>
  /// <remarks>What it costs when it is not: both writes publish their acquittal into one field, so
  /// the first ends up waiting on a completion source nothing holds any more and hangs until the
  /// call ends - the write that broke no rule being the one that pays.
  /// <para>
  ///   The first write is held inside its own serializer rather than left to the network, because
  ///   over loopback a write is acquitted before the next statement runs: measured against this
  ///   test written the obvious way, the second write found the field free and succeeded, and the
  ///   defect went unseen. Inside the serializer the claim is certainly held, since it is taken
  ///   before the message is serialized at all.
  /// </para></remarks>
  [Test]
  public async Task ASecondWriteWhileOneIsInFlightIsRefused()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var serializing = new ManualResetEventSlim(false);
    using var finish = new ManualResetEventSlim(false);

    // The first message alone is held. A serializer that held every one would block the second
    // write inside itself, and the deadlock would be the test's own rather than the binding's.
    var first = 0;
    var held = Marshallers.Create<EchoRequest>((request,
                                                context) =>
                                               {
                                                 if (Interlocked.Exchange(ref first,
                                                                          1) == 0)
                                                 {
                                                   serializing.Set();
                                                   finish.Wait();
                                                 }

                                                 context.Complete(request.ToByteArray());
                                               },
                                               context => EchoRequest.Parser
                                                                     .ParseFrom(context.PayloadAsNewBuffer()));

    using var call = channel.CreateCallInvoker()
                            .AsyncClientStreamingCall(CollectWith(held),
                                                      null,
                                                      new CallOptions());

    var writing = Task.Run(() => call.RequestStream.WriteAsync(new EchoRequest
                                                               {
                                                                 Text = "one",
                                                               }));

    try
    {
      Assert.That(serializing.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the first write reached its serializer");

      Assert.That(() => call.RequestStream.WriteAsync(new EchoRequest
                                                      {
                                                        Text = "two",
                                                      }),
                  Throws.InstanceOf<InvalidOperationException>(),
                  "one writer at a time, and the second is told which rule it broke");
    }
    finally
    {
      // Released whatever happened above: a serializer left waiting holds a buffer of the
      // engine's, and the channel's disposal waits for that - so a failed assertion would hang
      // the run rather than report.
      finish.Set();
    }

    // Bounded, because what the missing refusal costs is a wait and not a fault: the second write
    // takes the field the first is waiting on, so the first waits for an acquittal nothing
    // completes. Unbounded, this assertion would hang the suite instead of naming the defect.
    Assert.That(writing.Wait(TimeSpan.FromSeconds(30)),
                Is.True,
                "and the write that broke no rule answers rather than waiting for an acquittal a second writer took from it");
  }

  /// <summary>A send parked against the memory ceiling is released by its own call's terminal.</summary>
  /// <remarks>
  ///   What it costs when it is not: the settlement waits for the sender to hand its buffer back
  ///   before it cancels the token that sender is waiting on, so a call whose terminal is already
  ///   in waits for room to serialize a message that can no longer go anywhere - and the room is
  ///   other calls' to give up, on a schedule this one does not control. Here nothing gives it up:
  ///   the ceiling is held by a serializer on another channel that is held open on purpose.
  ///   <para>
  ///     The disposal path is not this one and was never exposed: `CancelAndDrain` ends the call
  ///     before it drains, so a cancelled or disposed call already releases its parked sender.
  ///     What this drives is the natural terminal - the server answered, the reduction consumed
  ///     it, and nobody cancelled anything.
  ///   </para>
  /// </remarks>
  [Test]
  public async Task ASendParkedAgainstTheCeilingIsReleasedByItsOwnTerminal()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var holder = runtime.Channel(Endpoint);
    await using var parked = runtime.Channel(Endpoint);

    using var holding = new ManualResetEventSlim(false);
    using var release = new ManualResetEventSlim(false);
    using var serializing = new ManualResetEventSlim(false);

    // Lends the whole ceiling and keeps it: `bytes_used` reaches `ceiling` and stays there for as
    // long as this serializer has not returned, which is what leaves the call below no room.
    var wholeCeiling = Marshallers.Create<EchoRequest>((request,
                                                        context) =>
                                                       {
                                                         context.SetPayloadLength(ceiling);
                                                         holding.Set();
                                                         release.Wait();

                                                         // The announcement was for the room, not
                                                         // for the message: the array replaces it,
                                                         // giving the ceiling back once the hold
                                                         // has done its work.
                                                         context.Complete(request.ToByteArray());
                                                       },
                                                       context => EchoRequest.Parser
                                                                             .ParseFrom(context.PayloadAsNewBuffer()));

    // Says when the send below has reached its lend. What follows it is one downcall answering
    // BUDGET_BUSY and the wait that is the subject here; what follows the assertions is a server
    // round trip, which is the longer of the two by orders of magnitude.
    var announces = Marshallers.Create<EchoRequest>((request,
                                                     context) =>
                                                    {
                                                      serializing.Set();
                                                      context.Complete(request.ToByteArray());
                                                    },
                                                    context => EchoRequest.Parser
                                                                          .ParseFrom(context.PayloadAsNewBuffer()));

    using var holdsTheCeiling = holder.CreateCallInvoker()
                                      .AsyncClientStreamingCall(CollectWith(wholeCeiling),
                                                                null,
                                                                new CallOptions());
    var lending = Task.Run(() => holdsTheCeiling.RequestStream.WriteAsync(new EchoRequest
                                                                         {
                                                                           Text = "the ceiling",
                                                                         }));

    Assert.That(holding.Wait(TimeSpan.FromSeconds(30)),
                Is.True,
                "the ceiling is lent and held");

    try
    {
      using var call = parked.CreateCallInvoker()
                             .AsyncClientStreamingCall(CollectWith(announces),
                                                       null,
                                                       new CallOptions());

      var parkedWrite = Task.Run(() => call.RequestStream.WriteAsync(new EchoRequest
                                                                    {
                                                                      Text = "no room for this",
                                                                    }));

      Assert.That(serializing.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the second call reached its lend, which the ceiling refuses");

      // The server answers on the half-close, so the terminal arrives while that send is parked -
      // and the reduction below consumes it, which is what starts the settlement.
      await call.RequestStream.CompleteAsync()
                .ConfigureAwait(false);

      var reply = await call.ResponseAsync.ConfigureAwait(false);
      Assert.That(reply.Text,
                  Is.EqualTo("0:"),
                  "the server read no message, the one this call had never left");

      // Bounded, and awaited through its outcome rather than by an assertion that awaits: what
      // the missing cancel costs is a wait and not a fault, so an unbounded assertion here would
      // hang the run instead of naming the defect.
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
                  "the parked send hears that its call is over, rather than waiting for room nobody is giving back");
      Assert.That(outcome.Result,
                  Is.InstanceOf<RpcException>(),
                  "and hears it as this binding's one public failure type");
    }
    finally
    {
      release.Set();
      await lending.ConfigureAwait(false);
      await holdsTheCeiling.RequestStream.CompleteAsync()
                           .ConfigureAwait(false);
      await holdsTheCeiling.ResponseAsync.ConfigureAwait(false);
    }
  }

  private static Method<EchoRequest, EchoReply> CollectWith(Marshaller<EchoRequest> requests)
    => new(MethodType.ClientStreaming,
           "armonik.transport.ffi.test.Echo",
           "Collect",
           requests,
           Marshallers.Create(reply => reply.ToByteArray(),
                              EchoReply.Parser.ParseFrom));

  [Test]
  public async Task AWriteAfterTheStreamIsClosedIsRefused()
  {
    await using var channel = Runtime.Channel(Endpoint);
    using var call = Client(channel)
      .Collect();

    call.RequestStream.CompleteAsync()
        .GetAwaiter()
        .GetResult();

    Assert.That(() => call.RequestStream.WriteAsync(new EchoRequest
                                                    {
                                                      Text = "late",
                                                    }),
                Throws.InstanceOf<InvalidOperationException>(),
                "the engine would refuse it anyway, but the writer says which rule was broken");
  }
}
