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
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.Submitter;

using Grpc.Core;
using Grpc.Net.Client;

using Microsoft.Extensions.Configuration;

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
                                                                                  true)
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
                                                                                     true)
                                                                          .Encode()),
                   Does.Contain(@"""Server"":""Unverified"""));

  /// <summary>Only the backoff that is set is sent, and the engine checks the pair once the options are merged.</summary>
  [Test]
  public void OnlyTheBackoffThatIsSetIsSent()
  {
    string Encoded(GrpcClient options)
      => System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                           true)
                                                                .Encode());

    Assert.Multiple(() =>
                    {
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            InitialBackOff = TimeSpan.FromSeconds(10),
                                          }),
                                  Does.Contain(@"""InitialBackoffSeconds"":10")
                                      .And.Not.Contain("MaxBackoffSeconds"));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            MaxBackOff = TimeSpan.FromSeconds(0.5),
                                          }),
                                  Does.Contain(@"""MaxBackoffSeconds"":0.5")
                                      .And.Not.Contain("InitialBackoffSeconds"));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            InitialBackOff = TimeSpan.FromSeconds(10),
                                            MaxBackOff     = TimeSpan.FromSeconds(3),
                                          }),
                                  Does.Contain(@"""InitialBackoffSeconds"":10")
                                      .And.Contain(@"""MaxBackoffSeconds"":3"),
                                  "two bounds that cross are sent as they are set");
                    });
  }

  /// <summary>An initial backoff above the maximum the options hold once merged is refused naming both keys, and one under it is not.</summary>
  [Test]
  public async Task AnInitialBackoffAboveTheMergedMaximumIsRefusedNamingBothKeys()
  {
    var options = new GrpcClient
                  {
                    Endpoint       = Endpoint,
                    Transport      = ClientTransport.Native,
                    InitialBackOff = TimeSpan.FromSeconds(10),
                  };

    // The defaults of GrpcClient state a maximum of 5 s, which the initial backoff passes.
    Assert.That(() => GrpcChannelFactory.CreateChannelBase(options),
                Throws.InstanceOf<ArgumentException>()
                      .With.Message.Contains("Grpc.Retry.InitialBackoffSeconds")
                      .And.Message.Contains("Grpc.Retry.MaxBackoffSeconds")
                      .And.Message.Contains("incoherent"));
    await NativeChannelFactory.Instance.ShutdownAsync()
                              .ConfigureAwait(false);

    // The environment's maximum of 60 s is in the options by the time the channel is made.
    const string name = EnvironmentPrefix + "ChannelDefaults__Grpc__Retry__MaxBackoffSeconds";
    Environment.SetEnvironmentVariable(name,
                                       "60");
    try
    {
      await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
  }

  /// <summary>Incoherent defaults in the environment start the engine, and only a channel that keeps them is refused.</summary>
  [Test]
  public void IncoherentDefaultsStartTheEngineAndRefuseAChannelThatKeepsThem()
  {
    const string name = EnvironmentPrefix + "ChannelDefaults__Grpc__Rate__Limit__Calls";
    Environment.SetEnvironmentVariable(name,
                                       "5");
    try
    {
      var options = new GrpcClient
                    {
                      Endpoint  = Endpoint,
                      Transport = ClientTransport.Native,
                    };
      Assert.That(() => GrpcChannelFactory.CreateChannelBase(options),
                  Throws.InstanceOf<ArgumentException>()
                        .With.Message.Contains("Grpc.Rate.Limit.Calls")
                        .And.Message.Contains("Grpc.Rate.Limit.PerSeconds"),
                  "the runtime was created, and the channel is refused");
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
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

  /// <summary>A value the caller set to its default overrides the environment, and one left unset keeps what the environment says.</summary>
  [Test]
  public async Task AValueSetToItsDefaultOverridesTheEnvironmentAndAnUnsetOneKeepsIt()
  {
    // A deadline of a nanosecond ends every call before it is answered, so what the channel does
    // says which of the two the engine read.
    const string name = EnvironmentPrefix + "ChannelDefaults__Grpc__DefaultDeadlineSeconds";
    Environment.SetEnvironmentVariable(name,
                                       "1e-9");
    try
    {
      await using (var kept = (NativeChannel)GrpcChannelFactory.CreateChannelBase(new GrpcClient
                                                                                  {
                                                                                    Endpoint  = Endpoint,
                                                                                    Transport = ClientTransport.Native,
                                                                                  }))
      {
        Assert.That(async () => await Client(kept)
                                      .SayAsync(new EchoRequest
                                                {
                                                  Text = "kept",
                                                })
                                      .ResponseAsync.ConfigureAwait(false),
                    Throws.InstanceOf<RpcException>()
                          .With.Property("StatusCode")
                          .EqualTo(StatusCode.DeadlineExceeded),
                    "unset: the environment's deadline applies");
      }

      await using var overridden = (NativeChannel)GrpcChannelFactory.CreateChannelBase(new GrpcClient
                                                                                       {
                                                                                         Endpoint       = Endpoint,
                                                                                         Transport      = ClientTransport.Native,
                                                                                         RequestTimeout = System.Threading.Timeout.InfiniteTimeSpan,
                                                                                       });
      var reply = await Client(overridden)
                        .SayAsync(new EchoRequest
                                  {
                                    Text = "overridden",
                                  })
                        .ResponseAsync.ConfigureAwait(false);
      Assert.That(reply.Text,
                  Is.EqualTo("overridden"),
                  "set to its default: it turns the environment's deadline off");
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
  }

  /// <summary>An option is set by the keys of the configuration it was bound from, and only by them.</summary>
  [Test]
  public void BindingFromAConfigurationSetsExactlyTheKeysItHolds()
  {
    var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["GrpcClient:Endpoint"]              = "http://server.test:5001",
                                                                           ["GrpcClient:MaxAttempts"]           = "5",
                                                                           ["GrpcClient:KeepAliveTime"]         = "00:00:10",
                                                                           ["GrpcClient:CertPem"]               = "client.pem",
                                                                           ["GrpcClient:AllowUnsafeConnection"] = "false",
                                                                         })
                                                  .Build();

    var options = configuration.GetRequiredSection(GrpcClient.SettingSection)
                               .Get<GrpcClient>()!;

    Assert.Multiple(() =>
                    {
                      foreach (var name in new[]
                                           {
                                             nameof(GrpcClient.MaxAttempts),
                                             nameof(GrpcClient.KeepAliveTime),
                                             nameof(GrpcClient.CertPem),
                                             nameof(GrpcClient.AllowUnsafeConnection),
                                           })
                      {
                        Assert.That(options.IsSet(name),
                                    Is.True,
                                    name);
                      }

                      foreach (var name in new[]
                                           {
                                             nameof(GrpcClient.KeyPem),
                                             nameof(GrpcClient.KeepAliveTimeInterval),
                                             nameof(GrpcClient.MaxIdleTime),
                                             nameof(GrpcClient.InitialBackOff),
                                             nameof(GrpcClient.MaxBackOff),
                                             nameof(GrpcClient.BackoffMultiplier),
                                             nameof(GrpcClient.RequestTimeout),
                                             nameof(GrpcClient.Proxy),
                                             nameof(GrpcClient.CaCert),
                                           })
                      {
                        Assert.That(options.IsSet(name),
                                    Is.False,
                                    name);
                      }

                      Assert.That(options.MaxAttempts,
                                  Is.EqualTo(5));
                      Assert.That(options.KeepAliveTime,
                                  Is.EqualTo(TimeSpan.FromSeconds(10)));
                    });
  }

  /// <summary>Assigning an option sets it, and creating or reading one does not.</summary>
  [Test]
  public void AssigningAnOptionSetsItAndReadingDoesNot()
  {
    var options = new GrpcClient();
    Assert.That(options.MaxAttempts,
                Is.EqualTo(5));
    Assert.That(options.IsSet(nameof(GrpcClient.MaxAttempts)),
                Is.False);

    // Another option is read in between: an assignment right after the read of the same option
    // is the shape the configuration binder gives an absent key, and is not recorded.
    Assert.That(options.BackoffMultiplier,
                Is.EqualTo(1.5));
    options.MaxAttempts = 5;
    Assert.That(options.IsSet(nameof(GrpcClient.MaxAttempts)),
                Is.True);
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

  /// <summary>A channel opened while the engine stops is refused, or opened on the next engine if it has stopped by then.</summary>
  [Test]
  public async Task AChannelOpenedWhileTheEngineStopsIsRefusedOrOpenedOnTheNextOne()
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
      try
      {
        // Either the refusal, or the engine had stopped by now and this one is a new engine's.
        await using var during = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
        Assert.That(stopping.IsCompleted,
                    Is.True,
                    "a channel opened while the engine was stopping");
      }
      catch (InvalidOperationException refused)
      {
        Assert.That(refused.Message,
                    Does.Contain("shutting down"));
      }

      await stopping.ConfigureAwait(false);
    }

    await using var next = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    Assert.That(next.DeliveryCredits,
                Is.GreaterThan(0));
  }

  /// <summary>Options that ask for no bound where a default sets one state it as zero, so that they turn it off.</summary>
  [Test]
  public async Task OptionsThatAskForNoBoundTurnTheDefaultOff()
  {
    var options = new GrpcClient
                  {
                    Endpoint      = Endpoint,
                    Transport     = ClientTransport.Native,
                    KeepAliveTime = System.Threading.Timeout.InfiniteTimeSpan,
                    MaxIdleTime   = TimeSpan.Zero,
                  };

    Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                                  true)
                                                                       .Encode()),
                Does.Contain(@"""IdleSeconds"":0")
                    .And.Contain(@"""IdleTimeoutSeconds"":0"));
    Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient
                                                                                  {
                                                                                    RequestTimeout = TimeSpan.Zero,
                                                                                  },
                                                                                  true)
                                                                       .Encode()),
                Does.Contain(@"""DefaultDeadlineSeconds"":0"));

    // The floor sets a keepalive with its interval, and the zero over it must not be refused
    // for the interval that stays.
    await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "off",
                                })
                      .ResponseAsync.ConfigureAwait(false);
    Assert.That(reply.Text,
                Is.EqualTo("off"));
  }

  /// <summary>The keepalive counts whole seconds, rounded up, so that a span that is positive is never none.</summary>
  [Test]
  public void TheKeepaliveIsSentInWholeSeconds()
  {
    var encoded = System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient
                                                                                    {
                                                                                      KeepAliveTime         = TimeSpan.FromMilliseconds(500),
                                                                                      KeepAliveTimeInterval = TimeSpan.FromMilliseconds(1500),
                                                                                    },
                                                                                    true)
                                                                         .Encode());
    Assert.That(encoded,
                Does.Contain(@"""IdleSeconds"":1")
                    .And.Contain(@"""IntervalSeconds"":2"));
  }

  /// <summary>An interval the engine cannot honour is carried on, and refused when the channel is made, naming the key.</summary>
  [Test]
  public void AnIntervalThatIsNotPositiveBesideAKeepaliveIsRefusedWhenTheChannelIsMade()
  {
    var options = new GrpcClient
                  {
                    Endpoint              = Endpoint,
                    Transport             = ClientTransport.Native,
                    KeepAliveTimeInterval = System.Threading.Timeout.InfiniteTimeSpan,
                  };

    Assert.That(() => GrpcChannelFactory.CreateChannelBase(options),
                Throws.InstanceOf<ArgumentException>()
                      .With.Message.Contains("IntervalSeconds has to be at least 1"));
  }

  /// <summary>Turning the keepalive off with an interval that is off too is what a caller writes, and is not refused.</summary>
  [Test]
  public async Task ABothInfiniteKeepaliveTurnsItOff()
  {
    var options = new GrpcClient
                  {
                    Endpoint              = Endpoint,
                    Transport             = ClientTransport.Native,
                    KeepAliveTime         = System.Threading.Timeout.InfiniteTimeSpan,
                    KeepAliveTimeInterval = System.Threading.Timeout.InfiniteTimeSpan,
                  };

    Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                                  true)
                                                                       .Encode()),
                Does.Contain(@"""IdleSeconds"":0")
                    .And.Not.Contain("IntervalSeconds"));
    await using var channel = (NativeChannel)GrpcChannelFactory.CreateChannelBase(options);
  }

  /// <summary>The proxy words and a P12 bundle are translated as the managed transport reads them.</summary>
  [Test]
  public void TheProxyWordsAndAP12BundleAreTranslated()
  {
    string Encoded(GrpcClient options)
      => System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(options,
                                                                           true)
                                                                .Encode());

    Assert.Multiple(() =>
                    {
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            Proxy = "none",
                                          }),
                                  Does.Contain(@"""Proxy"":""None"""));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            Proxy = "System",
                                          }),
                                  Does.Contain(@"""Proxy"":{""System"":"));
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            Proxy = "",
                                          }),
                                  Does.Contain(@"""Proxy"":{""System"":"),
                                  "an empty proxy set is the system's, over one an earlier source named");
                      Assert.That(Encoded(new GrpcClient
                                          {
                                            ProxyUsername = "user",
                                          }),
                                  Does.Not.Contain("Proxy"),
                                  "credentials alone say nothing of the proxy");
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

  /// <summary>Only the options the caller set are translated, a value equal to the default included.</summary>
  [Test]
  public void OnlyTheOptionsTheCallerSetAreTranslated()
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
                                                                                                    true)
                                                                                         .Encode()),
                                  Does.Not.Contain("MaxAttempts")
                                      .And.Not.Contain("IdleSeconds")
                                      .And.Not.Contain("IdleTimeoutSeconds"),
                                  "an option left alone is left to the engine's sources");
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient
                                                                                                    {
                                                                                                      MaxAttempts = 5,
                                                                                                      MaxIdleTime = TimeSpan.FromMinutes(5),
                                                                                                    },
                                                                                                    true)
                                                                                         .Encode()),
                                  Does.Contain(@"""MaxAttempts"":5")
                                      .And.Contain(@"""IdleTimeoutSeconds"":300")
                                      .And.Not.Contain("IdleSeconds"),
                                  "an option set to its default is translated");
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(stated,
                                                                                                    true)
                                                                                         .Encode()),
                                  Does.Contain(@"""Server"":""Unverified""")
                                      .And.Contain(@"""Pem"":{""Certificate"":""client.pem"",""Key"":""client.key""}")
                                      .And.Contain(@"""Url"":{""Address"":""http://proxy.test:3128"",""Username"":""user""}")
                                      .And.Contain(@"""MaxAttempts"":3")
                                      .And.Contain(@"""DefaultDeadlineSeconds"":7"));
                      Assert.That(System.Text.Encoding.UTF8.GetString(NativeClientOptions.Translate(new GrpcClient(),
                                                                                                    false)
                                                                                         .Encode()),
                                  Does.Contain(@"""IdleSeconds"":30")
                                      .And.Contain(@"""MaxAttempts"":5")
                                      .And.Contain(@"""Codes"":""GrpcClient""")
                                      .And.Contain(@"""IdleTimeoutSeconds"":300"),
                                  "for the defaults of a runtime, every option is translated");
                    });
  }
}
