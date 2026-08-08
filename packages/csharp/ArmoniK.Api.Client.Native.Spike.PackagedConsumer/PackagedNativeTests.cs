using System;
using System.IO;
using System.Runtime.InteropServices;

using ArmoniK.Api.Client.Native.Spike.Packaging;

using NUnit.Framework;

namespace ArmoniK.Api.Client.Native.Spike.PackagedConsumer;

/// <summary>
///   What a .NET Framework application gets from the package alone.
/// </summary>
[TestFixture]
public class PackagedNativeTests
{
  [Test]
  public void ThePackageDeliversBothArchitecturesBesideTheApplication()
  {
    var runtimes = Path.Combine(TestContext.CurrentContext.TestDirectory,
                                "runtimes");

    Assert.That(Directory.Exists(runtimes),
                Is.True,
                "the package's targets did not copy anything: on .NET Framework, `runtimes/` in a "
              + "package is inert without them");

    foreach (var rid in new[]
                        {
                          "win-x64",
                          "win-x86",
                        })
    {
      var library = Path.Combine(runtimes,
                                 rid,
                                 "native",
                                 "armonik_transport_ffi.dll");
      Assert.That(File.Exists(library),
                  Is.True,
                  $"{rid} is missing from the output");
      TestContext.WriteLine($"{rid}: {new FileInfo(library).Length:N0} bytes");
    }

    // And deliberately not in the output root: two builds of one library cannot share a directory,
    // which is the whole reason a loader has to choose.
    Assert.That(File.Exists(Path.Combine(TestContext.CurrentContext.TestDirectory,
                                         "armonik_transport_ffi.dll")),
                Is.False);
  }

  [Test]
  public void TheLoaderPicksTheBuildThisProcessCanActuallyUse()
  {
    var expected = IntPtr.Size == 8
                     ? "win-x64"
                     : "win-x86";
    Assert.That(NativeLoader.RuntimeIdentifier,
                Is.EqualTo(expected));

    var loaded = NativeLoader.Ensure();
    TestContext.WriteLine($"loaded {loaded} into a {IntPtr.Size * 8}-bit process");
    Assert.That(loaded,
                Does.Contain(expected));
  }

  [Test]
  public void APInvokeResolvesToThePreloadedLibrary()
  {
    // The point of the loader. Nothing named `armonik_transport_ffi.dll` sits beside this assembly,
    // so a bare `DllImport` has nowhere to find it - unless the module is already in the process
    // under that name, which is what `Ensure` arranges.
    NativeLoader.Ensure();

    var config = System.Text.Encoding.UTF8.GetBytes("{\"Endpoint\": \"http://127.0.0.1:1/\"}");
    var status = ak_client_create(config,
                                  (UIntPtr)config.Length,
                                  out var client,
                                  out _);

    Assert.That(status,
                Is.Zero,
                "AK_OK");
    Assert.That(client,
                Is.Not.EqualTo(IntPtr.Zero));
    ak_client_release(client);
  }

  [StructLayout(LayoutKind.Sequential)]
  private struct AkBytes
  {
    internal IntPtr  Ptr;
    internal UIntPtr Len;
    internal IntPtr  Owner;
  }

  [DllImport("armonik_transport_ffi",
             CallingConvention = CallingConvention.Cdecl)]
  private static extern int ak_client_create(byte[]      configJson,
                                             UIntPtr     len,
                                             out IntPtr  client,
                                             out AkBytes error);

  [DllImport("armonik_transport_ffi",
             CallingConvention = CallingConvention.Cdecl)]
  private static extern void ak_client_release(IntPtr client);
}
