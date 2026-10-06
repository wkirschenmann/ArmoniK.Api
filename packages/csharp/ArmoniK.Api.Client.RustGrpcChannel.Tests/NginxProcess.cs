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
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>An nginx in front of the echo server, as ArmoniK deployments put one.</summary>
/// <remarks>
///   The executable is named by ARMONIK_TEST_NGINX, nginx 1.25.1 or later for `http2 on`; without
///   it a test that needs one is ignored. Plain HTTP/2 on both sides, its own prefix directory,
///   and an access log that names each request's client connection and the `x-call` header, so a
///   test reads back where every call went.
/// </remarks>
internal sealed class NginxProcess : IDisposable
{
  internal const string Variable = "ARMONIK_TEST_NGINX";

  private static readonly TimeSpan Patience = TimeSpan.FromSeconds(10);

  private readonly StringBuilder complaints_;
  private readonly string        executable_;
  private readonly Process       master_;
  private readonly string        prefix_;

  private NginxProcess(string        executable,
                       string        prefix,
                       Process       master,
                       StringBuilder complaints,
                       int           port)
  {
    executable_ = executable;
    prefix_     = prefix;
    master_     = master;
    complaints_ = complaints;
    Endpoint    = $"http://127.0.0.1:{port}";
  }

  internal string Endpoint { get; }

  internal static string? Executable
    => Environment.GetEnvironmentVariable(Variable) is { Length: > 0 } path
         ? path
         : null;

