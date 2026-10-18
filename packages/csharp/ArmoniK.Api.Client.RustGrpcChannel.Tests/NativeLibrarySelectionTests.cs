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
#if NET8_0_OR_GREATER
using System.Runtime.Intrinsics.X86;
#endif
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

  /// <summary>Whether one of the two folders a file is in has this name: an engine is in the folder
  /// of its build, or in an x86-64-v4 folder inside it.</summary>
  private static bool InAFolderNamed(string file,
                                     string folder)
  {
    var parent = Path.GetDirectoryName(file)!;
    return new[]
           {
             Path.GetFileName(parent),
             Path.GetFileName(Path.GetDirectoryName(parent)),
           }.Contains(folder,
                      StringComparer.OrdinalIgnoreCase);
  }

  [Test]
  public void TheBuildTheProcessAskedForIsTheOneLoaded()
  {
    LoadTheEngine();

    var inAMetricsFolder = InAFolderNamed(LoadedModule(),
                                          "metrics");
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

  /// <summary>A Windows arm64 system that reports the atomics is not refused.</summary>
  [Test]
  public void WindowsArm64WithTheAtomicsIsNotRefused()
  {
    var asked = 0;

    Assert.DoesNotThrow(() => NativeLibrarySelection.RequireAtomics(Architecture.Arm64,
                                                                    true,
                                                                    () =>
                                                                    {
                                                                      asked++;
                                                                      return true;
                                                                    }));
    Assert.That(asked,
                Is.EqualTo(1));
  }

  /// <summary>Without them the engine is refused with a message that says why, before it can run.</summary>
  [Test]
  public void WindowsArm64WithoutTheAtomicsIsRefused()
  {
    var refused = Assert.Throws<PlatformNotSupportedException>(() => NativeLibrarySelection.RequireAtomics(Architecture.Arm64,
                                                                                                          true,
                                                                                                          () => false));

    Assert.That(refused!.Message,
                Does.Contain("Armv8.1 atomic instructions (LSE)")
                    .And.Contain("every Windows 11 on Arm device")
                    .And.Contain("this system does not report them"));
  }

  /// <summary>A probe that cannot be asked is a refusal that keeps its cause.</summary>
  [Test]
  public void AProbeThatFailsIsRefused()
  {
    var cause = new EntryPointNotFoundException();

    var refused = Assert.Throws<PlatformNotSupportedException>(() => NativeLibrarySelection.RequireAtomics(Architecture.Arm64,
                                                                                                          true,
                                                                                                          () => throw cause));

    Assert.That(refused!.InnerException,
                Is.SameAs(cause));
  }

  /// <summary>Any other process is never asked: x64 and x86 have no such feature, and the answer is the
  /// OS's to give on Windows only.</summary>
  [TestCase(Architecture.X64,
            true)]
  [TestCase(Architecture.X86,
            true)]
  [TestCase(Architecture.Arm,
            true)]
  [TestCase(Architecture.Arm64,
            false)]
  [TestCase(Architecture.X64,
            false)]
  public void AnythingElseIsNeverAsked(Architecture architecture,
                                       bool         windows)
  {
    Assert.DoesNotThrow(() => NativeLibrarySelection.RequireAtomics(architecture,
                                                                    windows,
                                                                    () => throw new AssertionException("the system was asked")));
  }

  /// <summary>The x86-64-v4 build is the one loaded exactly when the process runs it and the test
  /// build put it where the package does.</summary>
  [Test]
  public void TheX64V4BuildIsLoadedExactlyWhenTheProcessRunsIt()
  {
    LoadTheEngine();

    Assert.That(InAFolderNamed(LoadedModule(),
                               NativeLibrarySelection.X64V4Folder),
                Is.EqualTo(NativeLibrarySelection.RunsX64V4() && X64V4BuildIsInPlace()),
                LoadedModule());
  }

  /// <summary>Whether the test build put the x86-64-v4 build of the build this process asked for
  /// where the package puts it.</summary>
  private static bool X64V4BuildIsInPlace()
    => NativeLibrarySelection.Candidates(new[]
                                         {
                                           AppDomain.CurrentDomain.BaseDirectory,
                                         },
                                         NativeEngineSelection.Wanted,
                                         true)
                             .Where(candidate => InAFolderNamed(candidate,
                                                                NativeLibrarySelection.X64V4Folder))
                             .Any(File.Exists);

  /// <summary>An x64 process that runs x86-64-v4 loads that build, which the test above cannot tell
  /// from a binding that never picks it. The runtime says whether it does on .NET 8 and later, and
  /// the CPU probe, which the runtime checks there, on .NET Framework.</summary>
  [Test]
  public void WithX64V4TheX64V4BuildIsLoaded()
  {
#if NET8_0_OR_GREATER
    var runsX64V4 = OperatingSystemReportsAvx512F() && RuntimeReportsX64V4();
#else
    var runsX64V4 = OperatingSystemReportsAvx512F() && AskTheTestBuildsCpuProbe() == true;
#endif
    Assume.That(RuntimeInformation.ProcessArchitecture == Architecture.X64 && runsX64V4,
                Is.True,
                "this process does not run x86-64-v4");
    Assume.That(X64V4BuildIsInPlace(),
                Is.True,
                "the test build put no x86-64-v4 build for this identifier");
    LoadTheEngine();

    Assert.That(InAFolderNamed(LoadedModule(),
                               NativeLibrarySelection.X64V4Folder),
                Is.True,
                LoadedModule());
  }

#if NET8_0_OR_GREATER
  /// <summary>Whether the runtime reports every AVX-512 subset x86-64-v4 has, which it does only once
  /// the operating system has enabled their state.</summary>
  private static bool RuntimeReportsX64V4()
    => Avx512F.IsSupported && Avx512F.VL.IsSupported && Avx512BW.IsSupported && Avx512CD.IsSupported && Avx512DQ.IsSupported;
#endif

  /// <summary>The answer of the CPU probe the test build put where the package does.</summary>
  private static bool? AskTheTestBuildsCpuProbe()
    => NativeLibrarySelection.AskTheCpuProbe(NativeLibrarySelection.CpuProbeCandidates(new[]
                                                                                      {
                                                                                        AppDomain.CurrentDomain.BaseDirectory,
                                                                                      }));

  /// <summary>Whether the test build put the CPU probe where the package does.</summary>
  private static bool CpuProbeIsInPlace()
    => NativeLibrarySelection.CpuProbeCandidates(new[]
                                                 {
                                                   AppDomain.CurrentDomain.BaseDirectory,
                                                 })
                             .Any(File.Exists);

#if NET8_0_OR_GREATER
  /// <summary>The probe answers what the runtime reports, so .NET 8 and .NET Framework follow one rule.</summary>
  [Test]
  public void TheCpuProbeAnswersWhatTheRuntimeReports()
  {
    Assume.That(RuntimeInformation.ProcessArchitecture == Architecture.X64 && CpuProbeIsInPlace(),
                Is.True,
                "the test build put no CPU probe for this identifier");

    Assert.That(AskTheTestBuildsCpuProbe(),
                Is.EqualTo(RuntimeReportsX64V4()));
  }
#endif

#if NETFRAMEWORK
  /// <summary>On a CPU without AVX-512F the probe answers, and answers no.</summary>
  [Test]
  public void TheCpuProbeAnswersNoWithoutAvx512F()
  {
    Assume.That(RuntimeInformation.ProcessArchitecture == Architecture.X64 && CpuProbeIsInPlace(),
                Is.True,
                "the test build put no CPU probe for this identifier");
    Assume.That(OperatingSystemReportsAvx512F(),
                Is.False,
                "this CPU has AVX-512F");

    Assert.That(AskTheTestBuildsCpuProbe(),
                Is.False);
  }
#endif

  /// <summary>Only an x64 process asks the probe: no other architecture has an x86-64-v4 build.</summary>
  [TestCase(Architecture.X86)]
  [TestCase(Architecture.Arm)]
  [TestCase(Architecture.Arm64)]
  public void OnlyAnX64ProcessAsksTheCpuProbe(Architecture architecture)
    => Assert.That(NativeLibrarySelection.RunsX64V4(architecture,
                                                    () => throw new AssertionException("the probe was asked")),
                   Is.False);

  /// <summary>An x64 process runs x86-64-v4 when the probe says so, and a probe that is missing or
  /// fails is a no.</summary>
  [TestCase(true,
            ExpectedResult = true)]
  [TestCase(false,
            ExpectedResult = false)]
  [TestCase(null,
            ExpectedResult = false)]
  public bool AnX64ProcessRunsX64V4WhenTheCpuProbeSaysSo(bool? answer)
    => NativeLibrarySelection.RunsX64V4(Architecture.X64,
                                        () => answer);

  [Test]
  public void TheCpuProbeIsLookedForBesideTheBaselineOfEachLayout()
  {
    var probe = NativeLibrarySelection.LibraryFileName(NativeLibrarySelection.CpuProbeLibrary);

    Assert.That(NativeLibrarySelection.CpuProbeCandidates(new[]
                                                          {
                                                            Root,
                                                          }),
                Is.EqualTo(new[]
                           {
                             Path.Combine(Root,
                                          "runtimes",
                                          NativeLibrarySelection.RuntimeIdentifier(),
                                          "native",
                                          probe),
                             Path.Combine(Root,
                                          RuntimeInformation.ProcessArchitecture.ToString()
                                                            .ToLowerInvariant(),
                                          probe),
                             Path.Combine(Root,
                                          probe),
                           }));
  }

  [Test]
  public void AMissingCpuProbeGivesNoAnswer()
    => Assert.That(NativeLibrarySelection.AskTheCpuProbe(new[]
                                                         {
                                                           Path.Combine(Root,
                                                                        "nowhere",
                                                                        "armonik_cpu_probe.dll"),
                                                         }),
                   Is.Null);

  /// <summary>A file that is not a library is no answer, and no error: the probe is an optimisation.</summary>
  [Test]
  public void ACpuProbeThatDoesNotLoadGivesNoAnswer()
  {
    var folder = Path.Combine(Path.GetTempPath(),
                              Path.GetRandomFileName());
    Directory.CreateDirectory(folder);
    try
    {
      var file = Path.Combine(folder,
                              "armonik_cpu_probe.dll");
      File.WriteAllText(file,
                        "not a library");

      Assert.That(NativeLibrarySelection.AskTheCpuProbe(new[]
                                                        {
                                                          file,
                                                        }),
                  Is.Null);
    }
    finally
    {
      Directory.Delete(folder,
                       true);
    }
  }

  /// <summary>A library without the probe's export is no answer.</summary>
  [Test]
  public void ALibraryWithoutTheAnswerGivesNoAnswer()
  {
    Assume.That(RuntimeInformation.IsOSPlatform(OSPlatform.Windows),
                Is.True,
                "kernel32.dll is the library known to load");

    Assert.That(NativeLibrarySelection.AskTheCpuProbe(new[]
                                                      {
                                                        Path.Combine(Environment.SystemDirectory,
                                                                     "kernel32.dll"),
                                                      }),
                Is.Null);
  }

  /// <summary>An x86-64-v4 file that is there and will not load is reported, with the way out.</summary>
  [Test]
  public void AnX64V4BuildThatWillNotLoadSaysDeletingItLoadsTheBaseline()
  {
    var file = InThePackage(NativeLibrarySelection.X64V4Folder);
    var missing = RustEngineMissingException.ForBuild(NativeEngineBuild.Default,
                                                      null,
                                                      new BadImageFormatException("an illegal instruction"),
                                                      file);

    Assert.Multiple(() =>
                    {
                      Assert.That(missing.Message,
                                  Does.Contain(file));
                      Assert.That(missing.Message,
                                  Does.Contain("x86-64-v4"));
                      Assert.That(missing.Message,
                                  Does.Contain("delete it and the binding loads the baseline"));
                    });
  }

  [Test]
  public void AnX64V4BuildIsOneInAnX64V4Folder()
    => Assert.Multiple(() =>
                       {
                         Assert.That(NativeLibrarySelection.InAnX64V4Folder(InThePackage(NativeLibrarySelection.X64V4Folder)),
                                     Is.True);
                         Assert.That(NativeLibrarySelection.InAnX64V4Folder(InThePackage("metrics",
                                                                                         NativeLibrarySelection.X64V4Folder)),
                                     Is.True);
                         Assert.That(NativeLibrarySelection.InAnX64V4Folder(InThePackage("metrics")),
                                     Is.False);
                       });

  /// <summary>A CPU without AVX-512F, as the operating system or the runtime reports it, runs the
  /// baseline, whatever the package carries.</summary>
  /// <remarks>Both are asked, since a Windows that predates the feature's report answers false on any
  /// CPU.</remarks>
  [Test]
  public void WithoutAvx512FTheBaselineIsLoaded()
  {
#if NET8_0_OR_GREATER
    var runtimeReportsAvx512F = Avx512F.IsSupported;
#else
    var runtimeReportsAvx512F = false;
#endif
    Assume.That(OperatingSystemReportsAvx512F() || runtimeReportsAvx512F,
                Is.False,
                "this CPU has AVX-512F: the tests above check which build it loads");
    LoadTheEngine();

    Assert.Multiple(() =>
                    {
                      Assert.That(NativeLibrarySelection.RunsX64V4(),
                                  Is.False);
                      Assert.That(InAFolderNamed(LoadedModule(),
                                                 NativeLibrarySelection.X64V4Folder),
                                  Is.False,
                                  LoadedModule());
                    });
  }

  private const uint PF_AVX512F_INSTRUCTIONS_AVAILABLE = 41;

  /// <summary>Asked of the operating system rather than of the binding, which is what is checked.</summary>
  private static bool OperatingSystemReportsAvx512F()
  {
    if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
    {
      return NativeMethods.IsProcessorFeaturePresent(PF_AVX512F_INSTRUCTIONS_AVAILABLE);
    }

    if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
    {
      // The kernel lists the flag only when it saves the AVX-512 state.
      return File.ReadAllText("/proc/cpuinfo")
                 .Split(' ',
                        '\t',
                        '\n')
                 .Contains("avx512f");
    }

    Assert.Inconclusive("only Windows and Linux are asked whether they have AVX-512F");
    return false;
  }

  private static readonly string Root = Path.Combine(Path.GetTempPath(),
                                                     "a package");

  /// <summary>The path of the engine in a folder of the package's layout for .NET.</summary>
  private static string InThePackage(params string[] folders)
    => Path.Combine(new[]
                    {
                      Root,
                      "runtimes",
                      NativeLibrarySelection.RuntimeIdentifier(),
                    }.Concat(folders)
                     .Concat(new[]
                             {
                               NativeLibrarySelection.LibraryFileName(),
                             })
                     .ToArray());

  private static string? Pick(NativeEngineBuild build,
                              bool              x64V4,
                              params string[]   files)
    => NativeLibrarySelection.Locate(NativeLibrarySelection.Candidates(new[]
                                                                       {
                                                                         Root,
                                                                       },
                                                                       build,
                                                                       x64V4),
                                     files.Contains);

  [Test]
  public void TheDefaultBuildIsTheX64V4OneWhenTheProcessRunsItAndItIsThere()
    => Assert.That(Pick(NativeEngineBuild.Default,
                        true,
                        InThePackage(NativeLibrarySelection.X64V4Folder)),
                   Is.EqualTo(InThePackage(NativeLibrarySelection.X64V4Folder)));

  /// <summary>No file for the default build means the baseline, which the platform's own search finds.</summary>
  [Test]
  public void TheDefaultBuildIsTheBaselineWhenTheX64V4OneIsMissing()
    => Assert.That(Pick(NativeEngineBuild.Default,
                        true),
                   Is.Null);

  [Test]
  public void TheDefaultBuildIsTheBaselineWhenTheProcessDoesNotRunX64V4()
    => Assert.That(Pick(NativeEngineBuild.Default,
                        false,
                        InThePackage(NativeLibrarySelection.X64V4Folder)),
                   Is.Null);

  [Test]
  public void TheMetricsBuildIsTheX64V4OneWhenTheProcessRunsItAndItIsThere()
    => Assert.That(Pick(NativeEngineBuild.Metrics,
                        true,
                        InThePackage("metrics"),
                        InThePackage("metrics",
                                     NativeLibrarySelection.X64V4Folder)),
                   Is.EqualTo(InThePackage("metrics",
                                           NativeLibrarySelection.X64V4Folder)));

  [Test]
  public void TheMetricsBuildIsTheBaselineWhenTheX64V4OneIsMissing()
    => Assert.That(Pick(NativeEngineBuild.Metrics,
                        true,
                        InThePackage("metrics")),
                   Is.EqualTo(InThePackage("metrics")));

  [Test]
  public void TheMetricsBuildIsTheBaselineWhenTheProcessDoesNotRunX64V4()
    => Assert.That(Pick(NativeEngineBuild.Metrics,
                        false,
                        InThePackage("metrics"),
                        InThePackage("metrics",
                                     NativeLibrarySelection.X64V4Folder)),
                   Is.EqualTo(InThePackage("metrics")));
}
