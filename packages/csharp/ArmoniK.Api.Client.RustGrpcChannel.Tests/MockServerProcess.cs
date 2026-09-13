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
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Text;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>ArmoniK.Api.Mock, started by the tests that need a real ArmoniK server.</summary>
/// <remarks>The mock takes its ports from configuration and announces nothing, so the ports are
/// chosen here and readiness is read from its HTTP root - the one thing it serves that is not
/// gRPC. Choosing a port and then handing it over is not race-free: another process may take it
/// in between. The alternative is for the mock to bind zero and print what it got, which is what
/// the echo server does, and that is a change to a component these tests do not own.</remarks>
internal sealed class MockServerProcess : IDisposable
{
  private static readonly TimeSpan StartTimeout = TimeSpan.FromSeconds(60);

  private readonly Process process_;

  private MockServerProcess(Process process,
                            string  endpoint)
  {
    process_ = process;
    Endpoint = endpoint;
  }

  /// <summary>Where its gRPC services listen, as this engine needs it: plain HTTP/2.</summary>
  internal string Endpoint { get; }

  internal static MockServerProcess Start()
  {
    var assembly = BuiltServer.Assembly("MockServerAssembly");
    var (grpcPort, httpPort) = FreePorts();

    var process = new Process
                  {
                    StartInfo = new ProcessStartInfo("dotnet",
                                                     $"\"{assembly}\" --Grpc:Port={grpcPort} --Http:Port={httpPort}")
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

    // Both pipes drained from the start, so a chatty startup cannot fill one and block the server
    // on it while this waits for a port that will never open.
    var said = new StringBuilder();
    process.OutputDataReceived += (_,
                                   line) => Keep(said,
                                                 line.Data);
    process.ErrorDataReceived += (_,
                                  line) => Keep(said,
                                                line.Data);
    process.BeginOutputReadLine();
    process.BeginErrorReadLine();

    try
    {
      Await(process,
            httpPort,
            said);
      return new MockServerProcess(process,
                                   $"http://127.0.0.1:{grpcPort}");
    }
    catch
    {
      BuiltServer.Kill(process);
      throw;
    }
  }

  private static void Keep(StringBuilder said,
                           string?       line)
  {
    if (line is null)
    {
      return;
    }

    lock (said)
    {
      said.AppendLine(line);
    }
  }

  /// <summary>Waits for Kestrel to answer, which is when both its ports are listening.</summary>
  private static void Await(Process       process,
                            int           httpPort,
                            StringBuilder said)
  {
    using var probe = new HttpClient
                      {
                        Timeout = TimeSpan.FromSeconds(2),
                      };
    var deadline = DateTime.UtcNow + StartTimeout;

    while (DateTime.UtcNow < deadline)
    {
      if (process.HasExited)
      {
        throw new InvalidOperationException($"the mock ended before it listened (exit {process.ExitCode}){Reported(said)}");
      }

      try
      {
        probe.GetAsync($"http://127.0.0.1:{httpPort}/")
             .GetAwaiter()
             .GetResult()
             .Dispose();
        return;
      }
      catch (Exception)
      {
        // Not up yet. Anything the probe throws before the deadline is that and nothing else:
        // a real failure is the exit above, or the timeout below.
        System.Threading.Thread.Sleep(100);
      }
    }

    throw new TimeoutException($"the mock did not listen on {httpPort} in {StartTimeout}{Reported(said)}");
  }

  private static string Reported(StringBuilder said)
  {
    string reported;
    lock (said)
    {
      reported = said.ToString()
                     .Trim();
    }

    return reported.Length == 0
             ? string.Empty
             : Environment.NewLine + reported;
  }

  /// <summary>Two ports nothing is listening on as of now, and different from each other.</summary>
  /// <remarks>
  ///   Both listeners are held until both ports are read. Taken one at a time, the second call can
  ///   be handed the first one's port back, and the mock then binds a single listener speaking
  ///   HTTP/1 and HTTP/2 rather than one of each - which it does on purpose, for a caller that
  ///   asks for one port twice. This engine cannot use that one: it speaks h2c with prior
  ///   knowledge, and a cleartext endpoint that also admits HTTP/1 answers it with `the request
  ///   did not reach the peer: http2 error`.
  ///   <para>
  ///     What holding them does not remove is the window between the release below and the mock's
  ///     own bind, where another process may take either port.
  ///   </para>
  /// </remarks>
  private static (int Grpc, int Http) FreePorts()
  {
    var grpc = new TcpListener(IPAddress.Loopback,
                               0);
    var http = new TcpListener(IPAddress.Loopback,
                               0);
    grpc.Start();
    http.Start();

    var ports = (((IPEndPoint)grpc.LocalEndpoint).Port, ((IPEndPoint)http.LocalEndpoint).Port);

    grpc.Stop();
    http.Stop();
    return ports;
  }

  public void Dispose()
  {
    BuiltServer.Kill(process_);
    process_.Dispose();
  }
}