  /// <summary>Starts one in front of <paramref name="upstream" />.</summary>
  /// <param name="upstream">The echo server's endpoint, `http://host:port`.</param>
  /// <param name="directives">Server-level directives, e.g. `keepalive_requests 2;`.</param>
  internal static NginxProcess Start(string upstream,
                                     string directives)
  {
    var executable = Executable ?? throw new InvalidOperationException($"{Variable} is not set");
    var prefix = Path.Combine(Path.GetTempPath(),
                              $"armonik-test-nginx-{Guid.NewGuid():N}");
    Process master;
    var complaints = new StringBuilder();
    var port       = FreePort();
    try
    {
      foreach (var directory in new[]
                                {
                                  "conf",
                                  "logs",
                                  "temp",
                                })
      {
        Directory.CreateDirectory(Path.Combine(prefix,
                                               directory));
      }

      var target = new Uri(upstream);
      // In the foreground: on Linux nginx otherwise forks, and the process started here, the one
      // its exit is waited on, would be gone at once.
      File.WriteAllText(Path.Combine(prefix,
                                     "conf",
                                     "nginx.conf"),
                        $@"daemon off;
worker_processes 1;
error_log logs/error.log info;
pid logs/nginx.pid;
events {{ worker_connections 64; }}
http {{
  client_body_temp_path temp/client_body;
  proxy_temp_path temp/proxy;
  fastcgi_temp_path temp/fastcgi;
  uwsgi_temp_path temp/uwsgi;
  scgi_temp_path temp/scgi;
  log_format calls '$connection $http_x_call $status';
  access_log logs/access.log calls;
  server {{
    listen 127.0.0.1:{port};
    http2 on;
    {directives}
    location / {{ grpc_pass grpc://{target.Host}:{target.Port}; }}
  }}
}}
");

      // Its stderr kept, because nginx reports a bad directive there before its error log is
      // open.
      master = Run(executable,
                   prefix,
                   complaints);
    }
    catch
    {
      Delete(prefix);
      throw;
    }

    var nginx = new NginxProcess(executable,
                                 prefix,
                                 master,
                                 complaints,
                                 port);
    try
    {
      nginx.WaitUntilListening(port);
      return nginx;
    }
    catch
    {
      nginx.Dispose();
      throw;
    }
  }

  /// <summary>Stops nginx, so that its logs are whole, and reads the access log.</summary>
  /// <returns>One entry per request: the client connection's serial number, `x-call`, and the status.</returns>
  internal IReadOnlyList<(string Connection, string Call, string Status)> Calls()
  {
    Stop();
    return Read("access.log")
           .Split('\n')
           .Select(line => line.Trim()
                               .Split(' '))
           .Where(fields => fields.Length == 3)
           .Select(fields => (fields[0], fields[1], fields[2]))
           .ToList();
  }

  /// <summary>Stops nginx and reads what it logged at info level and above: why it closed each connection.</summary>
  internal string ErrorLog()
  {
    Stop();
    return Read("error.log");
  }

  public void Dispose()
  {
    try
    {
      Stop();
    }
    finally
    {
      master_.Dispose();
      Delete(prefix_);
    }
  }

  // nginx runs as a master and its worker, and `-s stop` reaches both through the pid file. The
  // master exits once the worker has, so its exit is when the logs are whole. One that does not
  // stop is killed with its worker, and reads as stopped from then on.
  private void Stop()
  {
    if (master_.HasExited)
    {
      return;
    }

    using (var stop = Run(executable_,
                          prefix_,
                          null,
                          "-s stop"))
    {
      stop.WaitForExit((int)Patience.TotalMilliseconds);
    }

    if (master_.WaitForExit((int)Patience.TotalMilliseconds))
    {
      return;
    }

#if NET
    master_.Kill(true);
#else
    // Framework kills the master alone, and its worker may outlive it.
    master_.Kill();
#endif
    master_.WaitForExit((int)Patience.TotalMilliseconds);
    throw new TimeoutException($"nginx did not stop in {Patience}, and was killed");
  }

  // Opened with sharing, because nginx may still hold the log.
  private string Read(string log)
  {
    var path = Path.Combine(prefix_,
                            "logs",
                            log);
    if (!File.Exists(path))
    {
      return string.Empty;
    }

    using var stream = new FileStream(path,
                                      FileMode.Open,
                                      FileAccess.Read,
                                      FileShare.ReadWrite | FileShare.Delete);
    using var reader = new StreamReader(stream);
    return reader.ReadToEnd();
  }

  private void WaitUntilListening(int port)
  {
    var deadline = DateTime.UtcNow + Patience;
    while (true)
    {
      try
      {
        using var probe = new TcpClient();
        probe.Connect(IPAddress.Loopback,
                      port);
        // Another process may hold a port nginx failed to bind.
        if (!master_.HasExited)
        {
          return;
        }
      }
      catch (SocketException) when (DateTime.UtcNow < deadline && !master_.HasExited)
      {
        Thread.Sleep(50);
        continue;
      }
      catch (SocketException)
      {
        // Past the deadline, or nginx is gone.
      }

      string said;
      lock (complaints_)
      {
        said = complaints_.ToString();
      }

      throw new TimeoutException($"nginx was not serving on {port} within {Patience}: {said}{Read("error.log")}");
    }
  }

  // nginx wants the prefix to end with a separator, and a quoted argument cannot end with a
  // backslash, so it is written with forward slashes, which Windows' nginx takes too.
  private static Process Run(string         executable,
                             string         prefix,
                             StringBuilder? complaints,
                             string         arguments = "")
  {
    var process = Process.Start(new ProcessStartInfo(executable,
                                                     $"-p \"{prefix.Replace('\\', '/')}/\" -c conf/nginx.conf {arguments}")
                                {
                                  UseShellExecute       = false,
                                  CreateNoWindow        = true,
                                  WorkingDirectory      = prefix,
                                  RedirectStandardError = complaints is not null,
                                }) ?? throw new InvalidOperationException($"{executable} did not start");
    if (complaints is not null)
    {
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
    }

    return process;
  }

  // It is in the temp folder, and a scanner or an exiting worker may hold a file a moment longer.
  private static void Delete(string prefix)
  {
    try
    {
      Directory.Delete(prefix,
                       true);
    }
    catch (IOException)
    {
    }
    catch (UnauthorizedAccessException)
    {
    }
  }

  private static int FreePort()
  {
    var listener = new TcpListener(IPAddress.Loopback,
                                   0);
    listener.Start();
    try
    {
      return ((IPEndPoint)listener.LocalEndpoint).Port;
    }
    finally
    {
      listener.Stop();
    }
  }
}
