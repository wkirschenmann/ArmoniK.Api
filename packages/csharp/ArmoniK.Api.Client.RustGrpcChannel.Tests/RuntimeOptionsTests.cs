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
                                                                   ["RustGrpcRuntime:WorkerThreads"]     = "2",
                                                                   ["RustGrpcRuntime:MemoryCeiling"]     = "65536",
                                                                   ["RustGrpcRuntime:MemoryHardCeiling"] = "131072",
                                                                 }));

    Assert.Multiple(() =>
                    {
                      Assert.That(options.WorkerThreads,
                                  Is.EqualTo(2));
                      Assert.That(options.MemoryCeiling,
                                  Is.EqualTo(65536));
                      Assert.That(options.MemoryHardCeiling,
                                  Is.EqualTo(131072));
                    });
  }

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
