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
using System.Text;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

internal sealed class EchoServerProcess : IDisposable
{
  private const string EndpointPrefix = "ENDPOINT ";
  private static readonly TimeSpan StartTimeout = TimeSpan.FromSeconds(30);

  private readonly Process process_;

  private EchoServerProcess(Process process,
                            string endpoint)
  {
    process_ = process;
    Endpoint = endpoint;
  }

  internal string Endpoint { get; }

  internal static EchoServerProcess Start()
  {
    var assembly = ServerAssembly();
    var process = new Process
                  {
                    StartInfo = new ProcessStartInfo("dotnet",
                                                     $"\"{assembly}\"")
                                {
                                  RedirectStandardOutput = true,
                                  RedirectStandardError  = true,
                                  UseShellExecute        = false,
                                  CreateNoWindow         = true,
                                },
                  };

    if (!process.Start())
    {
      throw new InvalidOperationException($"`dotnet {assembly}` did not start");
    }

    // Drained from the start, and kept, so a chatty startup cannot fill the pipe while nothing
    // reads it - the server would then block on its own stderr, say nothing on stdout, and the
    // whole start budget would go to a deadlock reported as "the server said nothing".
    var complaints = new StringBuilder();
    process.ErrorDataReceived += (_,
                                  line) =>
                                 {
                                   if (line.Data is not null)
                                   {
                                     lock (complaints)
                                     {
                                       complaints.AppendLine(line.Data);
                                     }
                                   }
                                 };
    process.BeginErrorReadLine();

    try
    {
      return new EchoServerProcess(process,
                                   ReadEndpoint(process,
                                                complaints));
    }
    catch
    {
      Kill(process);
      throw;
    }
  }

  private static string ReadEndpoint(Process       process,
                                     StringBuilder complaints)
  {
    var deadline = DateTime.UtcNow + StartTimeout;
    while (true)
    {
      // Bounded, because `ReadLine` is not: a server that starts and says nothing would otherwise
      // hold the fixture for as long as it lives, and the timeout below could never be reached.
      var reading = process.StandardOutput.ReadLineAsync();
      var left = deadline - DateTime.UtcNow;
      if (left <= TimeSpan.Zero || !reading.Wait(left))
      {
        throw new TimeoutException($"the server said nothing in {StartTimeout}");
      }

      var line = reading.Result;
      if (line is null)
      {
        string complaint;
        lock (complaints)
        {
          complaint = complaints.ToString()
                                .Trim();
        }

        throw new InvalidOperationException($"the server ended before saying where it listens (exit {process.ExitCode})"
                                            + (complaint.Length == 0
                                                 ? string.Empty
                                                 : Environment.NewLine + complaint));
      }

      if (line.StartsWith(EndpointPrefix,
                          StringComparison.Ordinal))
      {
        return line.Substring(EndpointPrefix.Length)
                   .Trim();
      }
    }
  }

  private static string ServerAssembly()
  {
    var recorded = typeof(EchoServerProcess).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
                                            .FirstOrDefault(metadata => metadata.Key == "TestServerAssembly")
                                           ?.Value;
    if (string.IsNullOrEmpty(recorded))
    {
      throw new InvalidOperationException("the build recorded no TestServerAssembly");
    }

    var assembly = Path.GetFullPath(recorded!);
    if (!File.Exists(assembly))
    {
      throw new FileNotFoundException($"the test server is not built: {assembly}",
                                      assembly);
    }

    return assembly;
  }

  private static void Kill(Process process)
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
    }
  }

  public void Dispose()
  {
    Kill(process_);
    process_.Dispose();
  }
}
