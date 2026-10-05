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
using System.Text;

using ArmoniK.Api.Common.Utils;

using Microsoft.Extensions.Configuration;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What the generated vocabulary sends, and what it refuses to send.</summary>
[TestFixture]
public class ChannelOptionsTests
{
  private static string Encoded(ChannelOptions options)
    => Encoding.UTF8.GetString(options.Encode());

  private static IConfiguration Configuration(Dictionary<string, string?> values)
    => new ConfigurationBuilder().AddInMemoryCollection(values)
                                 .Build();

  // The delivery window and the send window, where the vocabulary nests them.
  private static GrpcOptions Windows(int? credits = null,
                                     int? sends   = null)
    => new()
       {
         Host = new HostOptions
                {
                  Sends = sends is null
                            ? null
                            : new SendOptions
                              {
                                Window = sends,
                              },
                  Receive = credits is null
                              ? null
                              : new ReceiveOptions
                                {
                                  Window = credits,
                                },
                },
       };

  /// <summary>An option nobody set is absent, not null and not a default spelled out.</summary>
  /// <remarks>
  ///   The engine reads an absent option as its own default, and every option has one - which is
  ///   what makes `{}` a valid configuration. A null would be a type error on the far side, since
  ///   the schema gives no option a null form.
  /// </remarks>
  [Test]
  public void AnOptionNobodySetIsAbsent()
    => Assert.That(Encoded(new ChannelOptions()),
                   Is.EqualTo("{}"));

