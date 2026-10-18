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
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The builds of the native engine the package carries.</summary>
public enum NativeEngineBuild
{
  /// <summary>The engine with no counting: what a process loads when it asks for nothing.</summary>
  Default,

  /// <summary>The engine built with its <c>metrics</c> feature, which counts what it does for the
  /// instruments of <c>ArmoniK.Api.Client.RustGrpcChannel.*</c> meters to read.</summary>
  Metrics,
}

/// <summary>Which build of the native engine this process loads.</summary>
///
/// The package carries the engine twice, under the same file name: the default build in the folders a
/// package manager reads native assets from, and the build with its counters in a <c>metrics</c>
/// folder beside it. A library is loaded once for the life of the process, so the choice is made
/// before the first runtime is created - the first call of the binding that reaches the engine
/// loads it - and asking for the other build afterwards is refused. A process loads one build, since
/// the binding does not unload the library it loaded.
///
/// For win-x64 and linux-x64 each build is also carried compiled for x86-64-v4, in an
/// <c>x86-64-v4</c> folder, which the binding loads in its place when a CPU probe shipped beside the
/// baseline says the CPU and the operating system run it (the repository's
/// spec/armonik_grpc_ffi/architecture.md, "Which engine file a process loads", gives the details).
public static class NativeLibrarySelection
{
  private static readonly object Gate = new();

  private static NativeEngineBuild requested_ = NativeEngineBuild.Default;

  private static bool committed_;

  // Why the engine is not loaded: it is missing, or this system cannot run it.
  private static Exception? failure_;

  /// <summary>The build this process loaded, or <c>null</c> while it has loaded none.</summary>
  public static NativeEngineBuild? Loaded
  {
    get
    {
      lock (Gate)
      {
        return committed_ && failure_ is null
                 ? requested_
                 : null;
      }
    }
  }

  /// <summary>Asks for the build the process loads, which has to be done before the first runtime is created.</summary>
  /// <param name="build">The build to load.</param>
  /// <exception cref="InvalidOperationException">
  ///   The engine is already loaded as the other build.
  /// </exception>
  /// <exception cref="PlatformNotSupportedException">
  ///   The metrics build is asked for from the netstandard2.0 build of this assembly on a runtime
  ///   other than .NET Framework; or the engine was already refused on this system, a Windows on Arm one
  ///   that reports no Armv8.1 atomic instructions, which the first call into the engine finds out.
  /// </exception>
  public static void Select(NativeEngineBuild build)
  {
    if (!Enum.IsDefined(typeof(NativeEngineBuild),
                        build))
    {
      throw new ArgumentOutOfRangeException(nameof(build),
                                            build,
                                            "no such build");
    }

#if !NET5_0_OR_GREATER
    // Refused when asked: a library loaded by path loses to the application's copy here, so the
    // default build would load in its place.
    if (build == NativeEngineBuild.Metrics && !IsNetFramework())
    {
      throw new PlatformNotSupportedException("the metrics build of the native engine cannot be selected: this is the netstandard2.0 build of ArmoniK.Api.Client.RustGrpcChannel "
                                              + $"running on {RuntimeInformation.FrameworkDescription}, where a library loaded by path loses to the one beside the application, so the default build would load. "
                                              + "Run on .NET Framework, or use the build of this assembly for net8.0 or later, which a project targeting net8.0 or later resolves to.");
    }
#endif

    lock (Gate)
    {
      if (committed_)
      {
        if (failure_ is PlatformNotSupportedException)
        {
          throw failure_;
        }

        if (build != requested_)
        {
          throw new InvalidOperationException($"the native engine is already loaded as the {requested_} build, and asking for the {build} build is refused: " + "a process loads one build, since a library is loaded once for the life of the process and is not unloaded. "
                                              + "Select the build before the first native runtime is created.");
        }

        return;
      }

      requested_ = build;
    }
  }

