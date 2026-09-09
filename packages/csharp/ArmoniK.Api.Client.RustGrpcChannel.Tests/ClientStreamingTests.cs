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
    await using var channel = NativeRuntimeFactory.Channel(Endpoint);
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
    await using var channel = NativeRuntimeFactory.Channel(Endpoint);
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
    await using var channel = NativeRuntimeFactory.Channel(Endpoint);
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
    await using var channel = NativeRuntimeFactory.Channel(Endpoint);
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
    await using var channel = NativeRuntimeFactory.Channel(Endpoint);
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
