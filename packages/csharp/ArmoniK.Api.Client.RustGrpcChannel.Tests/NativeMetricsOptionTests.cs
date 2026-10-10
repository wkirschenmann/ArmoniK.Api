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
using System.Text.Json;
using System.Threading.Tasks;

using NUnit.Framework;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.Submitter;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary><see cref="GrpcClient.NativeMetrics" />, which the first native channel of a process reads.</summary>
///
/// That it selects the build is what a run with <c>ARMONIK_TEST_NATIVE=client</c> proves: the engine
/// is first loaded by a channel whose client set the option, and every test of the run then finds the
/// build with its counters.
[TestFixture]
public class NativeMetricsOptionTests
{
  private const string NoServer = "http://127.0.0.1:1";

  [Test]
  public void TheOptionIsOffByDefault()
    => Assert.That(new GrpcClient().NativeMetrics,
                   Is.False);

  /// <summary>The engine's options are what the client sets, and this one is the client's: the engine ignores a key it does not know, so a leak would pass unseen.</summary>
  [Test]
  public void TheOptionIsNotSentToTheEngine()
  {
    var without = JsonSerializer.Serialize(NativeClientOptions.Translate(new GrpcClient
                                                                         {
                                                                           Endpoint = NoServer,
                                                                         },
                                                                         true));
    var with = JsonSerializer.Serialize(NativeClientOptions.Translate(new GrpcClient
                                                                      {
                                                                        Endpoint      = NoServer,
                                                                        NativeMetrics = true,
                                                                      },
                                                                      true));

    Assert.Multiple(() =>
                    {
                      Assert.That(with,
                                  Is.EqualTo(without));
                      Assert.That(with,
                                  Does.Not.Contain("NativeMetrics"));
                    });
  }

  /// <summary>The engine starts with the build the option asks for, or with the one that is loaded when it asks for nothing.</summary>
  [Test]
  public async Task AClientThatAsksForTheBuildThatIsLoadedOpensItsChannel()
  {
    await NativeChannelFactory.Instance.ShutdownAsync()
                              .ConfigureAwait(false);
    try
    {
      var channel = NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                                            {
                                                                              Endpoint      = NoServer,
                                                                              NativeMetrics = NativeEngineSelection.Wanted == NativeEngineBuild.Metrics,
                                                                            });

      Assert.That(NativeLibrarySelection.Loaded,
                  Is.EqualTo(NativeEngineSelection.Wanted));
    }
    finally
    {
      await NativeChannelFactory.Instance.ShutdownAsync()
                                .ConfigureAwait(false);
    }
  }

  [Test]
  public async Task AClientThatAsksForTheOtherBuildFailsToOpenItsChannel()
  {
    if (NativeEngineSelection.Wanted == NativeEngineBuild.Metrics)
    {
      Assert.Ignore("a client asks for the build with its counters, and this run loaded it");
    }

    await NativeChannelFactory.Instance.ShutdownAsync()
                              .ConfigureAwait(false);

    try
    {
      var refused = Assert.Throws<InvalidOperationException>(() => NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                                                                               {
                                                                                                                 Endpoint      = NoServer,
                                                                                                                 NativeMetrics = true,
                                                                                                               }));

      Assert.Multiple(() =>
                      {
                        Assert.That(refused!.Message,
                                    Does.Contain("a process loads one build"));
                        Assert.That(NativeLibrarySelection.Loaded,
                                    Is.EqualTo(NativeEngineBuild.Default),
                                    "the refusal leaves the engine as it was");
                      });
    }
    finally
    {
      // A channel that was opened after all would leave its runtime to the tests that follow.
      await NativeChannelFactory.Instance.ShutdownAsync()
                                .ConfigureAwait(false);
    }
  }

  /// <summary>A client that asks for nothing takes what is loaded, whichever build that is.</summary>
  [Test]
  public async Task AClientThatAsksForNothingNeverFailsForTheBuild()
  {
    await NativeChannelFactory.Instance.ShutdownAsync()
                              .ConfigureAwait(false);
    try
    {
      var channel = NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                                            {
                                                                              Endpoint = NoServer,
                                                                            });

      Assert.That(NativeLibrarySelection.Loaded,
                  Is.EqualTo(NativeEngineSelection.Wanted));
    }
    finally
    {
      await NativeChannelFactory.Instance.ShutdownAsync()
                                .ConfigureAwait(false);
    }
  }
}
