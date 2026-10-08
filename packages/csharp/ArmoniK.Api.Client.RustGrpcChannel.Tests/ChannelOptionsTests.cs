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

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What the generated vocabulary sends, and what it refuses to send.</summary>
[TestFixture]
public class ChannelOptionsTests
{
  private static string Encoded(ChannelOptions options)
    => Encoding.UTF8.GetString(options.Encode());

  // The delivery window and the send window, where the vocabulary nests them.
  private static GrpcOptions Windows(int? credits = null,
                                     int? sends   = null)
    => new()
       {
         Host = new HostOptions
                {
                  Send = sends is null
                           ? null
                           : new HostSendOptions
                             {
                               Window = sends,
                             },
                  Receive = credits is null
                              ? null
                              : new HostReceiveOptions
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

  private static ChannelOptions Retrying(RetryCodes codes)
    => new()
       {
         Grpc = new GrpcOptions
                {
                  Retry = new RetryOptions
                          {
                            Codes = codes,
                          },
                },
       };

  /// <summary>The retryable statuses are an alternative: a preset names its set, and a list names each status.</summary>
  [Test]
  public void TheRetryCodesAreWrittenAsTheAlternativeThatIsChosen()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Encoded(Retrying(new RetryCodes.GoogleRpc())),
                                     Is.EqualTo(@"{""Grpc"":{""Retry"":{""Codes"":{""GoogleRpc"":true}}}}"));
                         Assert.That(Encoded(Retrying(new RetryCodes.GrpcClient())),
                                     Is.EqualTo(@"{""Grpc"":{""Retry"":{""Codes"":{""GrpcClient"":true}}}}"));
                         Assert.That(Encoded(Retrying(new RetryCodes.List(new[]
                                                                          {
                                                                            RetryableStatus.UNAVAILABLE,
                                                                            RetryableStatus.DEADLINE_EXCEEDED,
                                                                          }))),
                                     Is.EqualTo(@"{""Grpc"":{""Retry"":{""Codes"":{""List"":[""UNAVAILABLE"",""DEADLINE_EXCEEDED""]}}}}"));
                       });

  /// <summary>A list of statuses is a value: equal to one that names the same, and a copy of what it was given.</summary>
  [Test]
  public void AListOfRetryableStatusesIsAValueAndKeepsWhatItWasGiven()
  {
    var given = new List<RetryableStatus>
                {
                  RetryableStatus.ABORTED,
                };
    var list = new RetryCodes.List(given);
    given.Add(RetryableStatus.UNKNOWN);

    Assert.Multiple(() =>
                    {
                      Assert.That(list.Value,
                                  Is.EqualTo(new[]
                                             {
                                               RetryableStatus.ABORTED,
                                             }),
                                  "the record holds its own copy");
                      Assert.That(list,
                                  Is.EqualTo(new RetryCodes.List(new[]
                                                                 {
                                                                   RetryableStatus.ABORTED,
                                                                 })));
                      Assert.That(list.GetHashCode(),
                                  Is.EqualTo(new RetryCodes.List(new[]
                                                                 {
                                                                   RetryableStatus.ABORTED,
                                                                 }).GetHashCode()));
                      Assert.That(list,
                                  Is.Not.EqualTo(new RetryCodes.List(new[]
                                                                     {
                                                                       RetryableStatus.UNKNOWN,
                                                                     })));
                      Assert.That(list.ToString(),
                                  Does.Contain("[ABORTED]"));
                      Assert.That(() => new RetryCodes.List(null!),
                                  Throws.TypeOf<ArgumentNullException>());
                    });
  }

  /// <summary>A number that is no status is refused before it is sent.</summary>
  [Test]
  public void ARetryListOfAnUndefinedStatusIsRefusedBeforeItIsSent()
    => Assert.That(() => Retrying(new RetryCodes.List(new[]
                                                      {
                                                        (RetryableStatus)99,
                                                      })).Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("RetryableStatus"));

  /// <summary>A rate limit is two options of one group, written under the channel's calls.</summary>
  [Test]
  public void ARateLimitIsAGroupOfTwoOptions()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = new GrpcOptions
                                    {
                                      Rate = new RateOptions
                                             {
                                               Limit = new RateLimitOptions
                                                       {
                                                         Calls      = 100,
                                                         PerSeconds = 0.25,
                                                       },
                                             },
                                    },
                           }),
                   Is.EqualTo(@"{""Grpc"":{""Rate"":{""Limit"":{""Calls"":100,""PerSeconds"":0.25}}}}"));

  /// <summary>A rate limit that starts no request is refused before it is sent.</summary>
  [TestCase(0,
            1.0,
            "Calls has to be at least 1",
            TestName = "{m}(no calls)")]
  [TestCase(1,
            0.0,
            "PerSeconds has to be at least 1E-09",
            TestName = "{m}(a window that is over as it opens)")]
  public void ARateLimitThatStartsNoRequestIsRefusedBeforeItIsSent(int    calls,
                                                                   double perSeconds,
                                                                   string reason)
    => Assert.That(() => new ChannelOptions
                         {
                           Grpc = new GrpcOptions
                                  {
                                    Rate = new RateOptions
                                           {
                                             Limit = new RateLimitOptions
                                                     {
                                                       Calls      = calls,
                                                       PerSeconds = perSeconds,
                                                     },
                                           },
                                  },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains(reason));

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
                                    Receive = new GrpcReceiveOptions
                                              {
                                                MaxMessageSize = 0,
                                              },
                                  },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("MaxMessageSize has to be at least 1"));

  /// <summary>And the size the schema admits has no upper bound to run into.</summary>
  [Test]
  public void AMessageSizeAsLargeAsAnIntIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = new GrpcOptions
                                    {
                                      Receive = new GrpcReceiveOptions
                                                {
                                                  MaxMessageSize = int.MaxValue,
                                                },
                                    },
                           }),
                   Is.EqualTo(@"{""Grpc"":{""Receive"":{""MaxMessageSize"":2147483647}}}"));

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

  [Test]
  public void AValueOnEitherEdgeOfItsRangeIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             Grpc = Windows(1,
                                            536870910),
                           }),
                   Is.EqualTo(@"{""Grpc"":{""Host"":{""Send"":{""Window"":536870910},""Receive"":{""Window"":1}}}}"));

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
}
