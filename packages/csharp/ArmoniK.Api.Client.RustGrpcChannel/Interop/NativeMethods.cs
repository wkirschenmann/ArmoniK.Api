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
using System.Text;

namespace ArmoniK.Api.Client.RustGrpcChannel.Interop;

/// <summary>What the generated declarations in NativeMethods.g.cs do not carry: which library
/// they bind to, and where .NET Framework finds it.</summary>
internal static unsafe partial class NativeMethods
{
  internal const string Library = "armonik_transport_ffi";

  // The name every generated DllImport binds to.
  private const string __DllName = Library;

  /// <summary>Where the package's targets file copies the engine for .NET Framework: a folder
  /// named for the process architecture, as the Windows runtime identifiers name it.</summary>
  internal static string EngineDirectory
    => Path.Combine(AppDomain.CurrentDomain.BaseDirectory ?? string.Empty,
                    RuntimeInformation.ProcessArchitecture.ToString()
                                      .ToLowerInvariant());

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
    }
  }

  /// <summary>The engine's log callback, which the generated declarations carry as a plain pointer.</summary>
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
  internal delegate void LogCallback(void*          logCtx,
                                     ak_log_record* record);

  [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
  private static extern IntPtr LoadLibrary(string path);
}

internal unsafe partial struct ak_error
{
  /// <summary>The message a refusal wrote, given back to the library as it is read.</summary>
  /// <remarks>The detail is cleared with it, so a second read is empty rather than a read of
  /// freed memory.</remarks>
  internal string Take()
  {
    var message = detail.ptr == null
                    ? string.Empty
                    : Encoding.UTF8.GetString(detail.ptr,
                                              checked((int)detail.len));
    NativeMethods.ak_error_release(detail);
    detail = default;
    return message;
  }
}

internal unsafe partial struct ak_bytes_in
{
  /// <summary>A view of a pinned array. An empty one points nowhere, which is what `fixed`
  /// yields for an empty array and what the ABI reads for a zero length.</summary>
  internal static ak_bytes_in Borrow(byte* pinned,
                                     int   length)
    => new()
       {
         ptr = length == 0
                 ? null
                 : pinned,
         len = (nuint)length,
       };
}