  /// <summary>Settles the build and prepares its loading, once, before the first call into the engine.</summary>
  /// <remarks>Called by the declarations' static constructor, which runs before any of them is called.
  /// A failure is kept rather than thrown, since an exception of a static constructor is a type that
  /// can never be used again: <see cref="ThrowIfNotFound" /> reports it where a caller can read it.</remarks>
  internal static void Commit()
  {
    lock (Gate)
    {
      if (committed_)
      {
        return;
      }

      committed_ = true;
      try
      {
        RequireAtomics(RuntimeInformation.ProcessArchitecture,
                       RuntimeInformation.IsOSPlatform(OSPlatform.Windows),
                       () => NativeMethods.IsProcessorFeaturePresent(NativeMethods.PF_ARM_V81_ATOMIC_INSTRUCTIONS_AVAILABLE));
      }
      catch (PlatformNotSupportedException unsupported)
      {
        failure_ = unsupported;
        return;
      }

      string? path = null;
      string? x64V4File = null;
      try
      {
        var roots = Roots();
        path = Locate(Candidates(roots,
                                 requested_,
                                 RunsX64V4()),
                      File.Exists);
        if (path is not null)
        {
          x64V4File = InAnX64V4Folder(path)
                        ? path
                        : null;
          Load(path);
        }
        else if (requested_ == NativeEngineBuild.Metrics)
        {
          throw RustEngineMissingException.ForBuild(NativeEngineBuild.Metrics,
                                                    Folders(roots,
                                                            NativeEngineBuild.Metrics),
                                                    null);
        }
        else
        {
          LoadBesideTheApplication();
        }
      }
      catch (Exception error) when (error is not RustEngineMissingException)
      {
        // The default build's baseline is found by the platform's own search when this fails. A file
        // that is there and does not load is reported: falling back would hide a broken package.
        if (requested_ == NativeEngineBuild.Metrics || path is not null)
        {
          failure_ = RustEngineMissingException.ForBuild(requested_,
                                                         null,
                                                         error,
                                                         x64V4File);
        }
      }
      catch (RustEngineMissingException missing)
      {
        failure_ = missing;
      }
    }
  }

  /// <summary>Refuses a Windows arm64 process whose system reports no Armv8.1 atomics (LSE), which the engine
  /// for that platform needs: without them it stops on an illegal instruction with nothing to report it.</summary>
  /// <param name="architecture">The architecture of the process, which is that of the engine it loads.</param>
  /// <param name="windows">Whether the process runs on Windows.</param>
  /// <param name="atomicsPresent">Asks the system whether it has the atomics; called for Windows arm64 only.
  /// A probe that fails is a refusal, since the engine is not run on a guess.</param>
  /// <remarks>The architecture is <c>RuntimeInformation.ProcessArchitecture</c>, which netstandard2.0 has
  /// and which names the process rather than the machine: an x64 process emulated on arm64 loads the x64
  /// engine and is never asked.</remarks>
  /// <exception cref="PlatformNotSupportedException">The system reports no Armv8.1 atomics.</exception>
  internal static void RequireAtomics(Architecture architecture,
                                      bool         windows,
                                      Func<bool>   atomicsPresent)
  {
    if (!windows || architecture != Architecture.Arm64)
    {
      return;
    }

    bool present;
    try
    {
      present = atomicsPresent();
    }
    catch (Exception error)
    {
      throw new PlatformNotSupportedException("the system could not be asked whether it has the Armv8.1 atomic instructions (LSE) that the native engine for Windows on Arm needs",
                                              error);
    }

    if (present)
    {
      return;
    }

    throw new PlatformNotSupportedException("the native engine for Windows on Arm needs the Armv8.1 atomic instructions (LSE), which every Windows 11 on Arm device has, and this system does not report them. "
                                            + "It was not loaded, since it would stop on an illegal instruction.");
  }

  /// <summary>Raises why the build asked for could not be found, when it could not.</summary>
  /// <exception cref="PlatformNotSupportedException">The system cannot run the engine.</exception>
  internal static void ThrowIfNotFound()
  {
    lock (Gate)
    {
      if (failure_ is not null)
      {
        throw failure_;
      }
    }
  }

  /// <summary>The folder, inside the folder of each build, that holds that build compiled for x86-64-v4.</summary>
  internal const string X64V4Folder = "x86-64-v4";

  /// <summary>The library that answers whether the CPU runs x86-64-v4, carried beside the x64 baselines.</summary>
  internal const string CpuProbeLibrary = "armonik_cpu_probe";

  private const string CpuProbeExport = "armonik_cpu_probe_runs_x86_64_v4";

  /// <summary>Whether this process loads the engine compiled for x86-64-v4, as the CPU probe answers.</summary>
  internal static bool RunsX64V4()
    => RunsX64V4(RuntimeInformation.ProcessArchitecture,
                 () => AskTheCpuProbe(CpuProbeCandidates(Roots())));

