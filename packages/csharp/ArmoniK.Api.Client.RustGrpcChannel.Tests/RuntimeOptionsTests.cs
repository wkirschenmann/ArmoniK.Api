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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using Microsoft.Extensions.Configuration;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The runtime's options, read from a configuration as a channel's are.</summary>
[TestFixture]
public class RuntimeOptionsTests : RuntimeFixture
{
  private static IConfiguration Configuration(Dictionary<string, string?> values)
    => new ConfigurationBuilder().AddInMemoryCollection(values)
                                 .Build();

  [Test]
  public void ASectionCarriesEveryRuntimeOption()
  {
    var options = NativeRuntime.RuntimeOptionsFrom(Configuration(new Dictionary<string, string?>
                                                                 {
                                                                   ["RustGrpcRuntime:MemoryCeiling"]     = "65536",
                                                                   ["RustGrpcRuntime:MemoryHardCeiling"] = "131072",
                                                                 }));

    Assert.Multiple(() =>
                    {
                      Assert.That(options.MemoryCeiling,
                                  Is.EqualTo(65536));
                      Assert.That(options.MemoryHardCeiling,
                                  Is.EqualTo(131072));
                    });
  }

  /// <summary>Every channel option can be a runtime's default, read under its own section.</summary>
  [Test]
  public void ASectionCarriesTheChannelDefaults()
  {
    var options = NativeRuntime.RuntimeOptionsFrom(Configuration(new Dictionary<string, string?>
                                                                 {
                                                                   ["RustGrpcRuntime:ChannelDefaults:DeliveryCredits"]          = "2",
                                                                   ["RustGrpcRuntime:ChannelDefaults:Http2:KeepAliveWhileIdle"] = "true",
                                                                   ["RustGrpcRuntime:ChannelDefaults:Transport:Proxy:None"]     = "true",
                                                                 }));

    Assert.Multiple(() =>
                    {
                      Assert.That(options.ChannelDefaults?.DeliveryCredits,
                                  Is.EqualTo(2));
                      Assert.That(options.ChannelDefaults?.Http2?.KeepAliveWhileIdle,
                                  Is.True);
                      Assert.That(options.ChannelDefaults?.Transport?.Proxy,
                                  Is.EqualTo(new ProxyOptions.None()),
                                  "an alternative is named by its key there too");
                    });
  }

