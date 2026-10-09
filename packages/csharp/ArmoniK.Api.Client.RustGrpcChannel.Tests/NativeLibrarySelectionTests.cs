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
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;

using NUnit.Framework;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;
using ArmoniK.Api.Client.Submitter;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Selects the build of the native engine this test process loads, before any test touches it.</summary>
///
/// The suites run once for each build, because a library is loaded once for the life of a process:
/// <c>ARMONIK_TEST_NATIVE=metrics</c> runs them against the build with its counters, and
/// <c>ARMONIK_TEST_NATIVE=client</c> against the same build, asked for through
/// <see cref="GrpcClient.NativeMetrics" /> by the first channel a client opens.
[SetUpFixture]
public class NativeEngineSelection
{
  private static string Mode
    => Environment.GetEnvironmentVariable("ARMONIK_TEST_NATIVE") ?? string.Empty;

  /// <summary>Whether the build is asked for by the option of the client.</summary>
  public static bool ThroughTheClient
    => string.Equals(Mode,
                     "client",
                     StringComparison.OrdinalIgnoreCase);

  /// <summary>The build this process was started to run against.</summary>
  public static NativeEngineBuild Wanted
    => ThroughTheClient || string.Equals(Mode,
                                         "metrics",
                                         StringComparison.OrdinalIgnoreCase)
         ? NativeEngineBuild.Metrics
         : NativeEngineBuild.Default;

  [OneTimeSetUp]
  public async Task SelectTheBuild()
  {
    if (ThroughTheClient)
    {
      // The first channel of a client, which starts the engine and so loads it as the option says.
      NativeChannelFactory.Instance.CreateChannel(new GrpcClient
                                                  {
                                                    Endpoint      = "http://127.0.0.1:1",
                                                    NativeMetrics = true,
                                                  });
      await NativeChannelFactory.Instance.ShutdownAsync()
                                .ConfigureAwait(false);
    }
    else
    {
      NativeLibrarySelection.Select(Wanted);
    }

    // Loaded now, so that a test that asks for the other build finds it refused whatever ran first.
    NativeMethods.Prepare();
    NativeMethods.ak_abi_version();
  }
}

[TestFixture]
public class NativeLibrarySelectionTests
{
  private static void LoadTheEngine()
  {
    NativeMethods.Prepare();
    NativeMethods.ak_abi_version();
  }

  /// <summary>The module the process loaded, by what the operating system says and not by what the binding says.</summary>
  private static string LoadedModule()
    => Process.GetCurrentProcess()
              .Modules.Cast<ProcessModule>()
              .Single(module => string.Equals(Path.GetFileNameWithoutExtension(module.FileName),
                                              NativeMethods.Library,
                                              StringComparison.OrdinalIgnoreCase) || string.Equals(Path.GetFileNameWithoutExtension(module.FileName),
                                                                                                   "lib" + NativeMethods.Library,
                                                                                                   StringComparison.OrdinalIgnoreCase))
              .FileName;

  [Test]
  public void TheBuildTheProcessAskedForIsTheOneLoaded()
  {
    LoadTheEngine();

    var inAMetricsFolder = string.Equals(new DirectoryInfo(Path.GetDirectoryName(LoadedModule())!).Name,
                                         "metrics",
                                         StringComparison.OrdinalIgnoreCase);
    Assert.Multiple(() =>
                    {
                      Assert.That(NativeLibrarySelection.Loaded,
                                  Is.EqualTo(NativeEngineSelection.Wanted));
                      Assert.That(inAMetricsFolder,
                                  Is.EqualTo(NativeEngineSelection.Wanted == NativeEngineBuild.Metrics),
                                  LoadedModule());
                    });
  }

  [Test]
  public void AskingForTheBuildThatIsLoadedChangesNothing()
  {
    LoadTheEngine();

    Assert.DoesNotThrow(() => NativeLibrarySelection.Select(NativeEngineSelection.Wanted));
    Assert.That(NativeLibrarySelection.Loaded,
                Is.EqualTo(NativeEngineSelection.Wanted));
  }

  [Test]
  public void AskingForTheOtherBuildOnceTheEngineIsLoadedIsRefused()
  {
    LoadTheEngine();
    var other = NativeEngineSelection.Wanted == NativeEngineBuild.Metrics
                  ? NativeEngineBuild.Default
                  : NativeEngineBuild.Metrics;

    var refused = Assert.Throws<InvalidOperationException>(() => NativeLibrarySelection.Select(other));

    Assert.Multiple(() =>
                    {
                      Assert.That(refused!.Message,
                                  Does.Contain(other.ToString()));
                      Assert.That(refused.Message,
                                  Does.Contain("a process loads one build"));
                      Assert.That(NativeLibrarySelection.Loaded,
                                  Is.EqualTo(NativeEngineSelection.Wanted),
                                  "the refusal leaves the loaded build as it was");
                    });
  }

  [Test]
  public void ABuildThatIsNotWhereThePackagePutsItSaysWhereItWasLookedFor()
  {
    var missing = RustEngineMissingException.ForBuild(NativeEngineBuild.Metrics,
                                                      new[]
                                                      {
                                                        "the first folder",
                                                        "the second folder",
                                                      },
                                                      null);

    Assert.Multiple(() =>
                    {
                      Assert.That(missing.Message,
                                  Does.Contain("Metrics build"));
                      Assert.That(missing.Message,
                                  Does.Contain("looked for in"));
                      Assert.That(missing.Message,
                                  Does.Contain("the first folder"));
                      Assert.That(missing.Message,
                                  Does.Contain("the second folder"));
                    });
  }

  [Test]
  public void ABuildThatIsFoundAndWillNotLoadSaysWhyAndNotWhereItWasLookedFor()
  {
    var cause   = new BadImageFormatException("the file is for another architecture");
    var missing = RustEngineMissingException.ForBuild(NativeEngineBuild.Metrics,
                                                      null,
                                                      cause);

    Assert.Multiple(() =>
                    {
                      Assert.That(missing.Message,
                                  Does.Contain("Metrics build"));
                      Assert.That(missing.Message,
                                  Does.Not.Contain("looked for"));
                      Assert.That(missing.Message,
                                  Does.Contain("the file is for another architecture"));
                      Assert.That(missing.InnerException,
                                  Is.SameAs(cause));
                    });
  }

  /// <summary>The identifier names a folder of the package, which is where the build is looked for.</summary>
  [Test]
  public void TheRuntimeIdentifierIsTheOneThePackageNamesItsFoldersBy()
  {
    var identifier = NativeLibrarySelection.RuntimeIdentifier();

    var os = RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
               ? "win-"
               : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
                 ? "osx-"
                 : "linux";
    Assert.Multiple(() =>
                    {
                      Assert.That(identifier,
                                  Does.StartWith(os));
                      Assert.That(identifier,
                                  Does.EndWith(RuntimeInformation.ProcessArchitecture.ToString()
                                                                 .ToLowerInvariant()));
                    });
  }
}
