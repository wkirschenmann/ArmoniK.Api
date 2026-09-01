using System;
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
    // Start() refuses a library that does not match, so reaching the fixture proves it.
    => Assert.That(runtime_,
                   Is.Not.Null);

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
  public void ACancelledCallEndsWithoutWaitingForTheServer()
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

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseAsync.ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  [Test]
  public async Task SeveralCallsShareOneChannel()
  {
    using var channel = Channel();
    var client = Client(channel);

    var running = new Task<EchoReply>[8];
    for (var index = 0; index < running.Length; index++)
    {
      var text = $"call-{index}";
      running[index] = client.SayAsync(new EchoRequest
                                       {
                                         Text = text,
                                       })
                             .ResponseAsync;
    }

    var replies = await Task.WhenAll(running)
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
  public void AChannelThatIsReleasedTakesNoNewCall()
  {
    var channel = Channel();
    channel.Dispose();

    Assert.Throws<RpcException>(() => Client(channel)
                                  .Say(new EchoRequest
                                       {
                                         Text = "x",
                                       }));
  }
}