  /// <summary>Whether a process of this architecture loads the engine compiled for x86-64-v4: an x64
  /// one whose CPU probe says the CPU and the operating system run it.</summary>
  /// <param name="architecture">The architecture of the process.</param>
  /// <param name="probe">Asks the CPU probe; <c>null</c> when it is missing or fails, which is a no,
  /// since the baseline runs everywhere.</param>
  internal static bool RunsX64V4(Architecture architecture,
                                 Func<bool?>  probe)
  {
    if (architecture != Architecture.X64)
    {
      return false;
    }

#if !NET5_0_OR_GREATER
    // On a .NET runtime a library this build loads by its path loses to the one beside the
    // application, so the x86-64-v4 file would be loaded and the baseline bound.
    if (!IsNetFramework())
    {
      return false;
    }
#endif

    return probe() == true;
  }

  /// <summary>Where the CPU probe is looked for, in order: beside the baseline in each layout the
  /// engine is looked for in.</summary>
  /// <param name="roots">The folders the package's layouts are looked for in.</param>
  internal static IReadOnlyList<string> CpuProbeCandidates(IReadOnlyList<string> roots)
    => roots.SelectMany(root => new[]
                                {
                                  Path.Combine(root,
                                               "runtimes",
                                               RuntimeIdentifier(),
                                               "native"),
                                  Path.Combine(root,
                                               RuntimeInformation.ProcessArchitecture.ToString()
                                                                 .ToLowerInvariant()),
                                  root,
                                })
            .Select(folder => Path.Combine(folder,
                                           LibraryFileName(CpuProbeLibrary)))
            .ToList();

  /// <summary>The first of the candidates that is there, loaded, asked, and freed.</summary>
  /// <returns>Its answer, or <c>null</c> when none is there or the one found does not load or does
  /// not answer.</returns>
  internal static unsafe bool? AskTheCpuProbe(IEnumerable<string> candidates)
  {
    var path = Locate(candidates,
                      File.Exists);
    if (path is null)
    {
      return null;
    }

#if NET5_0_OR_GREATER
    if (!NativeLibrary.TryLoad(path,
                               out var library))
    {
      return null;
    }

    try
    {
      return NativeLibrary.TryGetExport(library,
                                        CpuProbeExport,
                                        out var answer)
               ? ((delegate* unmanaged[Cdecl]<byte>)answer)() != 0
               : null;
    }
    finally
    {
      NativeLibrary.Free(library);
    }
#else
    var library = NativeMethods.LoadLibrary(path);
    if (library == IntPtr.Zero)
    {
      return null;
    }

    try
    {
      var answer = NativeMethods.GetProcAddress(library,
                                                CpuProbeExport);
      return answer != IntPtr.Zero
               ? ((delegate* unmanaged[Cdecl]<byte>)answer)() != 0
               : null;
    }
    finally
    {
      NativeMethods.FreeLibrary(library);
    }
#endif
  }

  /// <summary>Whether a file is in an x86-64-v4 folder, as the x86-64-v4 builds are.</summary>
  internal static bool InAnX64V4Folder(string file)
    => string.Equals(Path.GetFileName(Path.GetDirectoryName(file)),
                     X64V4Folder,
                     StringComparison.OrdinalIgnoreCase);

  /// <summary>The files a build is looked for in, in order: its x86-64-v4 build when the process runs
  /// it, then, for the metrics build, its baseline. The default build's baseline is not among them,
  /// since the platform's own search finds it.</summary>
  /// <param name="roots">The folders the package's layouts are looked for in.</param>
  /// <param name="build">The build asked for.</param>
  /// <param name="x64V4">Whether the process runs the x86-64-v4 build.</param>
  internal static IReadOnlyList<string> Candidates(IReadOnlyList<string> roots,
                                                   NativeEngineBuild     build,
                                                   bool                  x64V4)
  {
    var folders = Folders(roots,
                          build);
    var levels = x64V4
                   ? folders.Select(folder => Path.Combine(folder,
                                                           X64V4Folder))
                   : Enumerable.Empty<string>();
    return (build == NativeEngineBuild.Metrics
              ? levels.Concat(folders)
              : levels).Select(folder => Path.Combine(folder,
                                                      LibraryFileName()))
                       .ToList();
  }

  /// <summary>The first of the candidates that is there, or <c>null</c> when none is.</summary>
  internal static string? Locate(IEnumerable<string> candidates,
                                 Func<string, bool>  exists)
    => candidates.FirstOrDefault(exists);

