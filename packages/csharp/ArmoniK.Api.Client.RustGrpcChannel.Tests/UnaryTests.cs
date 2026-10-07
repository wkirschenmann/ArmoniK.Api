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
using System.IO;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
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
  private NativeChannel Channel()
    => Runtime.Channel(Endpoint);

  /// <summary>Room for one of the messages the ceiling test sends, and not two.</summary>
  private const ulong Ceiling = 128 * 1024;

  /// <summary>Past the first threshold by a reply for every call the ceiling test makes: a call
  /// admitted to read below the first takes at most one reply past it, so none reaches this.</summary>
  private const ulong HardCeiling = Ceiling + 16 * 100_000;

  /// <summary>Every option set in a configuration, and a call over the channel it opens.</summary>
  /// <remarks>
  ///   The whole path: an environment variable, the generated binding and options, the JSON, and
  ///   the engine reading it. `ChannelOptionsTests` stops at the document; only a served call says
  ///   the engine accepted it. The timeout carries a fraction, which a culture's decimal comma or an
  ///   integer reading would break, and the proxy is an alternative, which only its key names.
  /// </remarks>
  [Test]
  public async Task EveryOptionSetOnlyInTheEnvironmentReachesTheEngine()
  {
    const string prefix = "AKRUSTUNARY_";

    var variables = new[]
                    {
                      ("Grpc__Host__Receive__Window", "1"),
                      ("Grpc__Receive__MaxMessageSize", "65536"),
                      ("Grpc__Host__Send__Window", "2"),
                      ("Transport__ConnectTimeoutSeconds", "2.5"),
                      ("Transport__Proxy__None", "true"),
                      ("Grpc__UserAgent", "unary-tests"),
                    };

    foreach (var (name, value) in variables)
    {
      Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__" + name,
                                         value);
    }

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
      foreach (var (name, _) in variables)
      {
        Environment.SetEnvironmentVariable(prefix + "RustGrpcChannel__" + name,
                                           null);
      }
    }
  }

  /// <summary>A channel opened on an empty endpoint reaches the Endpoint of the runtime's options.</summary>
  [Test]
  public async Task AChannelWithNoEndpointReachesTheRuntimes()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new RuntimeOptions
                                                                {
                                                                  Endpoint = Endpoint,
                                                                }))
                    .ConfigureAwait(false);
    await using var channel = runtime.Channel(string.Empty);

    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "the runtime's",
                                })
                      .ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("the runtime's"));
  }

  /// <summary>And is refused where the runtime's options name none.</summary>
  [Test]
  public void AChannelWithNoEndpointIsRefusedWhereTheRuntimeNamesNone()
    => Assert.That(() => Runtime.Channel(string.Empty),
                   Throws.InstanceOf<ArgumentException>()
                         .With.Message.Contains("Endpoint"));

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

    Assert.That(options.Grpc,
                Is.Null,
                "the window was resolved into the copy the channel holds, not into this");
  }

  /// <summary>The shortest timeout this side admits is one the engine admits too.</summary>
  /// <remarks>
  ///   The two check the same bound, each on its own, so a copy that drifted would refuse a
  ///   document the other had let through - and the engine's refusal names no option.
  /// </remarks>
  [Test]
  public async Task TheShortestTimeoutThisSideAdmitsOpensAChannel()
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              new ChannelOptions
                                              {
                                                Transport = new TransportOptions
                                                            {
                                                              ConnectTimeoutSeconds = 1e-9,
                                                            },
                                              });

    Assert.That(channel.NativeState,
                Is.EqualTo(ak_channel_state.AK_CHANNEL_OPEN));
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

    // The Machine field of the PE COFF header, two bytes past the PE signature. Read against the
    // process's architecture and not its pointer width, which names two of the three: an arm64
    // host is eight bytes wide like an x64 one and wants a different engine, and requirement 8.1
    // ships win-arm64. A mismatch here is the engine of the wrong architecture beside this host,
    // and it would otherwise surface as a DllNotFoundException that names nothing.
    var machine = reader.ReadUInt16();
    var wanted = RuntimeInformation.ProcessArchitecture switch
                 {
                   Architecture.X64   => 0x8664,
                   Architecture.X86   => 0x014c,
                   Architecture.Arm64 => 0xaa64,
                   Architecture.Arm   => 0x01c4,
                   var other          => throw new InconclusiveException($"no PE machine is recorded here for {other}"),
                 };

    Assert.That(machine,
                Is.EqualTo(wanted),
                $"a {RuntimeInformation.ProcessArchitecture} host beside a 0x{machine:x4} engine");
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

  /// <summary>A stream the peer resets carries the reason it was reset with.</summary>
  /// <remarks>
  ///   ENHANCE_YOUR_CALM, because its code, RESOURCE_EXHAUSTED, is one no default reading of a
  ///   reset lands on. Here and not only in the engine's own suite, which links tonic with its
  ///   `server` feature and so with a reset table of tonic's: the library this package ships is
  ///   built without it, and reads the reason only through the engine's.
  /// </remarks>
  [TestCase("before the head")]
  [TestCase("after the head")]
  public async Task AStreamThePeerResetsCarriesTheReasonItWasResetWith(string when)
  {
    await using var channel = Channel();

    var thrown = Assert.Throws<RpcException>(() => Client(channel)
                                               .Reset(new EchoRequest
                                                      {
                                                        Text = when,
                                                      }));

    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.ResourceExhausted),
                thrown.Status.Detail);
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
  public async Task ACallPastItsDeadlineEndsDeadlineExceededWithoutWaitingForTheServer()
  {
    await using var channel = Channel();

    using var call = Client(channel)
      .NeverAsync(new EchoRequest
                  {
                    Text = "x",
                  },
                  deadline: DateTime.UtcNow.AddMilliseconds(200));

    var answered = Task.WhenAny(call.ResponseAsync,
                                Task.Delay(TimeSpan.FromSeconds(30)));
    Assert.That(await answered.ConfigureAwait(false),
                Is.SameAs(call.ResponseAsync),
                "the deadline ended the call");

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.DeadlineExceeded));
  }

  [Test]
  public async Task ADeadlineAlreadyPassedEndsTheCallDeadlineExceeded()
  {
    await using var channel = Channel();
    var             client  = Client(channel);

    foreach (var deadline in new[]
                             {
                               DateTime.UtcNow.AddSeconds(-1),
                               DateTime.MinValue,
                             })
    {
      var thrown = Assert.ThrowsAsync<RpcException>(async () => await client.SayAsync(new EchoRequest
                                                                                      {
                                                                                        Text = "late",
                                                                                      },
                                                                                      deadline: deadline)
                                                                            .ResponseAsync.ConfigureAwait(false));
      Assert.That(thrown!.StatusCode,
                  Is.EqualTo(StatusCode.DeadlineExceeded),
                  $"{deadline:O}");
    }
  }

  /// <summary>
  ///   A deadline in the future, however far, lets the call answer, and one that is not UTC is
  ///   refused, as grpc-dotnet refuses it.
  /// </summary>
  [Test]
  public async Task ADistantDeadlineIsHonouredAndALocalOneIsRefused()
  {
    await using var channel = Channel();
    var             client  = Client(channel);

    foreach (var deadline in new[]
                             {
                               DateTime.UtcNow.AddMinutes(5),
                               DateTime.UtcNow.AddYears(1000),
                               DateTime.MaxValue,
                             })
    {
      var reply = await client.SayAsync(new EchoRequest
                                        {
                                          Text = "on time",
                                        },
                                        deadline: deadline)
                              .ResponseAsync.ConfigureAwait(false);
      Assert.That(reply.Text,
                  Is.EqualTo("on time"));
    }

    Assert.Throws<InvalidOperationException>(() => client.SayAsync(new EchoRequest
                                                                   {
                                                                     Text = "local",
                                                                   },
                                                                   deadline: DateTime.Now.AddMinutes(5)));
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

  /// <remarks>
  ///   The server lists these entries as it read them and sends them back in its head, so they
  ///   leave through the binding's encoder and the engine's decoder and return through the
  ///   engine's encoder and the binding's decoder: each codec is read against the other rather
  ///   than against its own expectations.
  /// </remarks>
  [Test]
  public async Task MetadataCrossesBothCodecsAsItWasSent()
  {
    await using var channel = Channel();
    var sent = new Metadata
               {
                 {
                   "x-reflect-text", "value"
                 },
                 {
                   "x-reflect-empty", string.Empty
                 },
                 {
                   "x-reflect-twice", "first"
                 },
                 {
                   "x-reflect-twice", "second"
                 },
                 {
                   "x-reflect-bin", new byte[]
                                    {
                                      0,
                                      1,
                                      2,
                                      255,
                                    }
                 },
                 {
                   "x-reflect-empty-bin", Array.Empty<byte>()
                 },
               };

    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "reflect",
                },
                sent);

    var head = await call.ResponseHeadersAsync.ConfigureAwait(false);
    var reply = await call.ResponseAsync.ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.SawMetadata.Split('\n'),
                                  Is.EqualTo(Reflected(sent)),
                                  "what the server read");
                      Assert.That(Reflected(head),
                                  Is.EqualTo(Reflected(sent)),
                                  "what the binding read back");
                    });
  }

  // Ordered by key, since a header map need not keep the order of distinct keys; the sort is
  // stable, so one key's values keep theirs, which HTTP does keep.
  private static string[] Reflected(IEnumerable<Metadata.Entry> entries)
    => entries.Where(entry => entry.Key.StartsWith("x-reflect-",
                                                   StringComparison.Ordinal))
              .OrderBy(entry => entry.Key,
                       StringComparer.Ordinal)
              .Select(entry => entry.IsBinary
                                 ? $"{entry.Key}={BitConverter.ToString(entry.ValueBytes)}"
                                 : $"{entry.Key}={entry.Value}")
              .ToArray();

  /// <remarks>What grpc-dotnet answers: a call no response reached fails its headers with its
  /// status, and its response the same way.</remarks>
  [Test]
  public async Task TheHeadersOfACallNoResponseReachedFailWithItsStatus()
  {
    // No retry: the subject is the head of the one attempt that dials nothing.
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              new ChannelOptions
                                              {
                                                Grpc = new GrpcOptions
                                                       {
                                                         Retry = new RetryOptions
                                                                 {
                                                                   MaxAttempts = 1,
                                                                 },
                                                       },
                                              });
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "nobody",
                });

    var headers = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseHeadersAsync.ConfigureAwait(false));
    var response = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));

    Assert.Multiple(() =>
                    {
                      Assert.That(headers!.StatusCode,
                                  Is.EqualTo(StatusCode.Unavailable));
                      Assert.That(response!.StatusCode,
                                  Is.EqualTo(StatusCode.Unavailable));
                    });
  }

  /// <remarks>A send that fails ends the call before any response, and whichever of the ring's
  /// consumers takes its head, the headers answer that as a call no response reached.</remarks>
  [Test]
  public async Task TheHeadersOfACallWhoseSendFailedFailWithItsStatus()
  {
    await using var channel = Channel();
    using var call = channel.CreateCallInvoker()
                            .AsyncUnaryCall(Say(Serializing((_,
                                                             context) =>
                                                            {
                                                              context.SetPayloadLength(16);
                                                              throw new InvalidOperationException("the serializer gave up");
                                                            }),
                                                ReplyMarshaller),
                                            null,
                                            new CallOptions(),
                                            new EchoRequest());

    var headers = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseHeadersAsync.ConfigureAwait(false));
    Assert.That(headers!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  /// <remarks>What grpc-dotnet answers: a Trailers-Only response's headers are its one header
  /// block, the trailers, whatever the status - the status is the response's to report.</remarks>
  [Test]
  public async Task ATrailersOnlyResponseAnswersItsHeadersWithItsTrailers()
  {
    await using var channel = Channel();
    using var call = Client(channel)
      .RefuseAsync(new EchoRequest
                   {
                     Text = "refused",
                   });

    var headers = await call.ResponseHeadersAsync.ConfigureAwait(false);
    var response = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));

    Assert.Multiple(() =>
                    {
                      Assert.That(headers.GetValue("x-reason"),
                                  Is.EqualTo("policy"));
                      Assert.That(response!.StatusCode,
                                  Is.EqualTo(StatusCode.PermissionDenied));
                    });
  }

  [Test]
  public async Task CallsWaitForRoomUnderAMemoryCeiling()
  {
    var text = new string('x',
                          100_000);

    var runtime = await RestartAsync(memoryCeiling: Ceiling,
                                     memoryHardCeiling: HardCeiling)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);
    var client = Client(channel);

    // Sampled while they run, because what the ceilings promise is about the middle of this and
    // not its end: that the bytes held at once, sent and received, never pass the second. Read at
    // the end alone, every call has given everything back and the reading is zero whether the
    // ceiling held or not.
    using var over = new CancellationTokenSource();
    var high = 0UL;
    var watching = Task.Run(() =>
                            {
                              while (!over.IsCancellationRequested)
                              {
                                if (Usage(runtime.Handle) is { } used)
                                {
                                  high = Math.Max(high,
                                                  used);
                                }

                                // Yielded rather than spun: the peak lasts as long as a lend
                                // does, so this samples often enough without holding a core
                                // against the calls it is watching.
                                Thread.Yield();
                              }
                            });

    var running = Task.WhenAll(Enumerable.Range(0,
                                                16)
                                         .Select(_ => Task.Run(() => client.SayAsync(new EchoRequest
                                                                                     {
                                                                                       Text = text,
                                                                                     }))));

    // Bounded, because a regression in the credit accounting is a wait and not a fault: sixteen
    // calls that never get their room would hold the run rather than report.
    Assert.That(running.Wait(TimeSpan.FromSeconds(60)),
                Is.True,
                "every call found its room");

    var replies = await RepliesOf(await running.ConfigureAwait(false))
                    .ConfigureAwait(false);

    over.Cancel();
    await watching.ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(replies,
                                  Has.All.Matches<EchoReply>(reply => reply.Text == text));
                      Assert.That(high,
                                  Is.GreaterThanOrEqualTo((ulong)text.Length),
                                  "the ceiling was reached, so a call did wait for room");
                      Assert.That(high,
                                  Is.LessThanOrEqualTo(HardCeiling),
                                  "and nothing was held past the second threshold");
                    });
  }

  [Test]
  public async Task AChannelMaySpeakWithADeeperDeliveryWindow()
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              deliveryCredits: 8);

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
  /// <remarks>`InvalidArgument` and not `Internal`: nothing left this process, and what was wrong
  /// is what the caller handed over. gRPC's own table maps a caller's error to INVALID_ARGUMENT,
  /// and `Internal` is what this binding says when the fault is its own - so pinning `Internal`
  /// here would have made a caller's mistake indistinguishable from a bug in the binding.</remarks>
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

    Assert.Multiple(() =>
                    {
                      Assert.That(refused!.StatusCode,
                                  Is.EqualTo(StatusCode.InvalidArgument),
                                  "what the caller handed over, not a fault of the binding's");
                      Assert.That(refused.Status.Detail,
                                  Does.Contain("grpc-timeout"),
                                  "and which entry it was, which one status for a whole document cannot say");
                    });
  }

  /// <summary>A call refused over its metadata leaves nothing rooted behind it.</summary>
  /// <remarks>
  ///   A call is rooted for the engine by a handle its terminal frees, and a call refused before
  ///   it starts has no terminal. The call holds its marshaller, so a marshaller that survives a
  ///   collection means the call is still rooted.
  /// </remarks>
  [Test]
  public async Task ACallRefusedOverItsMetadataLeavesNothingRooted()
  {
    await using var channel = Channel();

    var marshaller = RefusedOverItsMetadata(channel);
    GC.Collect();
    GC.WaitForPendingFinalizers();
    GC.Collect();

    Assert.That(marshaller.IsAlive,
                Is.False,
                "the refused call is still rooted, and its marshaller with it");
  }

  // Not inlined, so no local of the test's own frame keeps the marshaller alive.
  [MethodImpl(MethodImplOptions.NoInlining)]
  private static WeakReference RefusedOverItsMetadata(NativeChannel channel)
  {
    var marshaller = Marshallers.Create<EchoReply>(message => message.ToByteArray(),
                                                   EchoReply.Parser.ParseFrom);

    Assert.Throws<RpcException>(() => channel.StartCall("/armonik.transport.ffi.test.Echo/Say",
                                                        new Metadata
                                                        {
                                                          {
                                                            "grpc-timeout", "1S"
                                                          },
                                                        },
                                                        marshaller));

    return new WeakReference(marshaller);
  }

  /// <summary>An endpoint the engine will not dial is the caller's argument, not a state this
  /// binding lost its footing in.</summary>
  /// <remarks>
  ///   The distinction is the one a caller can act on, and it is what the two exception types
  ///   say: a different endpoint is worth trying, a runtime that has gone is not. One
  ///   `InvalidOperationException` for both would say neither, and `Channel` documents
  ///   `ArgumentException` for this one.
  /// </remarks>
  [Test]
  public void AnEndpointTheEngineWillNotDialIsTheCallersArgument()
    => Assert.That(() => Runtime.Channel("ftp://127.0.0.1:1"),
                   Throws.ArgumentException.With.Message.Contains("is not a scheme this connector dials"),
                   "the engine's reason reaches the caller");

  /// <summary>A document the engine refuses names the key it refused, read from .NET.</summary>
  /// <remarks>Through the native entry point, because this binding checks its options before they
  /// leave it, so only a raw document reaches the engine's refusal.</remarks>
  [Test]
  public unsafe void ARefusedDocumentNamesItsKey()
  {
    var       endpoint = Encoding.UTF8.GetBytes("http://127.0.0.1:1");
    var       json     = Encoding.UTF8.GetBytes("{\"Grpc\":{\"Host\":{\"Receive\":{\"Window\":\"2\"}}}}");
    ak_status status;
    ak_error  error   = default;
    ulong     channel = 0;
    fixed (byte* pinnedEndpoint = endpoint)
    fixed (byte* pinned = json)
    {
      status = NativeMethods.ak_channel_create(Runtime.Handle,
                                               ak_bytes_in.Borrow(pinnedEndpoint,
                                                                  endpoint.Length),
                                               ak_bytes_in.Borrow(pinned,
                                                                  json.Length),
                                               &channel,
                                               &error);
    }

    var kind    = error.kind;
    var why     = error.Take();
    var created = channel;
    Assert.Multiple(() =>
                    {
                      Assert.That(status,
                                  Is.EqualTo(ak_status.AK_STATUS_INVALID_ARG));
                      Assert.That(kind,
                                  Is.EqualTo(ak_error_kind.AK_ERROR_CONFIG));
                      Assert.That(why,
                                  Does.Contain("Grpc.Host.Receive.Window"));
                      Assert.That(created,
                                  Is.Zero);
                    });
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
                Is.EqualTo(ak_channel_state.AK_CHANNEL_OPEN));

    await Client(channel)
          .SayAsync(new EchoRequest
                    {
                      Text = "state",
                    })
          .ResponseAsync.ConfigureAwait(false);

    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    // The native half is gone: the channel is released, and its calls settled before the
    // release, so the engine reclaimed the handle there.
    Assert.Multiple(() =>
                    {
                      Assert.That(channel.NativeState,
                                  Is.EqualTo(ak_channel_state.AK_CHANNEL_NONE));
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

  /// <summary>The bytes the runtime has lent out, or nothing when it answers no usage.</summary>
  private static unsafe ulong? Usage(ulong runtime)
  {
    ak_memory_usage usage;
    return NativeMethods.ak_runtime_memory_usage(runtime,
                                                 &usage,
                                                 null) == ak_status.AK_STATUS_OK
             ? usage.bytes_used
             : null;
  }
}
