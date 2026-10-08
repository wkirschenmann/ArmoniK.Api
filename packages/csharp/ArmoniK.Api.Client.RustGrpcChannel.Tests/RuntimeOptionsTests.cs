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
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>A runtime started with options: its ceilings, and the channel defaults its channels take.</summary>
[TestFixture]
public class RuntimeOptionsTests : RuntimeFixture
{
  // A delivery window, where the vocabulary nests it.
  private static GrpcOptions Credits(int credits)
    => new()
       {
         Host = new HostOptions
                {
                  Receive = new HostReceiveOptions
                            {
                              Window = credits,
                            },
                },
       };

  /// <summary>A channel takes the runtime's delivery window where its options name none, and its
  /// own where they do: the ring this side sizes is the window the engine grants.</summary>
  [Test]
  public async Task AChannelTakesTheRuntimeDefaultsUnderItsOwnOptions()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new RuntimeOptions
                                                                {
                                                                  ChannelDefaults = new ChannelOptions
                                                                                    {
                                                                                      Grpc = Credits(2),
                                                                                    },
                                                                }))
                    .ConfigureAwait(false);

    var defaulted = runtime.Channel("http://127.0.0.1:1",
                                    new ChannelOptions());
    var bare = runtime.Channel("http://127.0.0.1:1");
    var own = runtime.Channel("http://127.0.0.1:1",
                              new ChannelOptions
                              {
                                Grpc = Credits(3),
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
                                                                    Grpc = Credits(0),
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
                                                                    Grpc = Credits(NativeRuntime.MaxDeliveryCredits + 1),
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
                                                                                                                         ServerCertificates = new ServerCertificates.CaPem("no/such/ca.pem"),
                                                                                                                       },
                                                                                                               },
                                                                                                 },
                                                                             }))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("ChannelDefaults: Transport.Tls.ServerCertificates.CaPem"));

  /// <summary>The engine runs with what its configuration says, read back from its own accounting.</summary>
  [Test]
  public async Task ARuntimeStartedFromAConfigurationEnforcesItsCeiling()
  {
    var configuration = new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                            {
                                                                              "--ArmoniK:Client:Grpc:MemoryCeiling=65536",
                                                                              "--ArmoniK:Client:Grpc:MemoryHardCeiling=131072",
                                                                            });

    var runtime = await RestartAsync(() => NativeRuntime.Create(configuration))
                    .ConfigureAwait(false);

    Assert.That(Ceiling(runtime.Handle),
                Is.EqualTo(65536UL));
  }

  /// <summary>A prefix of several parts is nested sections in a file, read from its depth.</summary>
  [Test]
  public async Task APrefixOfSeveralPartsIsNestedSectionsOfAFile()
  {
    var path = Path.Combine(Path.GetTempPath(),
                            "armonik-nested-" + Guid.NewGuid()
                                                    .ToString("N") + ".json");
    File.WriteAllText(path,
                      "{ \"Outer\": { \"Inner\": { \"MemoryCeiling\": 65536 } }, \"MemoryCeiling\": 1 }");
    try
    {
      var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration("Outer__Inner").LoadConfigFromFiles(path)))
                      .ConfigureAwait(false);

      Assert.That(Ceiling(runtime.Handle),
                  Is.EqualTo(65536UL));
    }
    finally
    {
      File.Delete(path);
    }
  }

  /// <summary>And of a command line, which .NET reads as a path of sections.</summary>
  [Test]
  public async Task APrefixOfSeveralPartsIsNestedSectionsOfACommandLine()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration("Outer__Inner").LoadConfigFromCommandLine(new[]
                                                                                                                                  {
                                                                                                                                    "--Outer:Inner:MemoryCeiling=32768",
                                                                                                                                    "--Outer:MemoryCeiling=1",
                                                                                                                                  })))
                    .ConfigureAwait(false);

    Assert.That(Ceiling(runtime.Handle),
                Is.EqualTo(32768UL));
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