  /// <summary>A channel takes the runtime's delivery window where its options name none, and its
  /// own where they do: the ring this side sizes is the window the engine grants.</summary>
  [Test]
  public async Task AChannelTakesTheRuntimeDefaultsUnderItsOwnOptions()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new RuntimeOptions
                                                                {
                                                                  ChannelDefaults = new ChannelOptions
                                                                                    {
                                                                                      DeliveryCredits = 2,
                                                                                    },
                                                                }))
                    .ConfigureAwait(false);

    var defaulted = runtime.Channel("http://127.0.0.1:1",
                                    new ChannelOptions());
    var bare = runtime.Channel("http://127.0.0.1:1");
    var own = runtime.Channel("http://127.0.0.1:1",
                              new ChannelOptions
                              {
                                DeliveryCredits = 3,
                              });
    try
    {
      Assert.Multiple(() =>
                      {
                        Assert.That(defaulted.DeliveryCredits,
                                    Is.EqualTo(2));
                        Assert.That(bare.DeliveryCredits,
                                    Is.EqualTo(2));
                        Assert.That(own.DeliveryCredits,
                                    Is.EqualTo(3));
                      });
    }
    finally
    {
      await defaulted.DisposeAsync()
                     .ConfigureAwait(false);
      await bare.DisposeAsync()
                .ConfigureAwait(false);
      await own.DisposeAsync()
               .ConfigureAwait(false);
    }
  }

  /// <summary>A default outside its bounds is refused when the runtime starts, not at the first
  /// channel.</summary>
  [Test]
  public void ADefaultOutsideItsBoundsIsRefusedAtTheStart()
    => Assert.That(() => NativeRuntime.Create(new RuntimeOptions
                                              {
                                                ChannelDefaults = new ChannelOptions
                                                                  {
                                                                    DeliveryCredits = 0,
                                                                  },
                                              }),
                   Throws.InstanceOf<ArgumentOutOfRangeException>());

  /// <summary>The binding's own bound on a delivery window holds for a default too.</summary>
  [Test]
  public void ADefaultWindowNoRingCanHoldIsRefusedAtTheStart()
    => Assert.That(() => NativeRuntime.Create(new RuntimeOptions
                                              {
                                                ChannelDefaults = new ChannelOptions
                                                                  {
                                                                    DeliveryCredits = NativeRuntime.MaxDeliveryCredits + 1,
                                                                  },
                                              }),
                   Throws.InstanceOf<ArgumentOutOfRangeException>());

  /// <summary>The defaults reach the engine, which refuses one this side cannot check, by its name.</summary>
  /// <remarks>A root file that names nothing is read by the engine alone, when the runtime starts.</remarks>
  [Test]
  public void ADefaultTheEngineRefusesIsRefusedAtTheStart()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new RuntimeOptions
                                                                             {
                                                                               ChannelDefaults = new ChannelOptions
                                                                                                 {
                                                                                                   Transport = new TransportOptions
                                                                                                               {
                                                                                                                 Tls = new TlsOptions
                                                                                                                       {
                                                                                                                         Server = new ServerVerification.CaPem("no/such/ca.pem"),
                                                                                                                       },
                                                                                                               },
                                                                                                 },
                                                                             }))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("ChannelDefaults: Transport.Tls.Server.CaPem"));

  /// <summary>Refused as a channel's misspelling is: a runtime nobody configured would start with
  /// the engine's defaults and say nothing.</summary>
  [Test]
  public void AKeyNoRuntimeOptionMatchesIsRefused()
  {
    var refused = Assert.Throws<InvalidOperationException>(() => NativeRuntime.RuntimeOptionsFrom(Configuration(new Dictionary<string, string?>
                                                                                                                {
                                                                                                                  ["RustGrpcRuntime:MemoryCeil"] = "65536",
                                                                                                                })));

    Assert.That(refused?.Message,
                Does.Contain("MemoryCeil"));
  }

  /// <summary>The engine runs with what the section says, read back from its own accounting.</summary>
  [Test]
  public async Task ARuntimeStartedFromAConfigurationEnforcesItsCeiling()
  {
    var configuration = Configuration(new Dictionary<string, string?>
                                      {
                                        ["RustGrpcRuntime:MemoryCeiling"]     = "65536",
                                        ["RustGrpcRuntime:MemoryHardCeiling"] = "131072",
                                      });

    var runtime = await RestartAsync(() => NativeRuntime.Create(configuration))
                    .ConfigureAwait(false);

    Assert.That(Ceiling(runtime.Handle),
                Is.EqualTo(65536UL));
  }

  /// <summary>Zero is the ABI's default and no option's value: asking for the default is leaving the
  /// option out.</summary>
  [Test]
  public void AZeroIsRefusedBeforeTheEngineIsAsked()
    => Assert.That(() => NativeRuntime.Create(new RuntimeOptions
                                              {
                                                MemoryCeiling = 0,
                                              }),
                   Throws.InstanceOf<ArgumentOutOfRangeException>());

  /// <summary>Refused by the engine, not by the binding: the order of the two is the engine's rule.
  /// Which is also what shows the second threshold reaching the engine.</summary>
  [Test]
  public void ASecondThresholdBelowTheFirstIsRefused()
    => Assert.That(() => RestartAsync(() => NativeRuntime.Create(new RuntimeOptions
                                                                {
                                                                  MemoryCeiling     = 65536,
                                                                  MemoryHardCeiling = 1024,
                                                                })),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("memory_hard_ceiling"));

  private static unsafe ulong Ceiling(ulong runtime)
  {
    ak_memory_usage usage;
    Assert.That(NativeMethods.ak_runtime_memory_usage(runtime,
                                                      &usage,
                                                      null),
                Is.EqualTo(ak_status.AK_STATUS_OK));
    return usage.ceiling;
  }
}
