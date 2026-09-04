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
using System.Runtime.InteropServices;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   The `ak_*` entry points, as declared in
///   `packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h`.
/// </summary>
/// <remarks>
///   The whole header, declared whether or not this binding calls it yet, so the two can be
///   diffed against each other. `Cdecl` is spelled out rather than left to the default: it is
///   what the header says, and on x86 the default would be wrong.
/// </remarks>
internal static class NativeMethods
{
  internal const string Library = "armonik_transport_ffi";

  /// <summary>
  ///   Where a .NET Framework consumer's engine is, which is the layout the package's targets
  ///   file writes. Read by the diagnostic too, so a change to the layout cannot leave the error
  ///   message naming a directory nothing looks in.
  /// </summary>
  internal static string EngineDirectory
    => Path.Combine(AppDomain.CurrentDomain.BaseDirectory ?? string.Empty,
                    IntPtr.Size == 8
                      ? "x64"
                      : "x86");

  /// <summary>The version this binding is written against.</summary>
  internal const int AbiVersion = 1;

  /// <summary>
  ///   Loads the engine for this process's word size, where the framework will not.
  /// </summary>
  /// <remarks>
  ///   .NET resolves a native asset by runtime identifier out of the package, so there is nothing
  ///   to do there. .NET Framework has no such thing: the package's targets file puts both
  ///   architectures in `x86` and `x64` beside the application, and this picks one and loads it by
  ///   full path, after which the runtime's own probe for the bare name finds it already in the
  ///   process. `IntPtr.Size` is the right question in every case, AnyCPU included, which is why
  ///   there is one path here and not one per `PlatformTarget`.
  ///   <para>
  ///     A static constructor because it must run before the first P/Invoke of this class, and
  ///     nothing here may throw: a failed one turns every later call into a
  ///     <see cref="TypeInitializationException" /> naming none of this.
  ///   </para>
  /// </remarks>
  static NativeMethods()
  {
    try
    {
      var beside = Path.Combine(EngineDirectory,
                                Library + ".dll");
      if (File.Exists(beside))
      {
        LoadLibrary(beside);
      }
    }
    catch
    {
      // Whatever the reason, the load that matters is the one the first DllImport does, and it
      // reports its own failure. Guessing here would only hide that one.
    }
  }

  [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
  private static extern IntPtr LoadLibrary(string path);

  internal enum AkStatus
  {
    Ok               = 0,
    HandleStale      = 1,
    SlotBusy         = 2,
    InvalidArg       = 3,
    Internal         = 4,
    BudgetBusy       = 5,
    InvalidState     = 6,
    MessageTooLarge  = 7,
  }

  internal enum AkRuntimeState
  {
    Running           = 1,
    GrpcStopping      = 2,
    GrpcStopped       = 3,
    Quiescent         = 4,
    FailedUnquiesced  = 5,
  }

  internal enum AkChannelState
  {
    None    = 0,
    Open    = 1,
    Closing = 2,
    Closed  = 3,
  }

  internal enum AkEventKind
  {
    InitialMetadata   = 1,
    Message           = 2,
    Status            = 3,
    WriteDone         = 4,
    ShutdownComplete  = 5,
    ResourcesReleased = 6,
  }

  internal enum AkHostDebt
  {
    NothingToReturn = 0,
    MustReturn      = 1,
  }

  /// <summary>Bytes lent to the library for the duration of one downcall.</summary>
  [StructLayout(LayoutKind.Sequential)]
  internal struct AkBytesIn
  {
    internal IntPtr Ptr;
    internal UIntPtr Len;
  }

  /// <summary>
  ///   A view owned by this side until <see cref="ak_event_consumed" />. It is <c>Owner</c> and not
  ///   <c>Ptr</c> that identifies the allocation, and a null <c>Owner</c> - not a zero length - is
  ///   what says there is nothing to give back.
  /// </summary>
  [StructLayout(LayoutKind.Sequential)]
  internal struct AkBytes
  {
    internal IntPtr Ptr;
    internal UIntPtr Len;
    internal IntPtr Owner;
  }

  /// <summary>A buffer lent out of a call's arena, given back exactly once.</summary>
  [StructLayout(LayoutKind.Sequential)]
  internal struct AkBuffer
  {
    internal IntPtr Ptr;
    internal UIntPtr Len;
    internal IntPtr Owner;
  }

  [StructLayout(LayoutKind.Sequential)]
  internal struct AkEvent
  {
    internal AkEventKind Kind;
    internal AkBytes Payload;
    internal int StatusCode;
    internal AkHostDebt HostDebt;
  }

  [StructLayout(LayoutKind.Sequential)]
  internal struct AkRuntimeConfig
  {
    internal uint StructSize;
    internal uint WorkerThreads;
    internal ulong MemoryCeiling;
  }

  [StructLayout(LayoutKind.Sequential)]
  internal struct AkCallStartOptions
  {
    internal uint StructSize;
    internal AkBytesIn Method;
    internal AkBytesIn Metadata;
  }

  [StructLayout(LayoutKind.Sequential)]
  internal struct AkMemoryUsage
  {
    internal ulong BytesUsed;
    internal ulong Ceiling;
  }

  [StructLayout(LayoutKind.Sequential)]
  internal struct AkCallDebt
  {
    internal uint PayloadsOwed;
    internal uint BuffersLent;
    internal uint CallbacksInFlight;
    internal int TerminalDelivered;
  }

  /// <summary>
  ///   Where every event of a runtime arrives, on one of the library's own threads.
  /// </summary>
  /// <remarks>
  ///   A delegate marshalled to a function pointer is not kept alive by the native side holding
  ///   that pointer, so whoever passes one has to root it for as long as the runtime may call it.
  /// </remarks>
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
  internal delegate void AkCallback(IntPtr runtimeCtx,
                                    IntPtr callCtx,
                                    IntPtr @event);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_runtime_create(ref AkRuntimeConfig config,
                                                    AkCallback callback,
                                                    IntPtr runtimeCtx,
                                                    out ulong outRuntime);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkRuntimeState ak_runtime_status(ulong runtime);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_runtime_begin_shutdown(ulong runtime);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_runtime_destroy(ulong runtime);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_runtime_memory_usage(ulong runtime,
                                                          out AkMemoryUsage outUsage);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_channel_create(ulong runtime,
                                                    AkBytesIn configJson,
                                                    out ulong outChannel);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_channel_release(ulong channel);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkChannelState ak_channel_status(ulong channel);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_call_start(ulong channel,
                                                ref AkCallStartOptions options,
                                                IntPtr callCtx,
                                                out ulong outCall);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_get_call_buffer(ulong call,
                                                     UIntPtr len,
                                                     out AkBuffer outBuffer);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_call_send_message(ulong call,
                                                       AkBuffer buffer);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_return_call_buffer(AkBuffer buffer);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_call_end_send(ulong call);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_call_cancel(ulong call);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern AkStatus ak_call_debt_of(ulong call,
                                                  out AkCallDebt outDebt);

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern int ak_abi_version();

  [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
  internal static extern void ak_event_consumed(AkBytes payload);

}
