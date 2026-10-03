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
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.RustGrpcChannel.Tests;
using ArmoniK.Api.Client.Submitter;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel.Benchmarks;

/// <summary>
///   One transport, measured against the test server over TLS: unary latency, server-streaming
///   throughput, and what the process holds once both have run. Prints one line of results.
/// </summary>
/// <remarks>
///   One transport per process, so that what one leaves allocated is not counted against the
///   other. TLS for both, because grpc-dotnet on .NET Framework speaks HTTP/2 only over it.
/// </remarks>
public static class Program
{
  private const int WarmupCalls   = 1_000;
  private const int MeasuredCalls = 10_000;
  private const int StreamRuns    = 5;
  private const int ChunkCount    = 2_000;
  private const int ChunkSize     = 64 * 1024;

  public static async Task<int> Main(string[] args)
  {
    if (args.Length != 1 || (args[0] != "native" && args[0] != "managed"))
    {
      Console.Error.WriteLine("usage: Benchmarks native|managed");
      return 2;
    }

    var transport = args[0];
    using var server = Server.Start();

    var before = Footprint.Read();
    var (channel, release) = Open(transport,
                                  server);
    try
    {
      var client = new Echo.EchoClient(channel);

      for (var call = 0; call < WarmupCalls; call++)
      {
        await client.SayAsync(new EchoRequest
                              {
                                Text = "x",
                              });
      }

      var latencies = new double[MeasuredCalls];
      var clock     = new Stopwatch();
      for (var call = 0; call < MeasuredCalls; call++)
      {
        clock.Restart();
        await client.SayAsync(new EchoRequest
                              {
                                Text = "x",
                              });
        latencies[call] = clock.Elapsed.TotalMilliseconds * 1000;
      }

      Array.Sort(latencies);

      var throughputs = new List<double>();
      for (var run = 0; run < StreamRuns; run++)
      {
        clock.Restart();
        long received = 0;
        using var stream = client.Stream(new StreamRequest
                                         {
                                           Count = ChunkCount,
                                           Size  = ChunkSize,
                                         });
        while (await stream.ResponseStream.MoveNext()
                           .ConfigureAwait(false))
        {
          received += stream.ResponseStream.Current.Data.Length;
        }

        throughputs.Add(received / clock.Elapsed.TotalSeconds / (1024 * 1024));
      }

      throughputs.Sort();

      var after = Footprint.Read();
      Console.WriteLine(string.Format(CultureInfo.InvariantCulture,
                                      "{0} {1} p50_us={2:F0} p95_us={3:F0} p99_us={4:F0} stream_mib_s={5:F0} private_mib={6:F1} managed_mib={7:F1}",
                                      transport,
                                      Framework(),
                                      Percentile(latencies,
                                                 0.50),
                                      Percentile(latencies,
                                                 0.95),
                                      Percentile(latencies,
                                                 0.99),
                                      throughputs[throughputs.Count / 2],
                                      (after.PrivateBytes - before.PrivateBytes) / (1024.0 * 1024),
                                      (after.ManagedBytes - before.ManagedBytes) / (1024.0 * 1024)));
      return 0;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  private static (ChannelBase Channel, Func<Task> Release) Open(string transport,
                                                               Server server)
  {
    if (transport == "native")
    {
      var runtime = NativeRuntime.Create();
      var channel = runtime.Channel(server.Endpoint,
                                    new ChannelOptions
                                    {
                                      Transport = new TransportOptions
                                                  {
                                                    Tls = new TlsOptions
                                                          {
                                                            CaCertPath = server.Authority,
                                                          },
                                                  },
                                    });
      return (channel, async () =>
                       {
                         await channel.DisposeAsync()
                                      .ConfigureAwait(false);
                         await runtime.DisposeAsync()
                                      .ConfigureAwait(false);
                       });
    }

    var managed = GrpcChannelFactory.CreateChannel(new GrpcClient
                                                   {
                                                     Endpoint = server.Endpoint,
                                                     CaCert   = server.Authority,
                                                   });
    return (managed, () =>
                     {
                       managed.Dispose();
                       return Task.CompletedTask;
                     });
  }

  /// <summary>The value below which <paramref name="fraction" /> of the sorted samples fall.</summary>
  private static double Percentile(double[] sorted,
                                   double   fraction)
    => sorted[Math.Min(sorted.Length - 1,
                       (int)Math.Ceiling(fraction * sorted.Length) - 1)];

  private static string Framework()
#if NETFRAMEWORK
    => "net4.8";
#else
    => "net8.0";
#endif

  /// <summary>What the process holds, after a full collection.</summary>
  private readonly struct Footprint
  {
    private Footprint(long privateBytes,
                      long managedBytes)
    {
      PrivateBytes = privateBytes;
      ManagedBytes = managedBytes;
    }

    internal long PrivateBytes { get; }
    internal long ManagedBytes { get; }

    internal static Footprint Read()
    {
      GC.Collect();
      GC.WaitForPendingFinalizers();
      GC.Collect();
      using var self = Process.GetCurrentProcess();
      return new Footprint(self.PrivateMemorySize64,
                           GC.GetTotalMemory(true));
    }
  }

  /// <summary>The test server, started over TLS for the length of one measurement.</summary>
  private sealed class Server : IDisposable
  {
    // What the test server prints, which this host cannot take from it: the server is a net8.0
    // process, not an assembly a net4.8 host could reference.
    private const string EndpointPrefix  = "ENDPOINT ";
    private const string AuthorityPrefix = "CA ";

    private readonly Process process_;

    private Server(Process process,
                   string  endpoint,
                   string  authority)
    {
      process_  = process;
      Endpoint  = endpoint;
      Authority = authority;
    }

    internal string Endpoint  { get; }
    internal string Authority { get; }

    /// <summary>Closes the server's stdin, which stops it and has it remove what it made.</summary>
    public void Dispose()
    {
      process_.StandardInput.Close();
      if (!process_.WaitForExit(10_000))
      {
        Kill(process_);
      }

      process_.Dispose();
      File.Delete(Authority);
    }

    internal static Server Start()
    {
      var assembly = Path.GetFullPath(typeof(Program).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
                                                     .Single(metadata => metadata.Key == "TestServerAssembly")
                                                     .Value!);
      var process = Process.Start(new ProcessStartInfo("dotnet",
                                                       $"\"{assembly}\" --tls")
                                  {
                                    RedirectStandardInput  = true,
                                    RedirectStandardOutput = true,
                                    UseShellExecute        = false,
                                    CreateNoWindow         = true,
                                  }) ?? throw new InvalidOperationException($"`dotnet {assembly}` did not start");

      // A server that prints neither line in a minute is killed, which ends the read below.
      using var deadline = new Timer(_ => Kill(process),
                                     null,
                                     TimeSpan.FromMinutes(1),
                                     Timeout.InfiniteTimeSpan);
      string? authority = null;
      try
      {
        while (process.StandardOutput.ReadLine() is { } line)
        {
          if (line.StartsWith(AuthorityPrefix,
                              StringComparison.Ordinal))
          {
            authority = line.Substring(AuthorityPrefix.Length);
          }
          else if (line.StartsWith(EndpointPrefix,
                                   StringComparison.Ordinal))
          {
            // Kestrel names the address it bound, which the certificate names too.
            return new Server(process,
                              line.Substring(EndpointPrefix.Length),
                              authority ?? throw new InvalidOperationException("the server named no authority"));
          }
        }

        throw new InvalidOperationException("the server ended, or was killed after a minute, without naming its endpoint");
      }
      catch
      {
        Kill(process);
        process.Dispose();
        // Killed, it did not remove the authority's file itself.
        if (authority is not null)
        {
          File.Delete(authority);
        }

        throw;
      }
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
        // Exited between the check and the kill.
      }
    }
  }
}
