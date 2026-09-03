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

/// <summary>
///   A unary call from .NET, through the C ABI, to a gRPC server Kestrel serves.
/// </summary>
/// <remarks>
///   The server is grpc-dotnet's, so what these exercise is the binding and the native engine
///   against an implementation that owes them nothing.
/// </remarks>
[TestFixture]
public class UnaryTests
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

  /// <summary>
  ///   Checked after every test, because a leaked lease would otherwise surface as a hang in
  ///   some later one: the generation is torn down by the release that empties the set, so a
  ///   test that disposed its channels leaves the factory with nothing.
  /// </summary>
  [TearDown]
  public void EveryLeaseWentBack()
  {
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo("Absent"),
                "the test left no lease behind");
    // The options are the generation's, so a test that set its own does not leave them here.
    NativeRuntimeFactory.Configure(workerThreads: 2);
  }

  [OneTimeTearDown]
  public void StopServer()
    => server_?.Dispose();

  private Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());

  private NativeChannel Channel()
    => NativeRuntimeFactory.Channel(endpoint_);

  /// <summary>
  ///   The engine beside this host is built for the word size this host runs as.
  /// </summary>
  /// <remarks>
  ///   The build chooses the engine from `PlatformTarget` and the test host's architecture comes
  ///   from the same place, so a mismatch means one of the two was decided somewhere else. It
  ///   would otherwise surface as a `BadImageFormatException` from whichever P/Invoke ran first,
  ///   which says nothing about why.
  /// </remarks>
  [Test]
  public void TheEngineBesideThisHostMatchesItsWordSize()
  {
    var engine = Path.Combine(AppDomain.CurrentDomain.BaseDirectory,
                              "armonik_transport_ffi.dll");
    if (!File.Exists(engine))
    {
      Assert.Ignore("not a Windows build; the engine is an .so or a .dylib");
    }

    // The COFF machine field, at the offset the PE signature points to.
    using var file = File.OpenRead(engine);
    using var reader = new BinaryReader(file);
    file.Seek(0x3c,
              SeekOrigin.Begin);
    file.Seek(reader.ReadUInt32() + 4,
              SeekOrigin.Begin);

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

    // Bounded, because the name claims it does not wait for the server - and `Never` would
    // otherwise let a binding that waits pass, since cancellation ends it there too.
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

    EchoReply[] replies;
    try
    {
      replies = await Task.WhenAll(Array.ConvertAll(calls,
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

  /// <summary>
  ///   The ceiling bounds lent buffers, so several calls at once contend for it and the refusals
  ///   are real. Without the wait behind `BUDGET_BUSY` this fails rather than slows down.
  /// </summary>
  [Test]
  public async Task CallsWaitForRoomUnderAMemoryCeiling()
  {
    var text = new string('x',
                          100_000);

    // The ceiling belongs to the generation, and there is one generation for the process, so
    // this test owns the factory for its duration - which is also the only moment its options
    // may be set.
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo("Absent"),
                "no other channel is open");
    NativeRuntimeFactory.Configure(workerThreads: 2,
                                   memoryCeiling: 128 * 1024);
    using var channel = NativeRuntimeFactory.Channel(endpoint_);
    var client = Client(channel);

    // Issued from the pool and not from here: a send holds its buffer only between the lend and
    // the commit, and both run inline, so calls started one after another never meet.
    var calls = await Task.WhenAll(Enumerable.Range(0,
                                                    16)
                                             .Select(_ => Task.Run(() => client.SayAsync(new EchoRequest
                                                                                         {
                                                                                           Text = text,
                                                                                         }))))
                          .ConfigureAwait(false);

    try
    {
      var replies = await Task.WhenAll(Array.ConvertAll(calls,
                                                        call => call.ResponseAsync))
                              .ConfigureAwait(false);

      Assert.That(replies,
                  Has.All.Matches<EchoReply>(reply => reply.Text == text));
    }
    finally
    {
      foreach (var call in calls)
      {
        call.Dispose();
      }
    }
  }

  /// <summary>
  ///   The window is a channel option the host chooses, because the host is what holds the
  ///   payloads. The engine refuses an option it does not know, so a wrong name fails here.
  /// </summary>
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

  /// <summary>
  ///   The managed dispose state and the engine's own agree, which is what
  ///   <c>ChannelStateMatchesNative</c> asks: active means open, and a disposed channel's half
  ///   is closed rather than merely closing.
  /// </summary>
  [Test]
  public async Task TheChannelsTwoHalvesAgreeOnItsState()
  {
    // A second channel holds the generation, so the first one's half can be looked at after it
    // is released rather than vanishing with the runtime.
    var keepsAlive = NativeRuntimeFactory.Channel(endpoint_);
    var channel = NativeRuntimeFactory.Channel(endpoint_);
    Assert.That(channel.NativeState,
                Is.EqualTo("Open"));

    await Client(channel)
          .SayAsync(new EchoRequest
                    {
                      Text = "state",
                    })
          .ResponseAsync.ConfigureAwait(false);

    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    // Disposing settles this channel's calls first, so the engine has nothing left to drain and
    // reports closed rather than closing.
    Assert.That(channel.NativeState,
                Is.EqualTo("Closed"));

    await keepsAlive.DisposeAsync()
                    .ConfigureAwait(false);

    // The last release destroyed the generation, and a released channel's handle is reclaimed
    // with it - so neither half names a channel any more.
    Assert.That(channel.NativeState,
                Is.EqualTo("None"));
  }

  /// <summary>
  ///   A channel is the unit of borrowing: the runtime outlives every lease and is torn down by
  ///   the release that empties the set, whose task completes only once the destroy is done.
  /// </summary>
  [Test]
  public async Task TheLastChannelReleasedIsTheOneThatTearsTheRuntimeDown()
  {
    var first = NativeRuntimeFactory.Channel(endpoint_);
    var second = NativeRuntimeFactory.Channel(endpoint_);

    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo("Active"),
                "one generation, two leases");

    await first.DisposeAsync()
               .ConfigureAwait(false);
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo("Active"),
                "a lease is still out, so nothing may shut down");

    await second.DisposeAsync()
                .ConfigureAwait(false);
    Assert.That(NativeRuntimeFactory.State,
                Is.EqualTo("Absent"),
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
                                  Is.EqualTo(StatusCode.Internal));
                      Assert.That(thrown.Status.Detail,
                                  Does.Contain("HandleStale"),
                                  "refused for the released handle, not for something else");
                    });
  }
}
