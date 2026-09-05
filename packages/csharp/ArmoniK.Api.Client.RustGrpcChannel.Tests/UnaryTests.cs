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

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class UnaryTests : RuntimeLeaseFixture
{
  private EchoServerProcess? server_;
  private string endpoint_ = string.Empty;

  [OneTimeSetUp]
  public void StartServer()
  {
    server_   = EchoServerProcess.Start();
    endpoint_ = server_.Endpoint;
    NativeRuntimeFactory.Configure(workerThreads: 2);
  }

  /// <summary>One test asks for a memory ceiling, and every other one runs without.</summary>
  protected override void ArmTheNextTest()
    => NativeRuntimeFactory.Configure(workerThreads: 2);

  [OneTimeTearDown]
  public void StopServer()
    => server_?.Dispose();

  private Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());

  private NativeChannel Channel()
    => NativeRuntimeFactory.Channel(endpoint_);

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
    => Assert.That(NativeRuntimeFactory.LibraryAbiVersion,
                   Is.EqualTo(1),
                   "the loaded library speaks the ABI this binding was written against");

  [Test]
  public async Task AUnaryCallReachesTheServerAndComesBack()
  {
    using var channel = Channel();
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
    using var channel = Channel();
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
  public void ABlockingCallAnswersTheSameWay()
  {
    using var channel = Channel();

    var reply = Client(channel)
      .Say(new EchoRequest
           {
             Text = "blocking",
           });

    Assert.That(reply.Text,
                Is.EqualTo("blocking"));
  }

  [Test]
  public void AServerThatRefusesComesBackAsThatStatus()
  {
    using var channel = Channel();

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
    using var channel = Channel();
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
  public async Task SeveralCallsShareOneChannel()
  {
    using var channel = Channel();
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
    using var channel = Channel();
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

    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo(RuntimeDisposeState.Absent),
                "no other channel is open");
    NativeRuntimeFactory.Configure(workerThreads: 2,
                                   memoryCeiling: 128 * 1024);
    using var channel = Channel();
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
    using var channel = NativeRuntimeFactory.Channel(endpoint_,
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

  [Test]
  public void AWindowOfZeroIsRefusedBeforeAnythingIsOpened()
    => Assert.Throws<ArgumentOutOfRangeException>(() => NativeRuntimeFactory.Channel(endpoint_,
                                                                                     deliveryCredits: 0));

  [Test]
  public async Task TheChannelsTwoHalvesAgreeOnItsState()
  {
    var keepsAlive = Channel();
    try
    {
      await TheTwoHalves(keepsAlive)
        .ConfigureAwait(false);
    }
    finally
    {
      await keepsAlive.DisposeAsync()
                      .ConfigureAwait(false);
    }
  }

  private async Task TheTwoHalves(NativeChannel keepsAlive)
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

    Assert.That(keepsAlive.NativeState,
                Is.EqualTo(NativeMethods.AkChannelState.Open));
  }

  [Test]
  public async Task TheLastChannelReleasedIsTheOneThatTearsTheRuntimeDown()
  {
    var first = Channel();
    var second = Channel();

    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo(RuntimeDisposeState.Active),
                "one generation, two leases");

    await first.DisposeAsync()
               .ConfigureAwait(false);
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo(RuntimeDisposeState.Active),
                "a lease is still out, so nothing may shut down");

    await second.DisposeAsync()
                .ConfigureAwait(false);
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo(RuntimeDisposeState.Absent),
                "the last release awaited the destroy before its task completed");
  }

  [Test]
  public void AChannelThatIsReleasedTakesNoNewCall()
  {
    var channel = Channel();
    channel.Dispose();

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
