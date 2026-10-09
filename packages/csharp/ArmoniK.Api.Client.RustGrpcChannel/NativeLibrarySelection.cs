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
public static class NativeLibrarySelection
{
  private static readonly object Gate = new();

  private static NativeEngineBuild requested_ = NativeEngineBuild.Default;

  private static bool committed_;

  private static RustEngineMissingException? failure_;

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
  ///   other than .NET Framework.
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
        if (requested_ == NativeEngineBuild.Metrics)
        {
          PrepareMetrics();
        }
        else
        {
          LoadBesideTheApplication();
        }
      }
      catch (Exception error) when (error is not RustEngineMissingException)
      {
        // The default build is found by the platform's own search when this fails.
        if (requested_ == NativeEngineBuild.Metrics)
        {
          failure_ = RustEngineMissingException.ForBuild(requested_,
                                                         null,
                                                         error);
        }
      }
      catch (RustEngineMissingException missing)
      {
        failure_ = missing;
      }
    }
  }

  /// <summary>Raises why the build asked for could not be found, when it could not.</summary>
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

  private static void PrepareMetrics()
  {
    var folders = Folders();
    var path = folders.Select(folder => Path.Combine(folder,
                                                     LibraryFileName()))
                      .FirstOrDefault(File.Exists);
    if (path is null)
    {
      throw RustEngineMissingException.ForBuild(NativeEngineBuild.Metrics,
                                                folders,
                                                null);
    }

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
    // the build this way; Select refuses the other runtimes.
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

  private static string LibraryFileName()
    => RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
         ? NativeMethods.Library + ".dll"
         : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
           ? "lib" + NativeMethods.Library + ".dylib"
           : "lib" + NativeMethods.Library + ".so";

  /// <summary>Where the metrics build is looked for, in order: the package's layout for .NET, the one
  /// its targets file makes for .NET Framework, and a folder of that name beside the application.</summary>
  private static IReadOnlyList<string> Folders()
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
                .SelectMany(root => new[]
                                    {
                                      Path.Combine(root,
                                                   "runtimes",
                                                   RuntimeIdentifier(),
                                                   "metrics"),
                                      Path.Combine(root,
                                                   RuntimeInformation.ProcessArchitecture.ToString()
                                                                     .ToLowerInvariant(),
                                                   "metrics"),
                                      Path.Combine(root,
                                                   "metrics"),
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
