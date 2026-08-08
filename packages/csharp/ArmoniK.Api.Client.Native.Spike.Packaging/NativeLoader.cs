using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

namespace ArmoniK.Api.Client.Native.Spike.Packaging;

/// <summary>
///   Loads the right build of the native library before the first P/Invoke reaches it.
/// </summary>
/// <remarks>
///   <para>
///     A .NET Core consumer needs none of this: the RID graph copies the matching
///     <c>runtimes/&lt;rid&gt;/native</c> asset next to the application, and a plain
///     <c>DllImport</c> finds it. A .NET Framework consumer gets nothing from <c>runtimes/</c>, and
///     an AnyCPU one does not even know its own bitness until it starts, so the choice can only be
///     made here.
///   </para>
///   <para>
///     The mechanism is the one <c>Grpc.Core</c> and <c>SQLitePCLRaw</c> use: load the library by
///     full path once, and every later <c>DllImport("armonik_transport_ffi")</c> binds to the module
///     already in the process rather than searching. It has to run before the first call - a static
///     constructor on the type holding the imports is the usual place.
///   </para>
/// </remarks>
public static class NativeLoader
{
  private static readonly object Gate = new();
  private static          bool   loaded_;

  /// <summary>The file name of the native library, without extension or prefix.</summary>
  public const string LibraryName = "armonik_transport_ffi";

  /// <summary>The runtime identifier this process needs a build for.</summary>
  public static string RuntimeIdentifier
    => IntPtr.Size == 8
         ? "win-x64"
         : "win-x86";

  /// <summary>
  ///   Make sure the native library is loaded, and answer with the path it came from.
  /// </summary>
  /// <remarks>
  ///   Idempotent, and safe to call from several threads: two add-ins in one Excel process both
  ///   reach this, and only the first does any work.
  /// </remarks>
  public static string Ensure()
  {
    var path = ExpectedPath();
    lock (Gate)
    {
      if (loaded_)
      {
        return path;
      }

      if (!File.Exists(path))
      {
        throw new FileNotFoundException($"the {RuntimeIdentifier} build of {LibraryName} is missing; the package should have copied it beside the application",
                                        path);
      }

      if (LoadLibraryW(path) == IntPtr.Zero)
      {
        throw new Win32Exception(Marshal.GetLastWin32Error(),
                                 $"could not load {path}");
      }

      loaded_ = true;
      return path;
    }
  }

  /// <summary>Where the package's targets put the build this process needs.</summary>
  public static string ExpectedPath()
    => Path.Combine(AppDomain.CurrentDomain.BaseDirectory,
                    "runtimes",
                    RuntimeIdentifier,
                    "native",
                    LibraryName + ".dll");

  [DllImport("kernel32",
             CharSet        = CharSet.Unicode,
             SetLastError   = true,
             ExactSpelling  = true)]
  private static extern IntPtr LoadLibraryW(string path);
}
