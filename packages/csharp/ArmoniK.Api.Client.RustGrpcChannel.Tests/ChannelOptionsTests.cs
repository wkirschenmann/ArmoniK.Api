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
using System.Text;

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

  [Test]
  public void AValueOnEitherEdgeOfItsRangeIsAdmitted()
    => Assert.That(Encoded(new ChannelOptions
                           {
                             DeliveryCredits  = 1,
                             MaxSendsInFlight = 536870910,
                           }),
                   Is.EqualTo(@"{""DeliveryCredits"":1,""MaxSendsInFlight"":536870910}"));
}