  [Test]
  public void AnOptionThatIsSetIsSpelledAsTheSchemaNamesIt()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = new GrpcOptions
                                    {
                                      UserAgent = "test",
                                      Host      = Windows(4).Host,
                                    },
                           }),
                   Is.EqualTo(@"{""Grpc"":{""UserAgent"":""test"",""Host"":{""Receive"":{""Window"":4}}}}"));

  /// <summary>A group is an object, because the document is typed and structured.</summary>
  [Test]
  public void AGroupOfOptionsIsANestedObject()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Transport = new TransportOptions
                                         {
                                           ConnectTimeoutSeconds = 2.5,
                                         },
                           }),
                   Is.EqualTo(@"{""Transport"":{""ConnectTimeoutSeconds"":2.5}}"));

  /// <summary>A value the schema excludes is refused here, not by the engine.</summary>
  /// <remarks>
  ///   `ak_channel_create` answers a bad document with a status naming neither the option nor the
  ///   bound, so a caller who set a window to zero would learn only that the endpoint was refused.
  /// </remarks>
  [Test]
  public void AWindowOutsideItsRangeIsRefusedBeforeItIsSent()
    => Assert.That(() => new ChannelOptions
                         {
                           Grpc = Windows(0),
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("Window has to be at least 1 and at most 536870910"));

  [Test]
  public void AnEmptyUserAgentIsRefusedBeforeItIsSent()
    => Assert.That(() => new ChannelOptions
                         {
                           Grpc = new GrpcOptions
                                  {
                                    UserAgent = string.Empty,
                                  },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("at least 1 character long"));

  /// <summary>A group's own bounds are checked through the group that holds it.</summary>
  /// <remarks>Below a nanosecond, which the engine could round to a timeout of zero.</remarks>
  [TestCase(0.0,
            TestName = "{m}(zero)")]
  [TestCase(9.99e-10,
            TestName = "{m}(below a nanosecond)")]
  public void ATimeoutBelowANanosecondIsRefusedThroughItsGroup(double timeout)
    => Assert.That(() => new ChannelOptions
                         {
                           Transport = new TransportOptions
                                       {
                                         ConnectTimeoutSeconds = timeout,
                                       },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("ConnectTimeoutSeconds has to be at least 1E-09"));

  /// <summary>A `double` holds three values a JSON number cannot, and all three are refused.</summary>
  /// <remarks>
  ///   NaN satisfies every bound - each comparison against it is false - and System.Text.Json
  ///   refuses to write any of the three. Left to it, a caller who set one would get its
  ///   ArgumentException, which names neither the option nor what was wrong with the value.
  /// </remarks>
  [TestCase(double.NaN,
            TestName = "ANotFiniteTimeout_NaN")]
  [TestCase(double.PositiveInfinity,
            TestName = "ANotFiniteTimeout_Positive")]
  [TestCase(double.NegativeInfinity,
            TestName = "ANotFiniteTimeout_Negative")]
  public void ATimeoutThatIsNotAFiniteNumberIsRefused(double timeout)
    => Assert.That(() => new ChannelOptions
                         {
                           Transport = new TransportOptions
                                       {
                                         ConnectTimeoutSeconds = timeout,
                                       },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("ConnectTimeoutSeconds"));

  /// <summary>The size the schema refuses, refused here with the reason.</summary>
  /// <remarks>
  ///   Zero is a channel that can receive no message at all, and the schema's `minimum` says so.
  ///   The message repeats the bound, because a caller who reads the option's documentation and
  ///   sets zero has to be told what to set instead.
  /// </remarks>
  [Test]
  public void AMessageSizeOfZeroIsRefused()
    => Assert.That(() => new ChannelOptions
                         {
                           Grpc = new GrpcOptions
                                  {
                                    MaxReceiveMessageSize = 0,
                                  },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("MaxReceiveMessageSize has to be at least 1"));

  /// <summary>And the size the schema admits has no upper bound to run into.</summary>
  [Test]
  public void AMessageSizeAsLargeAsAnIntIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = new GrpcOptions
                                    {
                                      MaxReceiveMessageSize = int.MaxValue,
                                    },
                           }),
                   Is.EqualTo(@"{""Grpc"":{""MaxReceiveMessageSize"":2147483647}}"));

  /// <summary>A copy shares nothing with what it copied, one group down included.</summary>
  /// <remarks>
  ///   What a channel sizes its rings from and what it sends the engine have to be one number,
  ///   and a settable property read twice is two numbers if anything sets it in between - so the
  ///   factory reads a copy. A group left shared would be the same hole one level down.
  /// </remarks>
  [Test]
  public void ACopySharesNothingWithWhatItCopied()
  {
    var original = new ChannelOptions
                   {
                     Grpc = Windows(4),
                     Transport = new TransportOptions
                                 {
                                   ConnectTimeoutSeconds = 2.5,
                                 },
                   };

    var copy = new ChannelOptions(original);

    original.Grpc!.Host!.Receive!.Window      = 8;
    original.Transport!.ConnectTimeoutSeconds = 30;

    Assert.Multiple(() =>
                    {
                      Assert.That(copy.Grpc!.Host!.Receive!.Window,
                                  Is.EqualTo(4),
                                  "groups three deep are copied too");
                      Assert.That(copy.Transport!.ConnectTimeoutSeconds,
                                  Is.EqualTo(2.5),
                                  "the group is copied and not shared");
                    });
  }

  [Test]
  public void ACopyOfNothingIsRefused()
    => Assert.That(() => new ChannelOptions(null!),
                   Throws.TypeOf<ArgumentNullException>());

  /// <summary>An option named in two layers is settled by .NET before it arrives.</summary>
  /// <remarks>This binds the composition and reads no source itself, so .NET's order decides.</remarks>
  [Test]
  public void AnOptionNamedInTwoLayersTakesTheLatersValue()
  {
    var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["Section:Grpc:Host:Receive:Window"] = "4",
                                                                           ["Section:Grpc:UserAgent"]           = "first",
                                                                         })
                                                  .AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["Section:Grpc:UserAgent"] = "second",
                                                                         })
                                                  .Build();

    var options = configuration.GetRequiredValue<ChannelOptions>("Section");

    Assert.Multiple(() =>
                    {
                      Assert.That(options.Grpc?.UserAgent,
                                  Is.EqualTo("second"),
                                  "the later layer wins");
                      Assert.That(options.Grpc?.Host?.Receive?.Window,
                                  Is.EqualTo(4),
                                  "and an option only the earlier layer names still arrives");
                    });
  }

  /// <summary>A group is reached by the separator .NET already maps onto a section.</summary>
  /// <remarks>The repository relies on that mapping today with `GrpcClient__Endpoint`.</remarks>
  [Test]
  public void AnOptionSetOnlyInTheEnvironmentReachesTheDocument()
  {
    const string prefix = "AKRUSTTEST_";

    Environment.SetEnvironmentVariable(prefix + "Section__Grpc__Host__Sends__Window",
                                       "7");
    Environment.SetEnvironmentVariable(prefix + "Section__Transport__ConnectTimeoutSeconds",
                                       "2.5");

    try
    {
      var configuration = new ConfigurationBuilder().AddEnvironmentVariables(prefix)
                                                    .Build();

      var options = configuration.GetRequiredValue<ChannelOptions>("Section");

      // The document, because an option bound but not serialized is one the engine never sees.
      Assert.That(Encoded(options),
                  Is.EqualTo(@"{""Transport"":{""ConnectTimeoutSeconds"":2.5},""Grpc"":{""Host"":{""Sends"":{""Window"":7}}}}"));
    }
    finally
    {
      Environment.SetEnvironmentVariable(prefix + "Section__Grpc__Host__Sends__Window",
                                         null);
      Environment.SetEnvironmentVariable(prefix + "Section__Transport__ConnectTimeoutSeconds",
                                         null);
    }
  }

  /// <summary>A key set to null is unset, as it is to .NET's binder.</summary>
  /// <remarks>A JSON file's `null` arrives as one.</remarks>
  [Test]
  public void AKeySetToNullIsUnset()
  {
    var options = NativeRuntime.OptionsFrom(new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                                             {
                                                                                               ["Section:Grpc:UserAgent"]           = null,
                                                                                               ["Section:Grpc:Host:Receive:Window"] = "4",
                                                                                             })
                                                                      .Build(),
                                            "Section");

    Assert.That(Encoded(options),
                Is.EqualTo(@"{""Grpc"":{""Host"":{""Receive"":{""Window"":4}}}}"));
  }

  /// <summary>A group left empty is unset, as it is to .NET's binder.</summary>
  [Test]
  public void AGroupLeftEmptyIsUnset()
  {
    var options = NativeRuntime.OptionsFrom(new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                                             {
                                                                                               ["Section:Transport"]                = string.Empty,
                                                                                               ["Section:Grpc:Host:Receive:Window"] = "4",
                                                                                             })
                                                                      .Build(),
                                            "Section");

    Assert.That(options.Transport,
                Is.Null);
  }

  /// <summary>A section holding no key configures nothing, and is refused as a missing one is.</summary>
  [Test]
  public void ASectionHoldingNoKeyIsRefused()
    => Assert.That(() => NativeRuntime.OptionsFrom(new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                                                    {
                                                                                                      ["Section"] = string.Empty,
                                                                                                    })
                                                                             .Build(),
                                                   "Section"),
                   Throws.TypeOf<InvalidOperationException>()
                         .With.Message.Contains("carries no options"));

  /// <summary>A configuration key no option matches is dropped by .NET's binder.</summary>
  /// <remarks>
  ///   Bound as it comes, which is what the factory's door may not do: a misspelling reaches the
  ///   engine as an absent option, and an absent option is a default.
  /// </remarks>
  [Test]
  public void AConfigurationKeyNoOptionMatchesIsIgnoredByTheBinder()
  {
    var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["Section:DeliveryCredit"] = "4",
                                                                         })
                                                  .Build();

    var options = configuration.GetRequiredValue<ChannelOptions>("Section");

    Assert.That(Encoded(options),
                Is.EqualTo("{}"));
  }

  /// <summary>And the door refuses it, as `additionalProperties: false` refuses it in the
  /// document a Rust or C++ host hands over.</summary>
  /// <remarks>Needs no engine: the section is bound before anything native is reached.</remarks>
  [Test]
  public void AConfigurationKeyNoOptionMatchesIsRefusedByTheDoor()
  {
    var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["Section:DeliveryCredit"] = "4",
                                                                         })
                                                  .Build();

    var refused = Assert.Throws<InvalidOperationException>(() => NativeRuntime.OptionsFrom(configuration,
                                                                                          "Section"));

    Assert.That(refused?.Message,
                Does.Contain("DeliveryCredit"),
                "the message names the key, since finding it is the whole difficulty");
  }

  [Test]
  public void AValueOnEitherEdgeOfItsRangeIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = Windows(1,
                                            536870910),
                           }),
                   Is.EqualTo(@"{""Grpc"":{""Host"":{""Sends"":{""Window"":536870910},""Receive"":{""Window"":1}}}}"));

  /// <summary>An alternative is an object whose one key names it, as the engine reads a Rust enum.</summary>
  /// <remarks>A field left unset is absent, as an option is; one carrying nothing is `true`.</remarks>
  [Test]
  public void AnAlternativeIsAnObjectWhoseOneKeyNamesIt()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Transport = new TransportOptions
                                         {
                                           Tls = new TlsOptions
                                                 {
                                                   Server = new ServerVerification.CaStore(new StoreSearch.Thumbprint("ab"),
                                                                                           StoreLocation.LocalMachine),
                                                   Client = new ClientCertificate.P12("me.p12"),
                                                 },
                                           Proxy = new ProxyOptions.None(),
                                         },
                           }),
                   Is.EqualTo(@"{""Transport"":{""Tls"":{""Server"":{""CaStore"":{""Location"":""LocalMachine"",""Find"":{""Thumbprint"":""ab""}}},""Client"":{""P12"":{""Path"":""me.p12""}}},""Proxy"":{""None"":true}}}"));

  /// <summary>A field's bounds are checked through the group holding its alternative.</summary>
  [Test]
  public void AFieldOutsideItsRangeIsRefusedThroughItsGroup()
    => Assert.That(() => new ChannelOptions
                         {
                           Transport = new TransportOptions
                                       {
                                         Tls = new TlsOptions
                                               {
                                                 Server = new ServerVerification.CaPem(string.Empty),
                                               },
                                       },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("Value has to be at least 1 character long"));

  /// <summary>An enum is a number, and a number its type does not name is refused.</summary>
  [Test]
  public void ALocationNoNameDeclaresIsRefused()
    => Assert.That(() => new ServerVerification.CaStore(new StoreSearch.FriendlyName("root"),
                                                        (StoreLocation)7).Validate(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("Location has to be a name StoreLocation declares"));

  /// <summary>A field an alternative cannot do without is refused null as it is passed.</summary>
  [Test]
  public void ARequiredFieldIsRefusedNull()
    => Assert.That(() => new ClientCertificate.Pem("me.pem",
                                                   null!),
                   Throws.TypeOf<ArgumentNullException>());

  /// <summary>A secret is printed elided, and so is a proxy URL carrying its credentials.</summary>
  /// <remarks>A record prints every property in its ToString, which reaches logs and debuggers.</remarks>
  [Test]
  public void ASecretIsPrintedElided()
  {
    // A Url's address is elided too: one written with credentials by mistake is still a secret.
    var printed = new ProxyOptions.Url("http://carol:s3cret@proxy.test:3128",
                                       "bob",
                                       "hunter2") + " " + new ProxyOptions.UrlWithCredentials("http://alice:s3cret@proxy.test:3128") + " " +
                  new ClientCertificate.P12("me.p12",
                                            "hunter2");

    Assert.Multiple(() =>
                    {
                      Assert.That(printed,
                                  Does.Not.Contain("s3cret")
                                      .And.Not.Contain("hunter2"));
                      Assert.That(printed,
                                  Does.Contain("Username = bob")
                                      .And.Contain("Path = me.p12"),
                                  "what is not secret is printed");
                    });
  }

  /// <summary>A URL carrying its credentials is an alternative of its own, written as the bare URL.</summary>
  [Test]
  public void AUrlWithCredentialsIsWrittenAndBoundAsTheBareUrl()
  {
    var options = NativeRuntime.OptionsFrom(Configuration(new Dictionary<string, string?>
                                                          {
                                                            ["Section:Transport:Proxy:UrlWithCredentials"] = "http://alice:pw@proxy.test:3128",
                                                          }),
                                            "Section");

    Assert.Multiple(() =>
                    {
                      Assert.That(options.Transport?.Proxy,
                                  Is.EqualTo(new ProxyOptions.UrlWithCredentials("http://alice:pw@proxy.test:3128")));
                      Assert.That(Encoded(options),
                                  Is.EqualTo(@"{""Transport"":{""Proxy"":{""UrlWithCredentials"":""http://alice:pw@proxy.test:3128""}}}"));
                    });
  }

  /// <summary>A section names an alternative by its one key, as the document does, without case.</summary>
  [Test]
  public void ASectionNamesAnAlternativeByItsKey()
  {
    var options = NativeRuntime.OptionsFrom(Configuration(new Dictionary<string, string?>
                                                          {
                                                            ["Section:Transport:Tls:Server:CaStore:Find:Thumbprint"] = "ab",
                                                            ["Section:Transport:Tls:Server:CaStore:Location"]        = "localmachine",
                                                            ["Section:Transport:Tls:Client:Pem:Certificate"]         = "me.pem",
                                                            ["Section:Transport:Tls:Client:Pem:Key"]                 = "me.key",
                                                            ["Section:Transport:Proxy:url:Address"]                  = "proxy.test:3128",
                                                          }),
                                            "Section");

    Assert.Multiple(() =>
                    {
                      Assert.That(options.Transport?.Tls?.Server,
                                  Is.EqualTo(new ServerVerification.CaStore(new StoreSearch.Thumbprint("ab"),
                                                                            StoreLocation.LocalMachine)));
                      Assert.That(options.Transport?.Tls?.Client,
                                  Is.EqualTo(new ClientCertificate.Pem("me.pem",
                                                                       "me.key")));
                      Assert.That(options.Transport?.Proxy,
                                  Is.EqualTo(new ProxyOptions.Url("proxy.test:3128")));
                    });
  }

  /// <summary>What a section cannot be is refused by the key at fault, and its value never quoted.</summary>
  /// <remarks>Not quoted, because a value may be a password.</remarks>
  [Test]
  public void ASectionThatIsNoAlternativeIsRefusedByTheKeyAtFault()
  {
    var refusals = new[]
                   {
                     (new Dictionary<string, string?>
                      {
                        ["Section:Transport:Proxy:None"]            = "true",
                        ["Section:Transport:Proxy:System:Username"] = "s3cret",
                      }, "Section:Transport:Proxy names 2 alternatives"),
                     (new Dictionary<string, string?>
                      {
                        ["Section:Transport:Proxy:Elsewhere"] = "s3cret",
                      }, "Section:Transport:Proxy:Elsewhere names nothing"),
                     (new Dictionary<string, string?>
                      {
                        ["Section:Transport:Tls:Server:Unverified"] = "false",
                      }, "Section:Transport:Tls:Server:Unverified has to be true"),
                     (new Dictionary<string, string?>
                      {
                        ["Section:Transport:Tls:Client:Pem:Certificate"] = "s3cret",
                      }, "Section:Transport:Tls:Client:Pem has to state Key"),
                     (new Dictionary<string, string?>
                      {
                        ["Section:Transport:Tls:Server:CaStore:Find:Thumbprint"] = "ab",
                        ["Section:Transport:Tls:Server:CaStore:Location"]        = "s3cret",
                      }, "Section:Transport:Tls:Server:CaStore:Location has to be one of"),
                     (new Dictionary<string, string?>
                      {
                        ["Section:Grpc:Host:Sends:Window"] = "s3cret",
                      }, "Section:Grpc:Host:Sends:Window has to be an integer"),
                   };

    foreach (var (values, said) in refusals)
    {
      var refused = Assert.Throws<InvalidOperationException>(() => NativeRuntime.OptionsFrom(Configuration(values),
                                                                                            "Section"));

      Assert.Multiple(() =>
                      {
                        Assert.That(refused?.Message,
                                    Does.StartWith(said));
                        Assert.That(refused?.Message,
                                    Does.Not.Contain("s3cret"));
                      });
    }
  }
}
