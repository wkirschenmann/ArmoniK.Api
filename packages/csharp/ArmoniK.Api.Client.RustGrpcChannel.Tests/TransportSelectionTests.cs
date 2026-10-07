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
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.Submitter;

using Grpc.Core;
using Grpc.Net.Client;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>ArmoniK.Api.Client chooses its transport by its own options.</summary>
/// <remarks>
///   Not a <see cref="RuntimeFixture" />: the engine here is the one the client's factory owns, and the
///   fixture gives it back after each test, since a process holds one runtime at a time.
/// </remarks>
[TestFixture]
public class TransportSelectionTests
{
  private const string EnvironmentPrefix = "ArmoniK__Client__Grpc__";

  private EchoServerProcess? server_;

  private string Endpoint { get; set; } = string.Empty;

  [OneTimeSetUp]
  public void StartTheEchoServer()
  {
    server_  = EchoServerProcess.Start();
    Endpoint = server_.Endpoint;
  }

  [OneTimeTearDown]
  public void StopTheEchoServer()
    => server_?.Dispose();

  [TearDown]
  public Task GiveTheEngineBack()
    => NativeChannelFactory.Instance.ShutdownAsync();

  private static Echo.EchoClient Client(ChannelBase channel)
    => new(channel.CreateCallInvoker());

  /// <summary>The default is the managed transport, and what it makes is what it always made.</summary>
  [Test]
  public void ManagedIsTheDefaultAndGivesAGrpcChannel()
  {
    var options = new GrpcClient
                  {
                    Endpoint = Endpoint,
                  };

    Assert.Multiple(() =>
                    {
                      Assert.That(options.Transport,
                                  Is.EqualTo(ClientTransport.Managed));
                      Assert.That(GrpcChannelFactory.CreateChannelBase(options),
                                  Is.InstanceOf<GrpcChannel>());
                      Assert.That(GrpcChannelFactory.CreateChannel(options),
                                  Is.InstanceOf<GrpcChannel>());
                    });
  }

  /// <summary>CreateChannel returns a GrpcChannel whatever the option says, and does not start the engine.</summary>
  [Test]
  public async Task CreateChannelIgnoresTheNativeTransport()
  {
    var options = new GrpcClient
                  {
                    Endpoint  = Endpoint,
                    Transport = ClientTransport.Native,
                  };

    using var channel = GrpcChannelFactory.CreateChannel(options);
    Assert.That(channel,
                Is.InstanceOf<GrpcChannel>());

    // A process holds one runtime, so this fails if the call above started one.
    var runtime = NativeRuntime.Create();
    await runtime.DisposeAsync()
                 .ConfigureAwait(false);
  }

  /// <summary>Options that are null are refused by name, whichever transport they name.</summary>
  [Test]
  public void NullOptionsAreRefused()
    => Assert.That(() => GrpcChannelFactory.CreateChannelBase(null!),
                   Throws.ArgumentNullException);