  /// <summary>.NET Framework has no deps file and no runtime identifier: the package's targets file
  /// puts the engine in a folder named for the process architecture, loaded by its path so that the
  /// declarations find it by its name.</summary>
  private static void LoadBesideTheApplication()
  {
    var beside = Path.Combine(NativeMethods.EngineDirectory,
                              NativeMethods.Library + ".dll");
    if (File.Exists(beside))
    {
      NativeMethods.LoadLibrary(beside);
    }
  }

  /// <summary>Loads the build at a path, for the declarations to bind to.</summary>
  private static void Load(string path)
  {
#if NET5_0_OR_GREATER
    // Loaded now rather than when the first call asks: a library that exists and will not load
    // is reported here, where the resolver could only hand the runtime's own search a miss.
    var handle = NativeLibrary.Load(path);
    NativeLibrary.SetDllImportResolver(typeof(NativeLibrarySelection).Assembly,
                                       (name,
                                        _,
                                        _) => name == NativeMethods.Library
                                                ? handle
                                                : IntPtr.Zero);
#else
    // Only .NET Framework, whose loader matches a module that is loaded by its name, can be told
    // the build this way; elsewhere Select refuses the metrics build and RunsX64V4 the x86-64-v4
    // one.
    if (NativeMethods.LoadLibrary(path) == IntPtr.Zero)
    {
      throw new DllNotFoundException($"`{path}` could not be loaded (error {Marshal.GetLastWin32Error()})");
    }
#endif
  }

#if !NET5_0_OR_GREATER
  private static bool IsNetFramework()
    => RuntimeInformation.FrameworkDescription.StartsWith(".NET Framework",
                                                          StringComparison.Ordinal);
#endif

  /// <summary>The engine's file name on this operating system, the same for every build.</summary>
  internal static string LibraryFileName()
    => LibraryFileName(NativeMethods.Library);

  /// <summary>The file name of a library on this operating system.</summary>
  internal static string LibraryFileName(string library)
    => RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
         ? library + ".dll"
         : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
           ? "lib" + library + ".dylib"
           : "lib" + library + ".so";

  /// <summary>The application's folder, and this assembly's when it is another.</summary>
  private static IReadOnlyList<string> Roots()
  {
    var roots = new List<string>
                {
                  AppDomain.CurrentDomain.BaseDirectory ?? string.Empty,
                };
    var beside = typeof(NativeLibrarySelection).Assembly.Location;
    if (!string.IsNullOrEmpty(beside))
    {
      roots.Add(Path.GetDirectoryName(beside)!);
    }

    return roots.Distinct()
                .ToList();
  }

  /// <summary>Where a build is looked for, in order: the package's layout for .NET, the one its
  /// targets file makes for .NET Framework, and the folder beside the application. The metrics
  /// build is in a <c>metrics</c> folder of each.</summary>
  private static IReadOnlyList<string> Folders(IReadOnlyList<string> roots,
                                               NativeEngineBuild     build)
  {
    var metrics = build == NativeEngineBuild.Metrics
                    ? "metrics"
                    : string.Empty;
    return roots.SelectMany(root => new[]
                                    {
                                      Path.Combine(root,
                                                   "runtimes",
                                                   RuntimeIdentifier(),
                                                   metrics),
                                      Path.Combine(root,
                                                   RuntimeInformation.ProcessArchitecture.ToString()
                                                                     .ToLowerInvariant(),
                                                   metrics),
                                      Path.Combine(root,
                                                   metrics),
                                    })
                .ToList();
  }

  /// <summary>The portable runtime identifier of the process, as the package's folders name it.</summary>
  /// <remarks>Not <c>RuntimeInformation.RuntimeIdentifier</c>, which names the distribution a runtime
  /// was built for.</remarks>
  internal static string RuntimeIdentifier()
  {
    var architecture = RuntimeInformation.ProcessArchitecture switch
                       {
                         Architecture.X64   => "x64",
                         Architecture.X86   => "x86",
                         Architecture.Arm   => "arm",
                         Architecture.Arm64 => "arm64",
                         var other          => other.ToString()
                                                    .ToLowerInvariant(),
                       };
    if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
    {
      return "win-" + architecture;
    }

    if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
    {
      return "osx-" + architecture;
    }

    return (UsesMusl()
              ? "linux-musl-"
              : "linux-") + architecture;
  }

  private static bool UsesMusl()
  {
    try
    {
      return Directory.Exists("/lib") && Directory.EnumerateFiles("/lib",
                                                                  "ld-musl-*")
                                                  .Any();
    }
    catch (IOException)
    {
      return false;
    }
    catch (UnauthorizedAccessException)
    {
      return false;
    }
  }
}
