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
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

using Google.Protobuf;

using Grpc.Core;

using Microsoft.Extensions.Configuration;

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class UnaryTests : EchoServerFixture
{
  /// <summary>Two workers, so a test that reads while another callback runs has a thread for
  /// both. Here rather than in the base, which every fixture shares.</summary>
  protected override NativeRuntime Start()
    => NativeRuntime.Create(workerThreads: 2);

  private NativeChannel Channel()
    => Runtime.Channel(Endpoint);

  /// <summary>A channel opened from a configuration, and a call over it.</summary>
  /// <remarks>
  ///   The whole path: an environment variable, .NET's binder, the generated options, the JSON,
  ///   and the engine reading it. `ChannelOptionsTests` stops at the document; only a served call
  ///   says the engine accepted it.
  /// </remarks>
  [Test]
  public async Task AnOptionSetOnlyInTheEnvironmentReachesTheEngine()
  {
    const string prefix = "AKRUSTUNARY_";

    Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__DeliveryCredits",
                                       "4");
    Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__Transport__ConnectTimeoutSeconds",
                                       "30");

    try
    {
      var configuration = new ConfigurationBuilder().AddEnvironmentVariables(prefix)
                                                    .Build();

      await using var channel = Runtime.Channel(Endpoint,
                                                configuration);

      var reply = await Client(channel)
                        .SayAsync(new EchoRequest
                                  {
                                    Text = "bound",
                                  })
                        .ResponseAsync.ConfigureAwait(false);

      Assert.That(reply.Text,
                  Is.EqualTo("bound"));
    }
    finally
    {
      Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__DeliveryCredits",
                                         null);
      Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__Transport__ConnectTimeoutSeconds",
                                         null);
    }
  }

  /// <summary>Opening a channel writes nothing to the options it was given.</summary>
  /// <remarks>
  ///   The factory resolves the delivery window into what it sends. Resolved into the caller's
  ///   instance, a second channel opened from it would inherit the first one's resolution.
  /// </remarks>
  [Test]
  public async Task OpeningAChannelLeavesTheCallersOptionsAsTheyWere()
  {
    var options = new ChannelOptions();

    await using var channel = Runtime.Channel(Endpoint,
                                              options);

    Assert.That(options.DeliveryCredits,
                Is.Null,
                "the window was resolved into the copy the channel holds, not into this");
  }

  /// <summary>A configuration with no section for this is refused, not defaulted.</summary>
  /// <remarks>
  ///   A misspelled section name would otherwise be a channel nobody configured, opened on the
  ///   engine's defaults and behaving almost right. `Channel(endpoint)` is how to ask for those.
  /// </remarks>
  [Test]
  public void AConfigurationWithNoSectionForThisIsRefused()
    => Assert.That(() => Runtime.Channel(Endpoint,
                                         new ConfigurationBuilder().Build()),
                   Throws.TypeOf<InvalidOperationException>());

  /// <summary>Every reply, and every call disposed even if one of them throws.</summary>
  private static async Task<EchoReply[]> RepliesOf(AsyncUnaryCall<EchoReply>[] calls)
  {
    try
    {
      return await Task.WhenAll(Array.ConvertAll(calls,
                                                 call => call.ResponseAsync))
                       .ConfigureAwait(false);
    }
    finally
    {
      foreach (var call in calls)
      {
        call.Dispose();
      }
    }
  }

  [Test]
  public void TheEngineBesideThisHostMatchesItsWordSize()
  {
    var engine = Path.Combine(AppDomain.CurrentDomain.BaseDirectory,
                              NativeMethods.Library + ".dll");
    if (!File.Exists(engine))
    {
      Assert.Ignore("not a Windows build; the engine is an .so or a .dylib");
    }

    using var file = File.OpenRead(engine);
    using var reader = new BinaryReader(file);
    file.Seek(0x3c,
              SeekOrigin.Begin);
    file.Seek(reader.ReadUInt32() + 4,
              SeekOrigin.Begin);

    // The Machine field of the PE COFF header, two bytes past the PE signature: 0x8664 is x64 and
    // 0x014c is x86. A mismatch here is the engine of the wrong architecture beside this host, and
    // it would otherwise surface as a DllNotFoundException that names nothing.
    var machine = reader.ReadUInt16();
    Assert.That(machine,
                Is.EqualTo(IntPtr.Size == 8
                             ? 0x8664
                             : 0x014c),
                $"a {IntPtr.Size * 8}-bit host beside a 0x{machine:x4} engine");
  }

  [Test]
  public void TheAbiVersionIsTheOneThisBindingSpeaks()
    => Assert.That(NativeRuntime.LibraryAbiVersion,
                   Is.EqualTo(1),
                   "the loaded library speaks the ABI this binding was written against");

  [Test]
  public async Task AUnaryCallReachesTheServerAndComesBack()
  {
    await using var channel = Channel();
    var client = Client(channel);

    var headers = new Metadata
                  {
                    {
                      "x-request", "ping"
                    },
                  };
    using var call = client.SayAsync(new EchoRequest
                                     {
                                       Text = "hello",
                                     },
                                     headers);

    var reply = await call.ResponseAsync.ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo("hello"));
                      Assert.That(reply.SawMetadata,
                                  Is.EqualTo("ping"),
                                  "the request metadata crossed the ABI");
                      Assert.That(call.GetStatus()
                                      .StatusCode,
                                  Is.EqualTo(StatusCode.OK));
                    });
  }

  [Test]
  public async Task TheResponseHeadArrivesBeforeTheAnswer()
  {
    await using var channel = Channel();
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "head",
                });

    var head = await call.ResponseHeadersAsync.ConfigureAwait(false);
    await call.ResponseAsync.ConfigureAwait(false);

    Assert.That(head.GetValue("x-answered"),
                Is.EqualTo("yes"));
  }

  [Test]
  public async Task ABlockingCallAnswersTheSameWay()
  {
    await using var channel = Channel();

    var reply = Client(channel)
      .Say(new EchoRequest
           {
             Text = "blocking",
           });

    Assert.That(reply.Text,
                Is.EqualTo("blocking"));
  }

  [Test]
  public async Task AServerThatRefusesComesBackAsThatStatus()
  {
    await using var channel = Channel();

    var thrown = Assert.Throws<RpcException>(() => Client(channel)
                                               .Refuse(new EchoRequest
                                                       {
                                                         Text = "x",
                                                       }));

    Assert.Multiple(() =>
                    {
                      Assert.That(thrown!.StatusCode,
                                  Is.EqualTo(StatusCode.PermissionDenied));
                      Assert.That(thrown.Status.Detail,
                                  Is.EqualTo("not for you"));
                      Assert.That(thrown.Trailers.GetValue("x-reason"),
                                  Is.EqualTo("policy"));
                    });
  }

  [Test]
  public async Task ACancelledCallEndsWithoutWaitingForTheServer()
  {
    await using var channel = Channel();
    using var cancellation = new CancellationTokenSource();

    using var call = Client(channel)
      .NeverAsync(new EchoRequest
                  {
                    Text = "x",
                  },
                  cancellationToken: cancellation.Token);

    cancellation.Cancel();

    var answered = Task.WhenAny(call.ResponseAsync,
                                Task.Delay(TimeSpan.FromSeconds(5)));
    Assert.That(await answered.ConfigureAwait(false),
                Is.SameAs(call.ResponseAsync),
                "the call ended without waiting on the server");

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  [Test]
  public async Task ACallCancelledBeforeItIsSentEndsCancelledAndNotInternal()
  {
    await using var channel = Channel();
    using var cancellation = new CancellationTokenSource();
    cancellation.Cancel();

    // Cancelled before the marshaller has asked for a buffer, so the engine refuses the send
    // itself. What the caller must read is the call's terminal, which is what grpc-dotnet and
    // Grpc.Core both answer here, and not a fault of the binding.
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "x",
                },
                cancellationToken: cancellation.Token);

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  [Test]
  public async Task SeveralCallsShareOneChannel()
  {
    await using var channel = Channel();
    var client = Client(channel);

    var calls = new AsyncUnaryCall<EchoReply>[8];
    for (var index = 0; index < calls.Length; index++)
    {
      calls[index] = client.SayAsync(new EchoRequest
                                     {
                                       Text = $"call-{index}",
                                     });
    }

    var replies = await RepliesOf(calls)
                    .ConfigureAwait(false);

    for (var index = 0; index < replies.Length; index++)
    {
      Assert.That(replies[index]
                    .Text,
                  Is.EqualTo($"call-{index}"));
    }
  }

  [Test]
  public async Task ABinaryMetadataEntryCrossesTheWireAsBytes()
  {
    await using var channel = Channel();
    var headers = new Metadata
                  {
                    {
                      "x-trace-bin", new byte[]
                                     {
                                       0,
                                       1,
                                       2,
                                       255,
                                     }
                    },
                  };

    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "binary",
                },
                headers);

    var reply = await call.ResponseAsync.ConfigureAwait(false);
    Assert.That(reply.SawMetadata,
                Is.EqualTo("000102ff"));
  }

  [Test]
  public async Task CallsWaitForRoomUnderAMemoryCeiling()
  {
    var text = new string('x',
                          100_000);

    var runtime = await RestartAsync(workerThreads: 2,
                                     memoryCeiling: 128 * 1024)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);
    var client = Client(channel);

    var calls = await Task.WhenAll(Enumerable.Range(0,
                                                    16)
                                             .Select(_ => Task.Run(() => client.SayAsync(new EchoRequest
                                                                                         {
                                                                                           Text = text,
                                                                                         }))))
                          .ConfigureAwait(false);

    var replies = await RepliesOf(calls)
                    .ConfigureAwait(false);

    Assert.That(replies,
                Has.All.Matches<EchoReply>(reply => reply.Text == text));
  }

  [Test]
  public async Task AChannelMaySpeakWithADeeperDeliveryWindow()
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              deliveryCredits: 4);

    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "deep",
                                })
                      .ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("deep"));
  }

  /// <summary>The real method, with marshallers of the test's choosing.</summary>
  private static Method<TRequest, TResponse> Say<TRequest, TResponse>(Marshaller<TRequest>  request,
                                                                      Marshaller<TResponse> response)
    => new(MethodType.Unary,
           "armonik.transport.ffi.test.Echo",
           "Say",
           request,
           response);

  private static Marshaller<EchoRequest> Serializing(Action<EchoRequest, SerializationContext> write)
    => new(write,
           context => EchoRequest.Parser.ParseFrom(context.PayloadAsNewBuffer()));

  private static readonly Marshaller<EchoReply> ReplyMarshaller = Marshallers.Create<EchoReply>(message => message.ToByteArray(),
                                                                                                EchoReply.Parser.ParseFrom);

  /// <summary>One reduction per call, because two would read the same ring.</summary>
  /// <remarks>
  ///   A cardinality that answers once takes the stream reader and reduces it to a single, and
  ///   the invoker calls for that where it knows which shape it asked the server for. Nothing
  ///   stops a second caller in the same assembly from asking for another, and the two would
  ///   then divide one message and one terminal between them.
  /// </remarks>
  [Test]
  public async Task ACallIsReadAsASingleResponseOnlyOnce()
  {
    await using var channel = Channel();

    var call = channel.StartCall("/armonik.transport.ffi.test.Echo/Say",
                                 null,
                                 ReplyMarshaller);

    var reduced = call.SingleAsync();

    // The task is discarded so this is an `Action`: NUnit awaits a delegate that returns one,
    // which would pass whether the refusal reached the call site or only the task. What is under
    // test is that it reaches the call site.
    Assert.That(() =>
                {
                  _ = call.SingleAsync();
                },
                Throws.TypeOf<InvalidOperationException>()
                      .With.Message.Contains("already being read"));

    call.Cancel();

    // Bounded, because a drain that stalls would hang the run rather than fail it - and the
    // fixture's teardown asserts the runtime quiesced, so this has to end either way. The
    // reduction's own failure is dropped: what is under test is the refusal above, not how a
    // cancelled call ends.
    var drained = await Task.WhenAny(reduced,
                                     Task.Delay(TimeSpan.FromSeconds(5)))
                            .ConfigureAwait(false);

    Assert.That(drained,
                Is.SameAs(reduced),
                "the cancelled call's reduction ended");

    try
    {
      await reduced.ConfigureAwait(false);
    }
    catch (RpcException)
    {
    }
  }

  /// <summary>Each of these is an error path that must still give the engine back everything it
  /// lent, or the runtime never quiesces - which the fixture's own teardown assertion catches.
  /// </summary>
  [Test]
  public async Task AMarshallerThatMisbehavesStillReturnsWhatTheEngineLent()
  {
    await using var channel = Channel();
    var       invoker = channel.CreateCallInvoker();

    // Announces more than it writes: the engine lends a buffer of that size and would send the
    // arena's leftovers as message bytes.
    var short_ = Assert.Throws<RpcException>(() => invoker.BlockingUnaryCall(Say(Serializing((_,
                                                                                              context) =>
                                                                                             {
                                                                                               context.SetPayloadLength(64);
                                                                                               context.GetBufferWriter()
                                                                                                      .Advance(8);
                                                                                               context.Complete();
                                                                                             }),
                                                                                 ReplyMarshaller),
                                                                             null,
                                                                             new CallOptions(),
                                                                             new EchoRequest()));
    Assert.That(short_!.Status.Detail,
                Does.Contain("announced 64 bytes and wrote 8"));

    // Throws with a buffer lent.
    Assert.Throws<InvalidOperationException>(() => invoker.BlockingUnaryCall(Say(Serializing((_,
                                                                                             context) =>
                                                                                            {
                                                                                              context.SetPayloadLength(16);
                                                                                              throw new InvalidOperationException("the serializer gave up");
                                                                                            }),
                                                                                ReplyMarshaller),
                                                                            null,
                                                                            new CallOptions(),
                                                                            new EchoRequest()));

    // Announces a length no message can be.
    Assert.Throws<ArgumentOutOfRangeException>(() => invoker.BlockingUnaryCall(Say(Serializing((_,
                                                                                               context) => context.SetPayloadLength(-1)),
                                                                                  ReplyMarshaller),
                                                                              null,
                                                                              new CallOptions(),
                                                                              new EchoRequest()));
  }

  /// <summary>A deserializer that throws owes the engine the payload it was handed.</summary>
  [Test]
  public async Task ADeserializerThatThrowsStillConsumesItsPayload()
  {
    await using var channel = Channel();
    var       invoker = channel.CreateCallInvoker();

    var refused = Assert.Throws<RpcException>(() => invoker.BlockingUnaryCall(Say(Marshallers.Create<EchoRequest>(message => message.ToByteArray(),
                                                                                                                 EchoRequest.Parser.ParseFrom),
                                                                                 Marshallers.Create<EchoReply>(message => message.ToByteArray(),
                                                                                                               _ => throw new InvalidOperationException("the deserializer gave up"))),
                                                                             null,
                                                                             new CallOptions(),
                                                                             new EchoRequest
                                                                             {
                                                                               Text = "read me",
                                                                             }));

    Assert.That(refused!.Status.Detail,
                Does.Contain("the deserializer gave up"));
  }

  /// <summary>A message of no bytes, which is what an all-default protobuf serialises to.</summary>
  /// <remarks>The header says a zero-length payload with a real owner must still be consumed -
  /// the credit comes back with the acquittal, not with the bytes - and nothing exercised it end
  /// to end. Both directions here: the request is empty and so is the reply's echo of it.</remarks>
  [Test]
  public async Task AMessageOfNoBytesCrossesAndIsGivenBack()
  {
    await using var channel = Channel();

    var reply = await Client(channel)
                      .SayAsync(new EchoRequest())
                      .ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.Empty);
  }

  /// <summary>Disposing with a call still running, which is the drain loop's only reason to
  /// exist and was never entered with anything in it.</summary>
  [Test]
  public async Task AChannelDisposedWithACallInFlightCancelsItAndComesBack()
  {
    var channel = Channel();
    var running = Client(channel)
      .NeverAsync(new EchoRequest
                  {
                    Text = "held",
                  });

    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    var ended = Assert.ThrowsAsync<RpcException>(async () => await running.ResponseAsync.ConfigureAwait(false));
    Assert.That(ended!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
    Assert.That(channel.DisposeState,
                Is.EqualTo(ChannelDisposeState.Disposed));
  }

  /// <summary>Metadata the engine refuses, as a caller sees it.</summary>
  [Test]
  public async Task AReservedMetadataKeyIsRefusedBeforeTheCallStarts()
  {
    await using var channel = Channel();

    var refused = Assert.Throws<RpcException>(() => Client(channel)
                                                .Say(new EchoRequest(),
                                                     new Metadata
                                                     {
                                                       {
                                                         "grpc-timeout", "1S"
                                                       },
                                                     }));

    Assert.That(refused!.StatusCode,
                Is.EqualTo(StatusCode.Internal));
  }

  [Test]
  public void AWindowOfZeroIsRefusedBeforeAnythingIsOpened()
    => Assert.Throws<ArgumentOutOfRangeException>(() => Runtime.Channel(Endpoint,
                                                                        deliveryCredits: 0));

  /// <summary>Every call of the channel sizes a ring from this, so a window nothing bounds is a
  /// per-call allocation nothing bounds - and, past 2^30, a shift that reaches zero and spins.
  /// </summary>
  [Test]
  public void AWindowDeeperThanAnyRingIsRefusedBeforeAnythingIsOpened()
    => Assert.Multiple(() =>
                       {
                         Assert.Throws<ArgumentOutOfRangeException>(() => Runtime.Channel(Endpoint,
                                                                                          NativeRuntime.MaxDeliveryCredits + 1));
                         Assert.Throws<ArgumentOutOfRangeException>(() => Runtime.Channel(Endpoint,
                                                                                          int.MaxValue));
                       });

  /// <summary>Refused, not dropped: a call that went out without the credentials the caller
  /// attached fails at the server, or is served anonymously, and neither answer names the
  /// binding that discarded them.</summary>
  [Test]
  public async Task CallOptionsThisInvokerCannotHonourAreRefusedRatherThanIgnored()
  {
    await using var channel = Channel();
    var       client  = Client(channel);

    var credentials = Assert.Throws<RpcException>(() => client.Say(new EchoRequest
                                                                  {
                                                                    Text = "identified",
                                                                  },
                                                                  new CallOptions(credentials: CallCredentials.FromInterceptor((_,
                                                                                                                               _) => Task.CompletedTask))));

    Assert.That(credentials!.StatusCode,
                Is.EqualTo(StatusCode.Unimplemented));
    Assert.That(credentials.Status.Detail,
                Does.Contain("call credentials"));
  }

  /// <summary>A per-call authority is refused too, which is the same rule one option later.</summary>
  /// <remarks>The `host` argument overrides the channel's authority for one call, and the endpoint
  /// crosses the ABI once, at the channel. Dropped, it would send the call to a server the caller
  /// did not name and let that server's answer stand for the binding's silence.</remarks>
  [Test]
  public async Task APerCallHostIsRefusedRatherThanDropped()
  {
    await using var channel = Channel();

    // Built here rather than taken from the generated client, which passes no host: reaching the
    // argument means calling the invoker the way a generated stub does.
    var say = new Method<EchoRequest, EchoReply>(MethodType.Unary,
                                                 "armonik.transport.ffi.test.Echo",
                                                 "Say",
                                                 Marshallers.Create(request => request.ToByteArray(),
                                                                    EchoRequest.Parser.ParseFrom),
                                                 Marshallers.Create(reply => reply.ToByteArray(),
                                                                    EchoReply.Parser.ParseFrom));

    var refused = Assert.Throws<RpcException>(() => channel.CreateCallInvoker()
                                                          .BlockingUnaryCall(say,
                                                                             "elsewhere.test",
                                                                             new CallOptions(),
                                                                             new EchoRequest
                                                                             {
                                                                               Text = "redirected",
                                                                             }));

    Assert.That(refused!.StatusCode,
                Is.EqualTo(StatusCode.Unimplemented));
    Assert.That(refused.Status.Detail,
                Does.Contain("per-call host"));
  }

  /// <summary>A serializer that announces a length twice is named, and the call is not blamed.</summary>
  /// <remarks>The engine lends one buffer at a time and answers a second ask with the status it
  /// also answers for a call that has ended, so the caller was told its call was over. Which
  /// stage the message is in is known here and nowhere else.</remarks>
  [Test]
  public async Task ASerializerThatAnnouncesTwiceIsNamedRatherThanTheCall()
  {
    await using var channel = Channel();

    var refused = Assert.Throws<RpcException>(() => channel.CreateCallInvoker()
                                                          .BlockingUnaryCall(SayWith(Marshallers.Create<EchoRequest>((request,
                                                                                                                      context) =>
                                                                                                                     {
                                                                                                                       var bytes = request.ToByteArray();
                                                                                                                       context.SetPayloadLength(bytes.Length);
                                                                                                                       context.SetPayloadLength(bytes.Length);
                                                                                                                     },
                                                                                                                     context => EchoRequest.Parser
                                                                                                                                           .ParseFrom(context.PayloadAsNewBuffer()))),
                                                                             null,
                                                                             new CallOptions(),
                                                                             new EchoRequest
                                                                             {
                                                                               Text = "announced twice",
                                                                             }));

    Assert.Multiple(() =>
                    {
                      Assert.That(refused!.StatusCode,
                                  Is.EqualTo(StatusCode.Internal));
                      Assert.That(refused.Status.Detail,
                                  Does.Contain("announced a length twice"));
                    });
  }

  /// <summary>And a serializer that swaps the buffer it was lent for an array of its own sends the
  /// array, whatever it announced first.</summary>
  /// <remarks>The one path that takes two buffers for one message: the first is given back before
  /// the second is asked for, because the engine lends one at a time.</remarks>
  [Test]
  public async Task AnArrayReplacesWhateverTheSerializerAnnounced()
  {
    await using var channel = Channel();

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(SayWith(Marshallers.Create<EchoRequest>((request,
                                                                                   context) =>
                                                                                  {
                                                                                    context.SetPayloadLength(1);
                                                                                    context.Complete(request.ToByteArray());
                                                                                  },
                                                                                  context => EchoRequest.Parser
                                                                                                        .ParseFrom(context.PayloadAsNewBuffer()))),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = "the array, not the announcement",
                                          });

    Assert.That(reply.Text,
                Is.EqualTo("the array, not the announcement"));
  }

  private static Method<EchoRequest, EchoReply> SayWith(Marshaller<EchoRequest> requests)
    => new(MethodType.Unary,
           "armonik.transport.ffi.test.Echo",
           "Say",
           requests,
           Marshallers.Create(reply => reply.ToByteArray(),
                              EchoReply.Parser.ParseFrom));

  [Test]
  public async Task TheChannelsTwoHalvesAgreeOnItsState()
  {
    var channel = Channel();
    Assert.That(channel.NativeState,
                Is.EqualTo(NativeMethods.AkChannelState.Open));

    await Client(channel)
          .SayAsync(new EchoRequest
                    {
                      Text = "state",
                    })
          .ResponseAsync.ConfigureAwait(false);

    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(channel.NativeState,
                                  Is.EqualTo(NativeMethods.AkChannelState.Closed));
                      Assert.That(channel.DisposeState,
                                  Is.EqualTo(ChannelDisposeState.Disposed));
                    });

  }

  /// <summary>A second engine is refused while one lives, by the engine and not by this side.</summary>
  /// <remarks>Requirement 14.9: several tokio runtimes in one process would share the machine's
  /// cores without knowing of each other. What the caller gets is an answer and not a wait, which
  /// is what owning the runtime buys - there is no lease whose return it could be waiting for.
  /// </remarks>
  [Test]
  public void ASecondRuntimeIsRefusedWhileOneLives()
    => Assert.That(() => NativeRuntime.Create(),
                   Throws.TypeOf<InvalidOperationException>(),
                   "the engine admits one runtime per process and says so");

  /// <summary>A runtime that is going away opens no new channel.</summary>
  [Test]
  public async Task AChannelAskedOfARuntimeGoingAwayIsRefused()
  {
    var runtime = Runtime;

    await GiveTheRuntimeBack()
      .ConfigureAwait(false);

    Assert.That(() => runtime.Channel(Endpoint),
                Throws.TypeOf<ObjectDisposedException>());
  }

  /// <summary>Disposing the runtime disposes the channels it made, whatever the caller did.</summary>
  /// <remarks>Which is the whole reason it keeps them: a channel outliving its engine holds a
  /// handle into a library that may have been unloaded, and no order of disposal a caller has to
  /// remember can be relied on to prevent that. One of the two is disposed first here, because a
  /// caller doing half the work itself is the ordinary case and not a race.</remarks>
  [Test]
  public async Task DisposingTheRuntimeDisposesTheChannelsItMade()
  {
    var itsOwn = Channel();
    var left = Channel();

    await itsOwn.DisposeAsync()
                .ConfigureAwait(false);

    await GiveTheRuntimeBack()
      .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(left.DisposeState,
                                  Is.EqualTo(ChannelDisposeState.Disposed),
                                  "the one the caller left behind went with the runtime");
                      Assert.That(itsOwn.DisposeState,
                                  Is.EqualTo(ChannelDisposeState.Disposed),
                                  "and the one it had already disposed is not disposed twice");
                    });
  }

  [Test]
  public async Task AChannelThatIsReleasedTakesNoNewCall()
  {
    var channel = Channel();
    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    var thrown = Assert.Throws<RpcException>(() => Client(channel)
                                              .Say(new EchoRequest
                                                   {
                                                     Text = "x",
                                                   }));

    Assert.Multiple(() =>
                    {
                      Assert.That(thrown!.StatusCode,
                                  Is.EqualTo(StatusCode.Unavailable));
                      Assert.That(thrown.Status.Detail,
                                  Does.Contain("takes no new calls"),
                                  "refused for being disposed, not for something else");
                    });
  }
}
