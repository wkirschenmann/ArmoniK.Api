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
using System.Reflection;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What both server fixtures need of a server the build produced.</summary>
internal static class BuiltServer
{
  /// <summary>The assembly the build recorded under <paramref name="key" />, as a full path.</summary>
  /// <remarks>The path is recorded relative to this project, so it is made absolute here rather
  /// than left to whatever directory the test host runs in.</remarks>
  internal static string Assembly(string key)
  {
    var recorded = typeof(BuiltServer).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
                                      .FirstOrDefault(metadata => metadata.Key == key)
                                     ?.Value;
    if (string.IsNullOrEmpty(recorded))
    {
      throw new InvalidOperationException($"the build recorded no {key}");
    }

    var assembly = Path.GetFullPath(recorded!);
    if (!File.Exists(assembly))
    {
      throw new FileNotFoundException($"{key} names an assembly that is not built: {assembly}",
                                      assembly);
    }

    return assembly;
  }

  internal static void Kill(Process process)
  {
    try
    {
      if (!process.HasExited)
      {
        process.Kill();
      }
    }
    catch (InvalidOperationException)
    {
      // It ended between the two, which is where it was going.
    }
  }
}