  /// <summary>The native transport gives a native channel, which carries calls.</summary>
  [Test]
  public async Task NativeGivesAChannelOfTheEngineThatCalls()
  {
    var options = new GrpcClient
                  {
                    Endpoint  = Endpoint,
                    Transport = ClientTransport.Native,
                  };

    var channel = GrpcChannelFactory.CreateChannelBase(options);
    await using var native = (NativeChannel)channel;

    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "native",
                                })
                      .ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("native"));
  }

  /// <summary>The managed transport accepts TLS options beside an http endpoint, ignoring them, and so does the native one.</summary>
  [Test]
  public async Task TlsOptionsBesideAClearEndpointAreIgnored()
  {
    var options = new GrpcClient
                  {
                    Endpoint              = Endpoint,
                    Transport             = ClientTransport.Native,
                    AllowUnsafeConnection = true,
                    CertPem               = "client.pem",
                    KeyPem                = "client.key",
                    OverrideTargetName    = "server.test",
                  };

    Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                                  new GrpcClient())
                                                                       .Encode()),
                Does.Not.Contain("Tls"));

    await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "clear",
                                })
                      .ResponseAsync.ConfigureAwait(false);
    Assert.That(reply.Text,
                Is.EqualTo("clear"));
  }

  /// <summary>The TLS options of an https endpoint are translated.</summary>
  [Test]
  public void TlsOptionsBesideASecureEndpointAreTranslated()
    => Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient
                                                                                     {
                                                                                       Endpoint              = "HTTPS://server.test:5001",
                                                                                       AllowUnsafeConnection = true,
                                                                                     },
                                                                                     new GrpcClient())
                                                                          .Encode()),
                   Does.Contain(@"""Unverified"""));

  /// <summary>An initial backoff past the default maximum travels with the maximum, raised to it.</summary>
  [Test]
  public async Task TheBackoffsAreTranslatedAsAPair()
  {
    var options = new GrpcClient
                  {
                    Endpoint       = Endpoint,
                    Transport      = ClientTransport.Native,
                    InitialBackOff = TimeSpan.FromSeconds(10),
                  };

    Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                                  new GrpcClient())
                                                                       .Encode()),
                Does.Contain(@"""InitialBackoffSeconds"":10")
                    .And.Contain(@"""MaxBackoffSeconds"":10"));

    // The engine refuses a maximum below the initial one, so creating the channel fails if only
    // the initial one is sent.
    await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
  }

  /// <summary>A backoff that is not stated is left to the engine's sources, unless the stated one would pass it.</summary>
  [Test]
  public void AnUnstatedBackoffIsSentOnlyWhenTheStatedOnePassesIt()
  {
    string Encoded(GrpcClient options)
      => System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                           new GrpcClient())
                                                                .Encode());

    Assert.Multiple(() =>
                    {
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            InitialBackOff = TimeSpan.FromSeconds(2),
                                          }),
                                  Does.Contain(@"""InitialBackoffSeconds"":2")
                                      .And.Not.Contain("MaxBackoffSeconds"),
                                  "an initial one under the default maximum travels alone");
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            MaxBackOff = TimeSpan.FromSeconds(30),
                                          }),
                                  Does.Contain(@"""MaxBackoffSeconds"":30")
                                      .And.Not.Contain("InitialBackoffSeconds"),
                                  "a maximum over the default initial one travels alone");
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            MaxBackOff = TimeSpan.FromSeconds(0.5),
                                          }),
                                  Does.Contain(@"""MaxBackoffSeconds"":0.5")
                                      .And.Contain(@"""InitialBackoffSeconds"":0.5"),
                                  "an initial one above a stated maximum is lowered to it");
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            InitialBackOff = TimeSpan.FromSeconds(10),
                                            MaxBackOff     = TimeSpan.FromSeconds(3),
                                          }),
                                  Does.Contain(@"""InitialBackoffSeconds"":10")
                                      .And.Contain(@"""MaxBackoffSeconds"":10"),
                                  "two stated bounds that cross are sent with the maximum raised");
                    });
  }

  /// <summary>Two channels are open on the engine the factory owns, and after a shutdown the next channel starts another.</summary>
  [Test]
  public async Task TwoChannelsAreOpenOnOneEngineAndAShutdownLetsANewOneStart()
  {
    var options = new GrpcClient
                  {
                    Endpoint  = Endpoint,
                    Transport = ClientTransport.Native,
                  };

    await using (var first = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options))
    await using (var second = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options))
    {
      Assert.That(first,
                  Is.Not.SameAs(second));
    }

    await NativeChannelFactory.Instance.ShutdownAsync()
                              .ConfigureAwait(false);

    await using var again = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    var reply = await Client(again)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "again",
                                })
                      .ResponseAsync.ConfigureAwait(false);
    Assert.That(reply.Text,
                Is.EqualTo("again"));
  }

  /// <summary>The engine reads the environment under the client's prefix, the endpoint and a channel default alike.</summary>
  [Test]
  public async Task TheEngineReadsTheEnvironmentUnderTheClientsPrefix()
  {
    Environment.SetEnvironmentVariable(EnvironmentPrefix + "Endpoint",
                                       Endpoint);
    Environment.SetEnvironmentVariable(EnvironmentPrefix + "ChannelDefaults__Grpc__Host__Receive__Window",
                                       "9");
    try
    {
      await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(new GrpcClient
                                                                                    {
                                                                                      Transport = ClientTransport.Native,
                                                                                    });

      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(9),
                  "the window the engine settled from the environment, read back");

      var reply = await Client(channel)
                        .SayAsync(new EchoRequest
                                  {
                                    Text = "environment",
                                  })
                        .ResponseAsync.ConfigureAwait(false);
      Assert.That(reply.Text,
                  Is.EqualTo("environment"),
                  "the channel dialled the engine's Endpoint");
    }
    finally
    {
      Environment.SetEnvironmentVariable(EnvironmentPrefix + "Endpoint",
                                         null);
      Environment.SetEnvironmentVariable(EnvironmentPrefix + "ChannelDefaults__Grpc__Host__Receive__Window",
                                         null);
    }
  }

  /// <summary>The command line the client hands the engine is read under the same prefix, over the environment.</summary>
  [Test]
  public async Task TheEngineReadsTheCommandLineOverTheEnvironment()
  {
    Environment.SetEnvironmentVariable(EnvironmentPrefix + "ChannelDefaults__Grpc__Host__Receive__Window",
                                       "9");
    try
    {
      var channel = NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                                {
                                                                  Endpoint  = Endpoint,
                                                                  Transport = ClientTransport.Native,
                                                                },
                                                                new[]
                                                                {
                                                                  "--ArmoniK:Client:Grpc:ChannelDefaults:Grpc:Host:Receive:Window=6",
                                                                });
      await using var native = (NativeChannel)channel;

      Assert.That(native.DeliveryCredits,
                  Is.EqualTo(6));
    }
    finally
    {
      Environment.SetEnvironmentVariable(EnvironmentPrefix + "ChannelDefaults__Grpc__Host__Receive__Window",
                                         null);
    }
  }

  /// <summary>Only the client's prefix is read: another's names are not options of the engine.</summary>
  [Test]
  public async Task OnlyTheClientsPrefixIsRead()
  {
    Environment.SetEnvironmentVariable("GrpcClient__ChannelDefaults__Grpc__Host__Receive__Window",
                                       "9");
    try
    {
      await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(new GrpcClient
                                                                                    {
                                                                                      Endpoint  = Endpoint,
                                                                                      Transport = ClientTransport.Native,
                                                                                    });

      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(4));
    }
    finally
    {
      Environment.SetEnvironmentVariable("GrpcClient__ChannelDefaults__Grpc__Host__Receive__Window",
                                         null);
    }
  }

  /// <summary>A command line the provider refuses is refused when the caller gave it.</summary>
  [Test]
  public void AnExplicitCommandLineTheProviderRefusesIsRefused()
    => Assert.That(() => NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                                     {
                                                                       Endpoint = Endpoint,
                                                                     },
                                                                     new[]
                                                                     {
                                                                       "-x=1",
                                                                     }),
                   Throws.InstanceOf<FormatException>());

  /// <summary>A channel is not opened while the engine is shutting down, and the engine starts again once it has stopped.</summary>
  [Test]
  public async Task AChannelIsRefusedWhileTheEngineIsShuttingDown()
  {
    var options = new GrpcClient
                  {
                    Endpoint  = Endpoint,
                    Transport = ClientTransport.Native,
                  };
    var first = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    await using (first)
    {
      var stopping = NativeChannelFactory.Instance.ShutdownAsync();
      if (!stopping.IsCompleted)
      {
        Assert.That(() => GrpcChannelFactory.CreateChannelBase(options),
                    Throws.InstanceOf<InvalidOperationException>()
                          .With.Message.Contains("shutting down"));
      }

      await stopping.ConfigureAwait(false);
    }

    await using var next = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    Assert.That(next.DeliveryCredits,
                Is.GreaterThan(0));
  }

  /// <summary>Options that ask for no bound where a default sets one are named, since the engine cannot turn them off.</summary>
  [Test]
  public void OptionsThatCannotTurnADefaultOffAreNamed()
  {
    var options = new GrpcClient
                  {
                    KeepAliveTime = System.Threading.Timeout.InfiniteTimeSpan,
                    MaxIdleTime   = TimeSpan.Zero,
                  };

    Assert.That(NativeClientOptions.CannotBeDisabled(options),
                Is.EqualTo(new[]
                           {
                             nameof(GrpcClient.KeepAliveTime),
                             nameof(GrpcClient.MaxIdleTime),
                           }));
  }

  /// <summary>The proxy words and a P12 bundle are translated as the managed transport reads them.</summary>
  [Test]
  public void TheProxyWordsAndAP12BundleAreTranslated()
  {
    string Encoded(GrpcClient options)
      => System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                           new GrpcClient())
                                                                .Encode());

    Assert.Multiple(() =>
                    {
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            Proxy = "none",
                                          }),
                                  Does.Contain(@"""Proxy"":{""None"":"));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            Proxy = "System",
                                          }),
                                  Does.Contain(@"""Proxy"":{""System"":"));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            CertP12 = "client.p12",
                                            CertPem = "ignored.pem",
                                            KeyPem  = "ignored.key",
                                          }),
                                  Does.Contain(@"""P12"":{""Path"":""client.p12""}")
                                      .And.Not.Contain("ignored"));
                    });
  }

  /// <summary>Against the floor, only what the options state beyond it is translated.</summary>
  [Test]
  public void OnlyWhatTheOptionsStateBeyondTheirDefaultsIsTranslated()
  {
    var stated = new GrpcClient
                 {
                   AllowUnsafeConnection = true,
                   CertPem               = "client.pem",
                   KeyPem                = "client.key",
                   Proxy                 = "http://proxy.test:3128",
                   ProxyUsername         = "user",
                   MaxAttempts           = 3,
                   RequestTimeout        = TimeSpan.FromSeconds(7),
                 };

    Assert.Multiple(() =>
                    {
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient(),
                                                                                                    new GrpcClient())
                                                                                         .Encode()),
                                  Does.Not.Contain("MaxAttempts")
                                      .And.Not.Contain("IdleSeconds")
                                      .And.Not.Contain("IdleTimeoutSeconds"),
                                  "an option left at its default is left to the engine's sources");
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(stated,
                                                                                                    new GrpcClient())
                                                                                         .Encode()),
                                  Does.Contain(@"""Unverified""")
                                      .And.Contain(@"""Pem"":{""Certificate"":""client.pem"",""Key"":""client.key""}")
                                      .And.Contain(@"""Url"":{""Address"":""http://proxy.test:3128"",""Username"":""user""}")
                                      .And.Contain(@"""MaxAttempts"":3")
                                      .And.Contain(@"""DefaultDeadlineSeconds"":7"));
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient(),
                                                                                                    null)
                                                                                         .Encode()),
                                  Does.Contain(@"""IdleSeconds"":30")
                                      .And.Contain(@"""MaxAttempts"":5")
                                      .And.Contain(@"""IdleTimeoutSeconds"":300"),
                                  "with no floor, the defaults of GrpcClient are translated");
                    });
  }
}
