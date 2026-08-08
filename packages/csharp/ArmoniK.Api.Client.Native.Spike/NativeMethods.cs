using System;
using System.Runtime.InteropServices;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   The `ak_*` entry points of `armonik_transport_ffi`, as declared in
///   `packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h`.
/// </summary>
/// <remarks>
///   `Cdecl` everywhere, spelled out rather than left to the default: it is what the header says,
///   and on x86 - which this will have to run on, Office 32-bit being what it is - the default
///   would be wrong.
/// </remarks>
internal static class NativeMethods
{
  private const string Library = "armonik_transport_ffi";

  /// <summary>An owned buffer the library handed over. Released with exactly one ak_bytes_free.</summary>
  [StructLayout(LayoutKind.Sequential)]
  internal struct AkBytes
  {
    internal IntPtr  Ptr;
    internal UIntPtr Len;
    internal IntPtr  Owner;
  }

  /// <summary>A borrowed view. On the event path it is valid only for the duration of the call.</summary>
  [StructLayout(LayoutKind.Sequential)]
  internal struct AkBytesIn
  {
    internal IntPtr  Ptr;
    internal UIntPtr Len;
  }

  /// <summary>
  ///   The event callback. Invoked from the library's own threads, serialised per request.
  /// </summary>
  /// <remarks>
  ///   The instance handed to <see cref="ak_request_start" /> has to stay rooted for as long as the
  ///   library may call it; a delegate marshalled to a function pointer is not kept alive by the
  ///   native side holding the pointer.
  /// </remarks>
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
  internal delegate void OnEvent(IntPtr ctx,
                                 int    kind,
                                 AkBytesIn payload,
                                 int    code);

  internal static class Status
  {
    internal const int Ok               = 0;
    internal const int NullArgument     = -1;
    internal const int InvalidUtf8      = -2;
    internal const int InvalidConfig    = -3;
    internal const int ConnectionFailed = -4;
    internal const int InvalidHandle    = -5;
    internal const int InvalidState     = -6;
    internal const int Internal         = -8;
    internal const int InternalPanic    = -9;
    internal const int Cancelled        = -10;
    internal const int Timeout          = -11;
    internal const int Transport        = -12;
  }

  internal static class Event
  {
    internal const int ResponseHeaders = 1;
    internal const int WriteDone       = 2;
    internal const int ReadDone        = 3;
    internal const int Completed       = 4;
  }

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_client_create(byte[]      configJson,
                                              UIntPtr     len,
                                              out IntPtr  client,
                                              out AkBytes error);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_client_release(IntPtr client);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_request_start(IntPtr      client,
                                              byte[]      headersBlob,
                                              UIntPtr     len,
                                              OnEvent     onEvent,
                                              IntPtr      ctx,
                                              out IntPtr  request,
                                              out AkBytes error);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_request_write(IntPtr  request,
                                              byte[]  data,
                                              UIntPtr len);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_request_close_send(IntPtr request);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_request_read(IntPtr request);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_request_cancel(IntPtr request);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_request_release(IntPtr request);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_bytes_free(AkBytes bytes);

  /// <summary>Read an owned buffer out as text and release it.</summary>
  internal static string TakeMessage(AkBytes bytes)
  {
    if (bytes.Ptr == IntPtr.Zero)
    {
      return string.Empty;
    }

    var buffer = new byte[(int)bytes.Len];
    Marshal.Copy(bytes.Ptr,
                 buffer,
                 0,
                 buffer.Length);
    ak_bytes_free(bytes);
    return System.Text.Encoding.UTF8.GetString(buffer);
  }
}
