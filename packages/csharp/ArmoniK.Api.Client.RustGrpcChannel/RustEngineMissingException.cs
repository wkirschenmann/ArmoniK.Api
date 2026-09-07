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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The native engine could not be loaded. The message says where it was looked for and
/// how this framework resolves it.</summary>
public sealed class RustEngineMissingException : Exception
{
  private RustEngineMissingException(string message,
                                     Exception inner)
    : base(message,
           inner)
  {
  }

  internal static RustEngineMissingException For(Exception inner)
  {
    var beside = NativeMethods.EngineDirectory;
    var how = RuntimeInformation.FrameworkDescription.StartsWith(".NET Framework",
                                                                 StringComparison.Ordinal)
                ? $"this is .NET Framework, which has no runtime-identifier probing: the package's targets file should have put it in `{beside}`"
                : "this is .NET, which resolves it by runtime identifier from `runtimes/<rid>/native` in the package";

    // The architecture as well as the width, because they are what disagree in the case this
    // message exists for: `EngineDirectory` picks x64 or x86 by pointer width alone, so an Arm64
    // process is told it looked in `x64` and can see for itself why nothing was there.
    return new RustEngineMissingException($"`{NativeMethods.Library}` could not be loaded for this {IntPtr.Size * 8}-bit {RuntimeInformation.ProcessArchitecture} process. "
                                          + how
                                          + $". Base directory: `{AppDomain.CurrentDomain.BaseDirectory}`.",
                                          inner);
  }
}
