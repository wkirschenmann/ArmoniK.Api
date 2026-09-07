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
                             DeliveryCredits = 4,
                             UserAgent       = "test",
                           }),
                   Is.EqualTo(@"{""DeliveryCredits"":4,""UserAgent"":""test""}"));

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
                           DeliveryCredits = 0,
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("DeliveryCredits has to be at least 1 and at most 536870910"));

  [Test]
  public void AnEmptyUserAgentIsRefusedBeforeItIsSent()
    => Assert.That(() => new ChannelOptions
                         {
                           UserAgent = string.Empty,
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("at least 1 character long"));

  /// <summary>A group's own bounds are checked through the group that holds it.</summary>
  [Test]
  public void ATimeoutOfZeroIsRefusedThroughItsGroup()
    => Assert.That(() => new ChannelOptions
                         {
                           Transport = new TransportOptions
                                       {
                                         ConnectTimeoutSeconds = 0,
                                       },
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("ConnectTimeoutSeconds has to be greater than 0"));

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
                           MaxReceiveMessageSize = 0,
                         }.Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("MaxReceiveMessageSize has to be at least 1"));

  /// <summary>And the size the schema admits has no upper bound to run into.</summary>
  [Test]
  public void AMessageSizeAsLargeAsAnIntIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             MaxReceiveMessageSize = int.MaxValue,
                           }),
                   Is.EqualTo(@"{""MaxReceiveMessageSize"":2147483647}"));

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
                     DeliveryCredits = 4,
                     Transport = new TransportOptions
                                 {
                                   ConnectTimeoutSeconds = 2.5,
                                 },
                   };

    var copy = new ChannelOptions(original);

    original.DeliveryCredits              = 8;
    original.Transport!.ConnectTimeoutSeconds = 30;

    Assert.Multiple(() =>
                    {
                      Assert.That(copy.DeliveryCredits,
                                  Is.EqualTo(4));
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
                                                                           ["Section:DeliveryCredits"] = "4",
                                                                           ["Section:UserAgent"]       = "first",
                                                                         })
                                                  .AddInMemoryCollection(new Dictionary<string, string?>
                                                                         {
                                                                           ["Section:UserAgent"] = "second",
                                                                         })
                                                  .Build();

    var options = configuration.GetRequiredValue<ChannelOptions>("Section");

    Assert.Multiple(() =>
                    {
                      Assert.That(options.UserAgent,
                                  Is.EqualTo("second"),
                                  "the later layer wins");
                      Assert.That(options.DeliveryCredits,
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

    Environment.SetEnvironmentVariable(prefix + "Section__MaxSendsInFlight",
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
                  Is.EqualTo(@"{""MaxSendsInFlight"":7,""Transport"":{""ConnectTimeoutSeconds"":2.5}}"));
    }
    finally
    {
      Environment.SetEnvironmentVariable(prefix + "Section__MaxSendsInFlight",
                                         null);
      Environment.SetEnvironmentVariable(prefix + "Section__Transport__ConnectTimeoutSeconds",
                                         null);
    }
  }

  /// <summary>A configuration key no option matches is dropped, not refused.</summary>
  /// <remarks>
  ///   .NET's binder ignores it. What `additionalProperties: false` refuses is a misspelling in
  ///   the document, which is the path a Rust or C++ host takes.
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

  [Test]
  public void AValueOnEitherEdgeOfItsRangeIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             DeliveryCredits  = 1,
                             MaxSendsInFlight = 536870910,
                           }),
                   Is.EqualTo(@"{""DeliveryCredits"":1,""MaxSendsInFlight"":536870910}"));
}
