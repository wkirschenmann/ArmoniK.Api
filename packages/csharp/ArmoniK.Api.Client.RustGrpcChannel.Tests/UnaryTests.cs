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
using System.Linq;
using System.Net;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;

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
  private WebApplication? server_;
  private string endpoint_ = string.Empty;
  private NativeRuntime? runtime_;

  [OneTimeSetUp]
  public async Task StartServer()
  {
    var builder = WebApplication.CreateBuilder();
    // 127.0.0.1 and not localhost: Kestrel refuses a dynamic port on the latter, and the engine
    // under test dials plain HTTP/2 with no upgrade, so the listener must speak it outright.
    builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback,
                                                               0,
                                                               listen => listen.Protocols = HttpProtocols.Http2));
    builder.Services.AddGrpc();

    server_ = builder.Build();
    server_.MapGrpcService<EchoService>();
    await server_.StartAsync()
                 .ConfigureAwait(false);

    endpoint_ = server_.Urls.GetEnumerator() is var urls && urls.MoveNext()
                  ? urls.Current
                  : throw new InvalidOperationException("the test server bound no address");
    runtime_ = NativeRuntime.Start(workerThreads: 2);
  }

  [OneTimeTearDown]
  public async Task StopServer()
  {
    runtime_?.Dispose();
    if (server_ is not null)
    {
      await server_.StopAsync()
                   .ConfigureAwait(false);
      await server_.DisposeAsync()
                   .ConfigureAwait(false);
    }
  }

  private Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());

  private NativeChannel Channel()
    => runtime_!.Channel(endpoint_);

  [Test]
  public void TheAbiVersionIsTheOneThisBindingSpeaks()
    => Assert.That(NativeRuntime.LibraryAbiVersion,
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
    using var runtime = NativeRuntime.Start(workerThreads: 2,
                                            memoryCeiling: 128 * 1024);
    using var channel = runtime.Channel(endpoint_);
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
