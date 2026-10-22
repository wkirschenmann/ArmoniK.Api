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
using System.Net.NetworkInformation;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.RustGrpcChannel.Tests;
using ArmoniK.Api.Client.Submitter;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel.Benchmarks;

/// <summary>
///   One transport, measured against the test server, over TLS unless its name ends in -h2c:
///   unary latency, server-streaming throughput, the throughput of sending 150 MiB as one message
///   and as a stream of chunks, and what the process holds once they have run. Prints one line of
///   results.
/// </summary>
/// <remarks>
///   One transport per process, so that what one leaves allocated is not counted against the
///   other. TLS by default for both, because grpc-dotnet on .NET Framework speaks HTTP/2 only over
///   it. The -h2c transports speak HTTP/2 in the clear, which tells TLS's share of a call from the
///   rest; grpc-dotnet speaks it in the clear only on .NET. The server runs with
///   <c>--plain-say</c>: its <c>Say</c> answers with the reply and <c>x-answered</c> alone.
///   The pinned and shared scenarios measure only the unary latency, on a native channel held to
///   one connection, without and with a streamed upload beside it through the whole measurement.
///   The gzip and zstd scenarios send and receive compressible text in that encoding, concurrent
///   runs 16 and then 64 unary calls in flight, and large sends unary messages of 4 and 16 MiB;
///   zstd needs a server built to accept it, which ARMONIK_BENCH_SERVER names. mixed runs large
///   client-streaming uploads beside small unary calls on one channel, over mutual TLS.
/// </remarks>
public static class Program
{
  private const int WarmupCalls   = 1_000;
  private const int MeasuredCalls = 10_000;
  private const int StreamRuns    = 5;
  private const int ChunkCount    = 2_000;
  private const int ChunkSize     = 64 * 1024;
  private const int UploadSize    = 150 * 1024 * 1024;
  private const int BusyWorkers   = 2;

  /// <summary>Far more calls than a pinned or shared run has in flight, so that a call whose
  /// predecessor's claim outlives its await still finds room, and the channel, which places a call
  /// on the fullest connection with room, keeps them all on one.</summary>
  private const int PinnedCallsPerConnection = 16;

  /// <summary>How long the upload of a shared run goes before the measured calls.</summary>
  private static readonly TimeSpan UploadLead = TimeSpan.FromSeconds(1);

  private const int CalibrationTries = 5;
  private const int CalibrationSteps = 4_000_000;

  /// <summary>What the calibration computes, kept so that the compiler keeps the computation.</summary>
  private static ulong calibrated_;

  public static async Task<int> Main(string[] args)
  {
    // mixed takes its parameters after it, as key=value.
    var mixed = args.Length >= 2 && args[1] == "mixed"
                  ? Mixed.Parse(args.Skip(2),
                                args[0].StartsWith("managed",
                                                   StringComparison.Ordinal))
                  : null;
    if (args.Length < 1 || (args.Length > 2 && args[1] != "mixed") ||
        (args[0] != "native" && args[0] != "native-batch" && args[0] != "native-metrics" && args[0] != "native-spin" && args[0] != "native-h2c" &&
         args[0] != "managed" && args[0] != "managed-h2c") ||
        (args.Length >= 2 && args[1] is not ("busy" or "pinned" or "shared" or "gzip" or "zstd" or "concurrent" or "large" or "mixed")) ||
        (args.Length == 2 && (args[1] is "pinned" or "shared") && !args[0].StartsWith("native",
                                                                                       StringComparison.Ordinal)) ||
        // grpc-dotnet's channel has no zstd provider.
        (args.Length == 2 && args[1] == "zstd" && args[0].StartsWith("managed",
                                                                      StringComparison.Ordinal)) ||
        (args.Length >= 2 && args[1] == "mixed" && mixed is null))
    {
      Console.Error.WriteLine("usage: Benchmarks native|native-batch|native-metrics|native-spin|native-h2c|managed|managed-h2c [busy|gzip|concurrent|large]");
      Console.Error.WriteLine("       Benchmarks native|native-batch|native-metrics|native-spin|native-h2c pinned|shared|zstd");
      Console.Error.WriteLine("       Benchmarks <transport> mixed [k=K] [m=M] [size=MiB] [chunk=KiB] [pool=server|N] [fpw=1..256] [comp=gzip|zstd]   (pool, fpw, comp: native only)");
      return 2;
    }

#if NETFRAMEWORK
    if (args[0] == "managed-h2c")
    {
      Console.Error.WriteLine("managed-h2c: WinHttpHandler speaks HTTP/2 only over TLS");
      return 2;
    }

    // What allocation counts on .NET Framework, which has no GC.GetTotalAllocatedBytes.
    AppDomain.MonitoringIsEnabled = true;
#endif
    Placement.ApplyToBenchmark(Process.GetCurrentProcess());
    var transport = args[0];
    var scenario  = args.Length >= 2
                      ? args[1]
                      : "idle";
    var busy = scenario == "busy";
    // Before the server and the load exist, so that nothing competes with it.
    var calibration = Calibration();
    // mixed runs over mutual TLS, the client presenting a certificate the server requires, or in
    // the clear for an -h2c transport, as every scenario does.
    using var server = Server.Start(!transport.EndsWith("-h2c",
                                                        StringComparison.Ordinal),
                                    mixed is not null);

    using var occupied = new CancellationTokenSource();
    var load = Array.Empty<Task>();
    string line;
    try
    {
      if (busy)
      {
        load = Occupy(occupied.Token);
      }

      line = scenario switch
             {
               "pinned" or "shared" => await MeasureShared(transport,
                                                           server,
                                                           scenario == "shared")
                                         .ConfigureAwait(false),
               "gzip" or "zstd" => await MeasureCompressed(transport,
                                                           server,
                                                           scenario)
                                     .ConfigureAwait(false),
               "concurrent" => await MeasureConcurrent(transport,
                                                       server)
                                 .ConfigureAwait(false),
               "large" => await MeasureLarge(transport,
                                             server)
                            .ConfigureAwait(false),
               "mixed" => await MeasureMixed(transport,
                                             server,
                                             mixed!)
                            .ConfigureAwait(false),
               _ => await Measure(transport,
                                  server,
                                  busy)
                      .ConfigureAwait(false),
             };
    }
    finally
    {
      occupied.Cancel();
      await Task.WhenAll(load)
                .ConfigureAwait(false);
    }

    // Once the load has stopped, so that a busy run's calibration times the processor alone.
    Console.WriteLine(line + string.Format(CultureInfo.InvariantCulture,
                                           " calib_us={0:F0} calib_end_us={1:F0}",
                                           calibration,
                                           Calibration()));
    return 0;
  }

  /// <summary>The line of results, without the calibrations.</summary>
  private static async Task<string> Measure(string transport,
                                            Server server,
                                            bool   busy)
  {
    var before = Footprint.Read();
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false);
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
      var allocated = Allocated();
      var poolItems = PoolItems();
      var cpu       = CpuNow();
      for (var call = 0; call < MeasuredCalls; call++)
      {
        clock.Restart();
        await client.SayAsync(new EchoRequest
                              {
                                Text = "x",
                              });
        latencies[call] = clock.Elapsed.TotalMilliseconds * 1000;
      }

      var allocatedPerCall = (Allocated() - allocated) / (double)MeasuredCalls;
      var poolItemsPerCall = (PoolItems() - poolItems) / (double)MeasuredCalls;
      var cpuPerCall       = (CpuNow() - cpu).TotalMilliseconds * 1000 / MeasuredCalls;

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
      var uploads = await Uploads(client)
                      .ConfigureAwait(false);
      var afterUploads = Footprint.Read();

      var line = string.Format(CultureInfo.InvariantCulture,
                               "{0} {1}{2} p50_us={3:F0} p95_us={4:F0} p99_us={5:F0} stream_mib_s={6:F0} upload_stream_mib_s={9:F0} upload_unary_mib_s={10:F0} alloc_b_call={7:F0}{8}",
                               transport,
                               Framework(),
                               busy
                                 ? " busy"
                                 : "",
                               Percentile(latencies,
                                          0.50),
                               Percentile(latencies,
                                          0.95),
                               Percentile(latencies,
                                          0.99),
                               throughputs[throughputs.Count / 2],
                               allocatedPerCall,
                               // .NET Framework counts no work items, which is not zero of them.
                               poolItems < 0
                                 ? ""
                                 : string.Format(CultureInfo.InvariantCulture,
                                                 " pool_items_call={0:F2}",
                                                 poolItemsPerCall),
                               uploads.Streamed,
                               uploads.Whole);
      // The load holds memory of its own, so a busy run's says as much about it as about the
      // transport.
      if (!busy)
      {
        line += string.Format(CultureInfo.InvariantCulture,
                              " cpu_us_call={3:F0} upload_stream_cpu_us_mib={4:F0} upload_unary_cpu_us_mib={5:F0} private_mib={0:F1} managed_mib={1:F1} private_after_uploads_mib={2:F1}",
                              (after.PrivateBytes - before.PrivateBytes) / (1024.0 * 1024),
                              (after.ManagedBytes - before.ManagedBytes) / (1024.0 * 1024),
                              (afterUploads.PrivateBytes - before.PrivateBytes) / (1024.0 * 1024),
                              cpuPerCall,
                              uploads.StreamedCpu,
                              uploads.WholeCpu);
      }

      return line;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  /// <summary>
  ///   The unary latency on a channel held to one connection, alone on it or beside a streamed
  ///   upload of chunks that starts before the measured calls and stops after them, and the MiB
  ///   per second that upload hands to the channel meanwhile.
  /// </summary>
  private static async Task<string> MeasureShared(string transport,
                                                  Server server,
                                                  bool   uploading)
  {
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: true);
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

      using var stop     = new CancellationTokenSource();
      var       uploaded = new StrongBox<long>();
      var upload = uploading
                     ? UploadUntil(client,
                                   uploaded,
                                   stop.Token)
                     : Task.CompletedTask;
      var    latencies = new double[MeasuredCalls];
      double beside;
      var    measured = false;
      try
      {
        if (uploading)
        {
          await Task.Delay(UploadLead)
                    .ConfigureAwait(false);
        }

        var clock  = new Stopwatch();
        var span   = Stopwatch.StartNew();
        var before = Interlocked.Read(ref uploaded.Value);
        for (var call = 0; call < MeasuredCalls; call++)
        {
          clock.Restart();
          await client.SayAsync(new EchoRequest
                                {
                                  Text = "x",
                                });
          latencies[call] = clock.Elapsed.TotalMilliseconds * 1000;
        }

        beside   = (Interlocked.Read(ref uploaded.Value) - before) / span.Elapsed.TotalSeconds / (1024 * 1024);
        measured = true;
      }
      finally
      {
        stop.Cancel();
        // A measurement that failed reports its own exception, not the upload's.
        try
        {
          await upload.ConfigureAwait(false);
        }
        catch (Exception) when (!measured)
        {
        }
      }

      Array.Sort(latencies);
      var line = string.Format(CultureInfo.InvariantCulture,
                               "{0} {1} {2} p50_us={3:F0} p95_us={4:F0} p99_us={5:F0}",
                               transport,
                               Framework(),
                               uploading
                                 ? "shared"
                                 : "pinned",
                               Percentile(latencies,
                                          0.50),
                               Percentile(latencies,
                                          0.95),
                               Percentile(latencies,
                                          0.99));
      if (uploading)
      {
        line += string.Format(CultureInfo.InvariantCulture,
                              " upload_beside_mib_s={0:F0}",
                              beside);
      }

      return line;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  private const int CompressedChunks = 1_024;
  private const int CompressedWhole  = 64 * 1024 * 1024;
  private const int ConcurrentWarmup = 2_000;
  private const int ConcurrentCalls  = 40_000;
  private const int LargeBytes       = 512 * 1024 * 1024;
  private const int LargeWarmupCalls = 2;

  /// <summary>The process's CPU time so far, every thread's.</summary>
  private static TimeSpan CpuNow()
    => Process.GetCurrentProcess()
              .TotalProcessorTime;

  /// <summary>
  ///   Compressible text sent streamed in 64 KiB chunks and as one 64 MiB message, and received in
  ///   64 KiB chunks the server compresses, with <paramref name="encoding" /> both ways: MiB per
  ///   second, the median of five runs of 64 MiB, and the process's CPU and the server's per MiB
  ///   over them, the server warmed first in that encoding.
  /// </summary>
  private static async Task<string> MeasureCompressed(string transport,
                                                      Server server,
                                                      string encoding)
  {
    var managed = transport.StartsWith("managed",
                                       StringComparison.Ordinal);
    var text = new byte[ChunkSize];
    Corpus.Fill(text);
    var chunk = new Chunk
                {
                  Data = Google.Protobuf.ByteString.CopyFrom(text),
                };
    // grpc-dotnet compresses a call's messages in the encoding this entry names.
    var compressed = managed
                       ? new Metadata
                         {
                           {
                             "grpc-internal-encoding-request", encoding
                           },
                         }
                       : null;
    var asked = new Metadata
                {
                  {
                    "x-compress-response", encoding
                  },
                };
    var serverWarm = await WarmServer(transport,
                                      server,
                                      new EchoRequest
                                      {
                                        Text = "x",
                                      },
                                      chunk,
                                      managed
                                        ? null
                                        : encoding,
                                      compressed,
                                      asked)
                       .ConfigureAwait(false);
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false,
                                  compression: managed
                                                 ? null
                                                 : encoding);
    try
    {
      var client = new Echo.EchoClient(channel);
      var wholeText = new byte[CompressedWhole];
      Corpus.Fill(wholeText);
      var whole = new Chunk
                  {
                    Data = Google.Protobuf.ByteString.CopyFrom(wholeText),
                  };

      await StreamUpload(client,
                         chunk,
                         16,
                         compressed)
        .ConfigureAwait(false);
      await client.UploadWholeAsync(chunk,
                                    compressed)
                  .ConfigureAwait(false);
      await Download(client,
                     16,
                     ChunkSize,
                     asked)
        .ConfigureAwait(false);

      // What the server saw of each measured upload and sent each measured download in: a run that
      // did not compress fails rather than report figures for plain text.
      void RequireEncoding(string? seen,
                           string  leg)
      {
        if (seen != encoding)
        {
          var went = string.IsNullOrEmpty(seen)
                       ? "identity"
                       : seen;
          throw new InvalidOperationException($"the {leg} went as {went}, not {encoding}");
        }
      }

      var clock     = new Stopwatch();
      var cpu       = CpuNow();
      var serverCpu = server.Cpu;
      var streamed  = new List<double>();
      for (var run = 0; run < StreamRuns; run++)
      {
        clock.Restart();
        var reply = await StreamUpload(client,
                                       chunk,
                                       CompressedChunks,
                                       compressed)
                      .ConfigureAwait(false);
        streamed.Add(Throughput(reply,
                                clock));
        RequireEncoding(reply.SawMetadata,
                        "streamed upload");
      }

      var streamedCpu = PerMib(ref cpu,
                               StreamRuns * (double)CompressedChunks * ChunkSize);
      var streamedServer = PerMib(ref serverCpu,
                                  server.Cpu,
                                  StreamRuns * (double)CompressedChunks * ChunkSize);
      var unary = new List<double>();
      for (var run = 0; run < StreamRuns; run++)
      {
        clock.Restart();
        var reply = await client.UploadWholeAsync(whole,
                                                  compressed)
                                .ConfigureAwait(false);
        unary.Add(Throughput(reply,
                             clock));
        RequireEncoding(reply.SawMetadata,
                        "unary upload");
      }

      var unaryCpu = PerMib(ref cpu,
                            StreamRuns * (double)CompressedWhole);
      var unaryServer = PerMib(ref serverCpu,
                               server.Cpu,
                               StreamRuns * (double)CompressedWhole);
      var received = new List<double>();
      for (var run = 0; run < StreamRuns; run++)
      {
        clock.Restart();
        var sent = await Download(client,
                                  CompressedChunks,
                                  ChunkSize,
                                  asked)
                     .ConfigureAwait(false);
        received.Add(CompressedChunks * (double)ChunkSize / clock.Elapsed.TotalSeconds / (1024 * 1024));
        RequireEncoding(sent,
                        "download");
      }

      var receivedCpu = PerMib(ref cpu,
                               StreamRuns * (double)CompressedChunks * ChunkSize);
      var receivedServer = PerMib(ref serverCpu,
                                  server.Cpu,
                                  StreamRuns * (double)CompressedChunks * ChunkSize);
      streamed.Sort();
      unary.Sort();
      received.Sort();
      return string.Format(CultureInfo.InvariantCulture,
                           "{0} {1} {2} up_stream_mib_s={3:F0} up_stream_cpu_us_mib={4:F0} up_stream_srv_us_mib={9:F0} up_unary_mib_s={5:F0} up_unary_cpu_us_mib={6:F0} up_unary_srv_us_mib={10:F0} down_mib_s={7:F0} down_cpu_us_mib={8:F0} down_srv_us_mib={11:F0} srv_warm_us_call={12:F1}",
                           transport,
                           Framework(),
                           encoding,
                           streamed[StreamRuns / 2],
                           streamedCpu,
                           unary[StreamRuns / 2],
                           unaryCpu,
                           received[StreamRuns / 2],
                           receivedCpu,
                           streamedServer,
                           unaryServer,
                           receivedServer,
                           serverWarm);
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  /// <summary>Sends <paramref name="chunks" /> chunks as one client stream; the server's reply.</summary>
  private static async Task<EchoReply> StreamUpload(Echo.EchoClient client,
                                                    Chunk           chunk,
                                                    int             chunks,
                                                    Metadata?       headers)
  {
    using var upload = client.Upload(headers);
    for (var sent = 0; sent < chunks; sent++)
    {
      await upload.RequestStream.WriteAsync(chunk)
                  .ConfigureAwait(false);
    }

    await upload.RequestStream.CompleteAsync()
                .ConfigureAwait(false);
    return await upload.ResponseAsync.ConfigureAwait(false);
  }

  /// <summary>Reads <paramref name="chunks" /> chunks of <paramref name="size" /> bytes; the
  /// encoding the server says it sent them in.</summary>
  private static async Task<string?> Download(Echo.EchoClient client,
                                              int             chunks,
                                              int             size,
                                              Metadata?       headers)
  {
    using var stream = client.Stream(new StreamRequest
                                     {
                                       Count = chunks,
                                       Size  = size,
                                     },
                                     headers);
    long received = 0;
    while (await stream.ResponseStream.MoveNext()
                       .ConfigureAwait(false))
    {
      received += stream.ResponseStream.Current.Data.Length;
    }

    if (received != (long)chunks * size)
    {
      throw new InvalidOperationException($"read {received} bytes of {(long)chunks * size}");
    }

    return stream.GetTrailers()
                 .GetValue("x-sent-encoding");
  }

  /// <summary>The CPU microseconds per MiB of <paramref name="bytes" /> since <paramref name="since" />,
  /// which then moves to now.</summary>
  private static double PerMib(ref TimeSpan since,
                               double       bytes)
    => PerMib(ref since,
              CpuNow(),
              bytes);

  /// <summary>The CPU microseconds per MiB of <paramref name="bytes" /> from <paramref name="since" />
  /// to <paramref name="now" />, a reading of any process's CPU time, to which it then moves.</summary>
  private static double PerMib(ref TimeSpan since,
                               TimeSpan     now,
                               double       bytes)
  {
    var used = (now - since).TotalMilliseconds * 1000 / (bytes / (1024 * 1024));
    since = now;
    return used;
  }

  /// <summary>
  ///   Unary calls with 16 and then 64 in flight on one channel, each worker starting its next call
  ///   once its last ends: calls a second, the process's CPU per call, and the latencies.
  /// </summary>
  private static async Task<string> MeasureConcurrent(string transport,
                                                      Server server)
  {
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false);
    try
    {
      var client = new Echo.EchoClient(channel);
      var line = string.Format(CultureInfo.InvariantCulture,
                               "{0} {1} concurrent",
                               transport,
                               Framework());
      foreach (var inFlight in new[]
                               {
                                 16, 64,
                               })
      {
        await Calls(client,
                    inFlight,
                    ConcurrentWarmup,
                    null)
          .ConfigureAwait(false);
        var latencies = new double[ConcurrentCalls];
        var cpu       = CpuNow();
        var clock     = Stopwatch.StartNew();
        await Calls(client,
                    inFlight,
                    ConcurrentCalls,
                    latencies)
          .ConfigureAwait(false);
        var elapsed = clock.Elapsed.TotalSeconds;
        var used    = (CpuNow() - cpu).TotalMilliseconds * 1000;
        Array.Sort(latencies);
        line += string.Format(CultureInfo.InvariantCulture,
                              " c{0}_calls_s={1:F0} c{0}_cpu_us_call={2:F1} c{0}_p50_us={3:F0} c{0}_p99_us={4:F0}",
                              inFlight,
                              ConcurrentCalls / elapsed,
                              used / ConcurrentCalls,
                              Percentile(latencies,
                                         0.50),
                              Percentile(latencies,
                                         0.99));
      }

      return line;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  /// <summary><paramref name="calls" /> unary calls made by <paramref name="workers" /> workers,
  /// each call's latency in <paramref name="latencies" /> when given.</summary>
  private static Task Calls(Echo.EchoClient client,
                            int             workers,
                            int             calls,
                            double[]?       latencies)
  {
    var next = -1;
    return Task.WhenAll(Enumerable.Range(0,
                                         workers)
                                  .Select(_ => Task.Run(async () =>
                                                        {
                                                          var clock = new Stopwatch();
                                                          int call;
                                                          while ((call = Interlocked.Increment(ref next)) < calls)
                                                          {
                                                            clock.Restart();
                                                            await client.SayAsync(new EchoRequest
                                                                                  {
                                                                                    Text = "x",
                                                                                  })
                                                                        .ConfigureAwait(false);
                                                            if (latencies is not null)
                                                            {
                                                              latencies[call] = clock.Elapsed.TotalMilliseconds * 1000;
                                                            }
                                                          }
                                                        })));
  }

  /// <summary>
  ///   512 MiB sent as unary messages of 4 MiB and of 16 MiB, one call at a time, and on a native
  ///   channel received as server-streaming calls of one such message: MiB per second and the
  ///   process's CPU per MiB. grpc-dotnet's channel refuses a message past 4 MiB, so a managed run
  ///   only sends.
  /// </summary>
  private static async Task<string> MeasureLarge(string transport,
                                                 Server server)
  {
    var native = transport.StartsWith("native",
                                      StringComparison.Ordinal);
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false,
                                  large: true);
    try
    {
      var client = new Echo.EchoClient(channel);
      var line = string.Format(CultureInfo.InvariantCulture,
                               "{0} {1} large",
                               transport,
                               Framework());
      foreach (var mib in new[]
                          {
                            4, 16,
                          })
      {
        var size  = mib * 1024 * 1024;
        var count = LargeBytes / size;
        var message = new Chunk
                      {
                        Data = Google.Protobuf.ByteString.CopyFrom(new byte[size]),
                      };
        for (var call = 0; call < LargeWarmupCalls; call++)
        {
          await client.UploadWholeAsync(message)
                      .ConfigureAwait(false);
        }

        var cpu   = CpuNow();
        var clock = Stopwatch.StartNew();
        for (var call = 0; call < count; call++)
        {
          await client.UploadWholeAsync(message)
                      .ConfigureAwait(false);
        }

        var elapsed = clock.Elapsed.TotalSeconds;
        line += string.Format(CultureInfo.InvariantCulture,
                              " l{0}_up_mib_s={1:F0} l{0}_up_cpu_us_mib={2:F0}",
                              mib,
                              LargeBytes / (1024.0 * 1024) / elapsed,
                              PerMib(ref cpu,
                                     LargeBytes));
        if (!native)
        {
          continue;
        }

        for (var call = 0; call < LargeWarmupCalls; call++)
        {
          await Download(client,
                         1,
                         size,
                         null)
            .ConfigureAwait(false);
        }

        cpu = CpuNow();
        clock.Restart();
        for (var call = 0; call < count; call++)
        {
          await Download(client,
                         1,
                         size,
                         null)
            .ConfigureAwait(false);
        }

        elapsed = clock.Elapsed.TotalSeconds;
        line += string.Format(CultureInfo.InvariantCulture,
                              " l{0}_down_mib_s={1:F0} l{0}_down_cpu_us_mib={2:F0}",
                              mib,
                              LargeBytes / (1024.0 * 1024) / elapsed,
                              PerMib(ref cpu,
                                     LargeBytes));
      }

      return line;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  private const int MixedWarmupCalls = 2_000;
  private const int MixedWarmupMiB   = 16;
  private const int MixedTextLength  = 256;

  /// <summary>How much the uploads of a run send in all, at the least: a run of one upload in
  /// flight then lasts several uploads.</summary>
  private const int MixedLeastMiB = 2_048;

  /// <summary>How long the unary calls run alone before the window, so the window opens on calls
  /// already in flight.</summary>
  private static readonly TimeSpan MixedLead = TimeSpan.FromMilliseconds(500);

  /// <summary>The window of a run without uploads.</summary>
  private static readonly TimeSpan MixedUnaryAlone = TimeSpan.FromSeconds(5);

  private static readonly TimeSpan ConnectionSampling = TimeSpan.FromMilliseconds(250);

  /// <summary>
  ///   Large client-streaming uploads and small unary calls on one channel at once, as an
  ///   application's two loops would make them: K upload workers taking uploads from one count, each
  ///   starting the next when its last ends, and M unary workers likewise. The window runs from the
  ///   uploads' start to the last one's end, or five seconds without uploads; what it reports is
  ///   measured over it, the calls and latencies those of the unary calls that end in it.
  /// </summary>
  /// <remarks>
  ///   The CPU figures are the process's over the window, both tasks' and the sampling's together,
  ///   per MiB uploaded, per unary call (the _both figures) and in cores; the server's likewise,
  ///   which tells a run the server held back. The connections established to the server and the
  ///   process's private bytes are sampled every 250 ms, the most of each reported: the process's
  ///   own peak would count the warm-up's.
  /// </remarks>
  private static async Task<string> MeasureMixed(string transport,
                                                 Server server,
                                                 Mixed  mixed)
  {
    var request = new EchoRequest
                  {
                    Text = new string('x',
                                      MixedTextLength),
                  };
    // Text when compressed, which the encodings shrink as they would a payload rather than to
    // almost nothing.
    var data = new byte[mixed.ChunkKiB * 1024];
    if (mixed.Compression is not null)
    {
      Corpus.Fill(data);
    }

    var chunk = new Chunk
                {
                  Data = Google.Protobuf.ByteString.CopyFrom(data),
                };
    var chunks     = (int)((long)mixed.SizeMiB * 1024 / mixed.ChunkKiB);
    var serverWarm = await WarmServer(transport,
                                      server,
                                      request,
                                      chunk,
                                      mixed.Compression)
                       .ConfigureAwait(false);

    var before = Footprint.Read();
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false,
                                  compression: mixed.Compression,
                                  pool: mixed.Pool == 0
                                          ? null
                                          : new CallsPerConnection.Limit(mixed.Pool),
                                  framesPerWrite: mixed.FramesPerWrite == 0
                                                    ? null
                                                    : mixed.FramesPerWrite);
    try
    {
      var client = new Echo.EchoClient(channel);

      // Both kinds of call at the run's own concurrency, so that the connections the channel opens
      // for them are open before the window.
      await Task.WhenAll(new[]
                         {
                           SayMany(client,
                                   request,
                                   mixed.Callers,
                                   MixedWarmupCalls),
                         }.Concat(Enumerable.Range(0,
                                                   mixed.Uploads)
                                            .Select(_ => Task.Run(() => StreamUpload(client,
                                                                                     chunk,
                                                                                     Math.Max(1,
                                                                                              MixedWarmupMiB * 1024 / mixed.ChunkKiB),
                                                                                     null)))))
                .ConfigureAwait(false);

      using var stop = new CancellationTokenSource();
      var callers = Enumerable.Range(0,
                                     mixed.Callers)
                              .Select(_ => Task.Run(() => Caller(client,
                                                                 request,
                                                                 stop.Token)))
                              .ToArray();
      var measured = false;
      var port     = new Uri(server.Endpoint).Port;
      var most     = 0;
      var peak     = 0L;
      var samples  = 0;
      var failures = 0;
      var failed   = (Exception?)null;
      var sampled  = new object();
      using var sampler = new Timer(_ =>
                                    {
                                      // A sample that fails is counted, not thrown: an exception
                                      // here would end the process mid-window.
                                      try
                                      {
                                        var       now  = Connections(port);
                                        using var self = Process.GetCurrentProcess();
                                        lock (sampled)
                                        {
                                          most = Math.Max(most,
                                                          now);
                                          peak = Math.Max(peak,
                                                          self.PrivateMemorySize64);
                                          samples++;
                                        }
                                      }
                                      catch (Exception error)
                                      {
                                        lock (sampled)
                                        {
                                          failures++;
                                          failed = error;
                                        }
                                      }
                                    });
      try
      {
        if (mixed.Callers > 0)
        {
          await Task.Delay(MixedLead)
                    .ConfigureAwait(false);
        }

        var uploads = mixed.Uploads == 0
                        ? 0
                        : Math.Max(mixed.Uploads,
                                   (int)Math.Ceiling(Math.Max(MixedLeastMiB,
                                                              8.0 * mixed.SizeMiB) / mixed.SizeMiB));
        var next      = 0;
        var threads   = ThreadTimes();
        var cpu       = CpuNow();
        var serverCpu = server.Cpu;
        var start     = Stopwatch.GetTimestamp();
        sampler.Change(TimeSpan.Zero,
                       ConnectionSampling);
        var uploaders = Enumerable.Range(0,
                                         mixed.Uploads)
                                  .Select(_ => Task.Run(() => UploadLoop(client,
                                                                         chunk,
                                                                         chunks,
                                                                         () => Interlocked.Increment(ref next) <= uploads)))
                                  .ToArray();
        if (mixed.Uploads == 0)
        {
          await Task.Delay(MixedUnaryAlone)
                    .ConfigureAwait(false);
        }

        var uploaded = await Task.WhenAll(uploaders)
                                 .ConfigureAwait(false);
        var end = Stopwatch.GetTimestamp();
        // Read at the close, so that the thread times end with the window; listing the threads costs
        // CPU the process figure counts, as it counts the sampler's.
        var busiest = Busiest(threads,
                              ThreadTimes());
        var used       = (CpuNow()   - cpu).TotalMilliseconds       * 1000;
        var serverUsed = (server.Cpu - serverCpu).TotalMilliseconds * 1000;
        sampler.Change(Timeout.InfiniteTimeSpan,
                       Timeout.InfiniteTimeSpan);
        stop.Cancel();
        var calls = await Task.WhenAll(callers)
                              .ConfigureAwait(false);
        measured = true;

        var seconds = (end - start) / (double)Stopwatch.Frequency;
        var line = string.Format(CultureInfo.InvariantCulture,
                                 "{0} {1} {2}",
                                 transport,
                                 Framework(),
                                 mixed.Label);
        if (mixed.Uploads > 0)
        {
          var durations = uploaded.SelectMany(worker => worker.Select(upload => upload.Seconds))
                                  .OrderBy(duration => duration)
                                  .ToArray();
          var mib = uploaded.Sum(worker => worker.Sum(upload => upload.Bytes)) / (1024.0 * 1024);
          line += string.Format(CultureInfo.InvariantCulture,
                                " l_mib_s={0:F0} l_up_p50_s={1:F2} l_up_max_s={2:F2} cpu_us_mib_both={3:F0} srv_us_mib_both={4:F0}",
                                mib / seconds,
                                Percentile(durations,
                                           0.50),
                                durations[durations.Length - 1],
                                used / mib,
                                serverUsed / mib);
        }

        if (mixed.Callers > 0)
        {
          // The calls that end in the window, a call the uploads' start held included.
          var latencies = calls.SelectMany(worker => worker.Where(call => call.End >= start && call.End <= end)
                                                           .Select(call => (call.End - call.Start) * 1e6 / Stopwatch.Frequency))
                               .OrderBy(latency => latency)
                               .ToArray();
          line += string.Format(CultureInfo.InvariantCulture,
                                " u_calls_s={0:F0}",
                                latencies.Length / seconds);
          // None ends when the uploads hold every call for the whole window.
          if (latencies.Length > 0)
          {
            line += string.Format(CultureInfo.InvariantCulture,
                                  " u_p50_us={0:F0} u_p99_us={1:F0} u_p999_us={2:F0} u_max_us={3:F0} cpu_us_call_both={4:F1} srv_us_call_both={5:F1}",
                                  Percentile(latencies,
                                             0.50),
                                  Percentile(latencies,
                                             0.99),
                                  Percentile(latencies,
                                             0.999),
                                  latencies[latencies.Length - 1],
                                  used / latencies.Length,
                                  serverUsed / latencies.Length);
          }
        }

        int        connections;
        long       privateBytes;
        int        taken;
        int        missed;
        Exception? lastFailure;
        lock (sampled)
        {
          connections  = most;
          privateBytes = peak;
          taken        = samples;
          missed       = failures;
          lastFailure  = failed;
        }

        // The window opens with a sample, so a run with none has its sampling broken, not empty.
        if (taken == 0)
        {
          throw new InvalidOperationException($"no sample of the connections and private bytes succeeded, {missed} failed",
                                              lastFailure);
        }

        return line + string.Format(CultureInfo.InvariantCulture,
                                    " cpu_cores={0:F2} busiest_thread_cores={1:F2} srv_cpu_cores={2:F2} conns={3} peak_above_mib={4:F0} base_private_mib={5:F0} window_s={6:F1} srv_warm_us_call={7:F1} samples_failed={8}",
                                    used / 1e6 / seconds,
                                    busiest / 1e6 / seconds,
                                    serverUsed / 1e6 / seconds,
                                    connections,
                                    (privateBytes - before.PrivateBytes) / (1024.0 * 1024),
                                    before.PrivateBytes / (1024.0 * 1024),
                                    seconds,
                                    serverWarm,
                                    missed);
      }
      finally
      {
        stop.Cancel();
        // A measurement that failed reports its own exception, not the callers'.
        try
        {
          await Task.WhenAll(callers)
                    .ConfigureAwait(false);
        }
        catch (Exception) when (!measured)
        {
        }
      }
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  private const int ServerWarmupCalls     = 40_000;
  private const int ServerSettledCalls    = 10_000;
  private const int ServerWarmupInFlight  = 64;
  private const int ServerWarmupUploads   = 4;
  private const int ServerWarmupUploadMiB = 64;

  /// <summary>
  ///   Warms the server on a channel of its own, closed before the run's opens, with unary calls
  ///   and then uploads in the run's chunks, compressed as the run's are: by a native channel in
  ///   <paramref name="compression" />, by a managed one per call through
  ///   <paramref name="headers" />; then, when <paramref name="download" /> is given, a download of
  ///   64 KiB chunks with those headers, which ask the server to compress. The server's CPU per call
  ///   over the last calls, which tells a server still compiling from a settled one.
  /// </summary>
  /// <remarks>
  ///   Its just-in-time compiler tiers a method up only after it has run a while: a server warmed
  ///   by a few thousand calls still spends several times its settled CPU per call. The run's own
  ///   channel then warms at the run's concurrency, which opens the connections it uses; this one
  ///   would hold others open beside them.
  /// </remarks>
  private static async Task<double> WarmServer(string      transport,
                                               Server      server,
                                               EchoRequest request,
                                               Chunk       chunk,
                                               string?     compression = null,
                                               Metadata?   headers     = null,
                                               Metadata?   download    = null)
  {
    var (channel, release) = Open(transport,
                                  server,
                                  pinned: false,
                                  compression: compression);
    try
    {
      var client = new Echo.EchoClient(channel);
      await SayMany(client,
                    request,
                    ServerWarmupInFlight,
                    ServerWarmupCalls)
        .ConfigureAwait(false);
      var cpu = server.Cpu;
      await SayMany(client,
                    request,
                    ServerWarmupInFlight,
                    ServerSettledCalls)
        .ConfigureAwait(false);
      var perCall = (server.Cpu - cpu).TotalMilliseconds * 1000 / ServerSettledCalls;
      await Task.WhenAll(Enumerable.Range(0,
                                          ServerWarmupUploads)
                                   .Select(_ => Task.Run(() => StreamUpload(client,
                                                                            chunk,
                                                                            ServerWarmupUploadMiB * 1024 * 1024 / chunk.Data.Length,
                                                                            headers))))
                .ConfigureAwait(false);
      if (download is not null)
      {
        await Download(client,
                       ServerWarmupUploadMiB * 1024 * 1024 / ChunkSize,
                       ChunkSize,
                       download)
          .ConfigureAwait(false);
      }

      return perCall;
    }
    finally
    {
      await release()
        .ConfigureAwait(false);
    }
  }

  /// <summary><paramref name="calls" /> unary calls made by <paramref name="workers" /> workers,
  /// each starting its next once its last ends.</summary>
  private static Task SayMany(Echo.EchoClient client,
                              EchoRequest     request,
                              int             workers,
                              int             calls)
  {
    var next = 0;
    return Task.WhenAll(Enumerable.Range(0,
                                         workers)
                                  .Select(_ => Task.Run(async () =>
                                                        {
                                                          while (Interlocked.Increment(ref next) <= calls)
                                                          {
                                                            await client.SayAsync(request)
                                                                        .ConfigureAwait(false);
                                                          }
                                                        })));
  }

  /// <summary>Unary calls one after the other until <paramref name="stop" />: each one's start and
  /// end, in <see cref="Stopwatch" /> ticks.</summary>
  private static async Task<List<(long Start, long End)>> Caller(Echo.EchoClient   client,
                                                                 EchoRequest       request,
                                                                 CancellationToken stop)
  {
    var calls = new List<(long Start, long End)>();
    while (!stop.IsCancellationRequested)
    {
      var start = Stopwatch.GetTimestamp();
      await client.SayAsync(request)
                  .ConfigureAwait(false);
      calls.Add((start, Stopwatch.GetTimestamp()));
    }

    return calls;
  }

  /// <summary>Client-streaming uploads one after the other while <paramref name="another" /> says
  /// so: each one's seconds, and the bytes the server says it read.</summary>
  private static async Task<List<(double Seconds, long Bytes)>> UploadLoop(Echo.EchoClient client,
                                                                           Chunk           chunk,
                                                                           int             chunks,
                                                                           Func<bool>      another)
  {
    var done  = new List<(double Seconds, long Bytes)>();
    var clock = new Stopwatch();
    while (another())
    {
      clock.Restart();
      var reply = await StreamUpload(client,
                                     chunk,
                                     chunks,
                                     null)
                    .ConfigureAwait(false);
      done.Add((clock.Elapsed.TotalSeconds, long.Parse(reply.Text,
                                                       CultureInfo.InvariantCulture)));
    }

    return done;
  }

  /// <summary>The TCP connections established to <paramref name="port" /> on this machine.</summary>
  /// <remarks>The server's own ends have the port as their local one, so only the client's count.</remarks>
  private static int Connections(int port)
    => IPGlobalProperties.GetIPGlobalProperties()
                         .GetActiveTcpConnections()
                         .Count(connection => connection.State == TcpState.Established && connection.RemoteEndPoint.Port == port);

  /// <summary>Each thread of the process and its CPU time so far, in microseconds.</summary>
  private static Dictionary<int, double> ThreadTimes()
  {
    using var self  = Process.GetCurrentProcess();
    var       times = new Dictionary<int, double>();
    foreach (ProcessThread thread in self.Threads)
    {
      // A thread that ends between the listing and the read has no time to give.
      try
      {
        times[thread.Id] = thread.TotalProcessorTime.TotalMilliseconds * 1000;
      }
      catch (Exception)
      {
      }
    }

    return times;
  }

  /// <summary>The most CPU microseconds one thread used between two readings, a thread new since
  /// the first counting from zero.</summary>
  /// <remarks>
  ///   A native channel's engine work runs on one thread, which lives through the window, so this
  ///   nears 1.0 core when that thread saturates. Threads are told apart by id: one that ends in
  ///   the window is not counted, and one that takes the id of a thread ended since the first
  ///   reading is undercounted, which only pool threads that retire, a managed run's, may do.
  /// </remarks>
  private static double Busiest(Dictionary<int, double> before,
                                Dictionary<int, double> after)
    => after.Select(thread => thread.Value - (before.TryGetValue(thread.Key,
                                                                 out var then)
                                                ? then
                                                : 0))
            .DefaultIfEmpty(0)
            .Max();

  /// <summary>The parameters of a mixed run, from its key=value arguments.</summary>
  private sealed class Mixed
  {
    /// <summary>K, the uploads in flight.</summary>
    internal int Uploads { get; private set; } = 1;

    /// <summary>M, the unary calls in flight.</summary>
    internal int Callers { get; private set; } = 16;

    internal int SizeMiB  { get; private set; } = 128;
    internal int ChunkKiB { get; private set; } = 64;

    /// <summary>The calls a native channel's connection carries at most; 0 for as many as the
    /// server allows, the engine's default.</summary>
    internal int Pool { get; private set; }

    /// <summary>The DATA frames one queued part of a native channel's request may span; 0 for the
    /// build's default.</summary>
    internal int FramesPerWrite { get; private set; }

    /// <summary>The encoding a native channel compresses every message in, both ways; null for
    /// none.</summary>
    internal string? Compression { get; private set; }

    /// <summary>The parameters, with the -fN suffix only when the frames per write are stated, and
    /// the encoding last when there is one.</summary>
    internal string Label
      => string.Format(CultureInfo.InvariantCulture,
                       "mixed-k{0}-m{1}-s{2}-c{3}-p{4}{5}{6}",
                       Uploads,
                       Callers,
                       SizeMiB,
                       ChunkKiB,
                       Pool == 0
                         ? "S"
                         : Pool.ToString(CultureInfo.InvariantCulture),
                       FramesPerWrite == 0
                         ? ""
                         : string.Format(CultureInfo.InvariantCulture,
                                         "-f{0}",
                                         FramesPerWrite),
                       Compression is null
                         ? ""
                         : "-" + Compression);

    /// <summary>Null for an argument it does not know, a value out of range, or a pool, frames per
    /// write or compression on a managed channel, which sets none of them.</summary>
    internal static Mixed? Parse(IEnumerable<string> args,
                                 bool                managed)
    {
      var mixed = new Mixed();
      foreach (var arg in args)
      {
        var parts = arg.Split('=');
        if (parts.Length != 2)
        {
          return null;
        }

        if (parts[0] == "pool" && parts[1] == "server")
        {
          mixed.Pool = 0;
          continue;
        }

        if (parts[0] == "comp" && parts[1] is "gzip" or "zstd")
        {
          mixed.Compression = parts[1];
          continue;
        }

        if (!int.TryParse(parts[1],
                          NumberStyles.None,
                          CultureInfo.InvariantCulture,
                          out var value))
        {
          return null;
        }

        switch (parts[0])
        {
          case "k":
            mixed.Uploads = value;
            break;
          case "m":
            mixed.Callers = value;
            break;
          case "size":
            mixed.SizeMiB = value;
            break;
          case "chunk":
            mixed.ChunkKiB = value;
            break;
          case "pool" when value > 0:
            mixed.Pool = value;
            break;
          case "fpw" when value is >= 1 and <= 256:
            mixed.FramesPerWrite = value;
            break;
          default:
            return null;
        }
      }

      // A chunk divides the upload. At most 1 GiB: its byte count has to fit an int and a protobuf
      // message (2 GiB).
      var valid = mixed.Uploads + mixed.Callers > 0        && mixed.Uploads <= 64 && mixed.Callers <= 1024 && mixed.SizeMiB is > 0 and <= 4096 &&
                  mixed.ChunkKiB is > 0 and <= 1024 * 1024 && (long)mixed.SizeMiB * 1024 % mixed.ChunkKiB == 0 &&
                  !(managed && (mixed.Pool != 0 || mixed.FramesPerWrite != 0 || mixed.Compression is not null));
      return valid
               ? mixed
               : null;
    }
  }

  /// <summary>Streams chunks to the server until <paramref name="stop" />, adding each one's bytes
  /// to <paramref name="sent" /> once the channel has taken it.</summary>
  /// <remarks>
  ///   <paramref name="stop" /> cancels the call, so that a write that waits for the channel to
  ///   take it ends too: .NET Framework's stream writer takes no token. A write that fails once
  ///   <paramref name="stop" /> is set ends the upload, whatever status the transport gives it.
  /// </remarks>
  private static async Task UploadUntil(Echo.EchoClient   client,
                                        StrongBox<long>   sent,
                                        CancellationToken stop)
  {
    var chunk = new Chunk
                {
                  Data = Google.Protobuf.ByteString.CopyFrom(new byte[ChunkSize]),
                };
    using var upload = client.Upload(cancellationToken: stop);
    try
    {
      while (!stop.IsCancellationRequested)
      {
        await upload.RequestStream.WriteAsync(chunk)
                    .ConfigureAwait(false);
        Interlocked.Add(ref sent.Value,
                        ChunkSize);
      }
    }
    catch (Exception) when (stop.IsCancellationRequested)
    {
    }
  }

  /// <summary>Thread-pool workers kept running, as in an application whose pool is never idle.</summary>
  /// <remarks>Each computes for 50 us and yields, so a completion may find a worker awake.</remarks>
  private static Task[] Occupy(CancellationToken stop)
    => Enumerable.Range(0,
                        BusyWorkers)
                 .Select(_ => Task.Run(async () =>
                                       {
                                         while (!stop.IsCancellationRequested)
                                         {
                                           var until = Stopwatch.GetTimestamp() + Stopwatch.Frequency / 20_000;
                                           while (Stopwatch.GetTimestamp() < until)
                                           {
                                           }

                                           await Task.Yield();
                                         }
                                       }))
                 .ToArray();

  /// <summary>The transport's channel to the server; a native one held to one connection when
  /// <paramref name="pinned" />, compressing both ways in <paramref name="compression" />, and
  /// accepting messages past 4 MiB when <paramref name="large" />, its connections each carrying
  /// at most <paramref name="pool" /> calls and a queued part spanning at most
  /// <paramref name="framesPerWrite" /> DATA frames when given. Either presents the server's client
  /// certificate when the server has one.</summary>
  /// <remarks>A managed channel takes none of the native options: grpc-dotnet compresses per call,
  /// its channel's receive limit is the factory's, and GrpcClient has no calls per connection.</remarks>
  private static (ChannelBase Channel, Func<Task> Release) Open(string              transport,
                                                               Server              server,
                                                               bool                pinned,
                                                               string?             compression    = null,
                                                               bool                large          = false,
                                                               CallsPerConnection? pool           = null,
                                                               int?                framesPerWrite = null)
  {
    if (transport is "native" or "native-batch" or "native-metrics" or "native-spin" or "native-h2c")
    {
      // native takes the build's frames per write, 16 against h2-batch's patch, the binding's
      // default, and 1 with -p:NativeEngineH2Batch=false; native-batch states 16, which a stock
      // build refuses. native-metrics loads the build with the engine's counters, which is
      // selected before the first runtime exists. native-spin has the channel's thread poll for
      // work for the spin's default window after each callback into the host before it sleeps.
      // native-h2c reaches a server with no authority, so its channel states no Tls.
      if (transport == "native-metrics")
      {
        NativeLibrarySelection.Select(NativeEngineBuild.Metrics);
      }

      var runtime = transport == "native-spin"
                      ? NativeRuntime.Create(new RuntimeOptions
                                             {
                                               BeforeSleep = new BeforeSleep.Spin(),
                                             })
                      : NativeRuntime.Create();
      var http2 = transport == "native-batch"
                    ? new Http2Options
                      {
                        Send = new Http2SendOptions
                               {
                                 FramesPerWrite = 16,
                               },
                      }
                    : null;
      if (pinned)
      {
        http2                                ??= new Http2Options();
        http2.SimultaneousCallsPerConnection =   new CallsPerConnection.Limit(PinnedCallsPerConnection);
      }

      if (pool is not null)
      {
        http2                                ??= new Http2Options();
        http2.SimultaneousCallsPerConnection =   pool;
      }

      if (framesPerWrite is not null)
      {
        http2                     ??= new Http2Options();
        http2.Send                ??= new Http2SendOptions();
        http2.Send.FramesPerWrite =   framesPerWrite;
      }

      var channel = runtime.Channel(server.Endpoint,
                                    new ChannelOptions
                                    {
                                      Transport = server.Authority is null
                                                    ? null
                                                    : new TransportOptions
                                                      {
                                                        Tls = new TlsOptions
                                                              {
                                                                ServerCertificates = new ServerCertificates.CaPem(server.Authority),
                                                                ClientCertificate = server.Client is null
                                                                                      ? null
                                                                                      : new ClientCertificate.Pem(server.Client.Value.Certificate,
                                                                                                                  server.Client.Value.Key),
                                                              },
                                                      },
                                      Http2 = http2,
                                      // A compressed run sends and accepts its encoding; a large
                                      // one accepts a message past the 4 MiB default.
                                      Grpc = compression is null && !large
                                               ? null
                                               : new GrpcOptions
                                                 {
                                                   Send = compression is null
                                                            ? null
                                                            : new GrpcSendOptions
                                                              {
                                                                Compression = compression == "gzip"
                                                                                ? SendCompression.Gzip
                                                                                : SendCompression.Zstd,
                                                              },
                                                   Receive = new GrpcReceiveOptions
                                                             {
                                                               Compression = compression is null
                                                                               ? null
                                                                               : new List<MessageEncoding>
                                                                                 {
                                                                                   compression == "gzip"
                                                                                     ? MessageEncoding.Gzip
                                                                                     : MessageEncoding.Zstd,
                                                                                 },
                                                               MessageSizeKiB = large
                                                                                  ? new ReceiveMessageSizeKiB.Max(32768)
                                                                                  : null,
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
                                                     CaCert   = server.Authority ?? "",
                                                     CertPem  = server.Client?.Certificate ?? "",
                                                     KeyPem   = server.Client?.Key         ?? "",
                                                   });
    return (managed, () =>
                     {
                       managed.Dispose();
                       return Task.CompletedTask;
                     });
  }

  /// <summary>The median MiB per second of sending 150 MiB as a stream of chunks, and as one
  /// message, and the process's CPU microseconds per MiB over each one's runs.</summary>
  private static async Task<(double Streamed, double Whole, double StreamedCpu, double WholeCpu)> Uploads(Echo.EchoClient client)
  {
    var clock = new Stopwatch();
    var chunk = new Chunk
                {
                  Data = Google.Protobuf.ByteString.CopyFrom(new byte[ChunkSize]),
                };
    var whole = new Chunk
                {
                  Data = Google.Protobuf.ByteString.CopyFrom(new byte[UploadSize]),
                };
    var cpu = CpuNow();
    var streamedUploads = new List<double>();
    for (var run = 0; run < StreamRuns; run++)
    {
      clock.Restart();
      using var upload = client.Upload();
      for (var sent = 0; sent < UploadSize / ChunkSize; sent++)
      {
        await upload.RequestStream.WriteAsync(chunk)
                    .ConfigureAwait(false);
      }

      await upload.RequestStream.CompleteAsync()
                  .ConfigureAwait(false);
      var reply = await upload.ResponseAsync.ConfigureAwait(false);
      streamedUploads.Add(Throughput(reply,
                                     clock));
    }

    streamedUploads.Sort();
    var streamedCpu = CpuPerMib(ref cpu);

    var wholeUploads = new List<double>();
    for (var run = 0; run < StreamRuns; run++)
    {
      clock.Restart();
      var reply = await client.UploadWholeAsync(whole)
                              .ConfigureAwait(false);
      wholeUploads.Add(Throughput(reply,
                                  clock));
    }

    wholeUploads.Sort();
    var wholeCpu = CpuPerMib(ref cpu);

    return (streamedUploads[streamedUploads.Count / 2], wholeUploads[wholeUploads.Count / 2], streamedCpu, wholeCpu);
  }

  /// <summary>The process's CPU microseconds per MiB of the uploads since <paramref name="since" />,
  /// which then moves to now.</summary>
  private static double CpuPerMib(ref TimeSpan since)
    => PerMib(ref since,
              StreamRuns * (double)UploadSize);

  /// <summary>The fewest microseconds a fixed computation takes, out of a few tries: the speed the
  /// processor runs at, which follows a power and thermal state no setting here holds.</summary>
  private static double Calibration()
  {
    var clock = new Stopwatch();
    var best  = double.MaxValue;
    for (var tried = 0; tried < CalibrationTries; tried++)
    {
      clock.Restart();
      var state = calibrated_ | 1;
      for (var step = 0; step < CalibrationSteps; step++)
      {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
      }

      best        = Math.Min(best,
                             clock.Elapsed.TotalMilliseconds * 1000);
      calibrated_ = state;
    }

    return best;
  }

  /// <summary>The MiB per second of an upload, from the bytes the server says it read.</summary>
  private static double Throughput(EchoReply reply,
                                   Stopwatch clock)
    => long.Parse(reply.Text,
                  CultureInfo.InvariantCulture) / clock.Elapsed.TotalSeconds / (1024 * 1024);

  /// <summary>The work items the thread pool has run so far; -1 on .NET Framework, which does
  /// not count them.</summary>
  private static long PoolItems()
#if NETFRAMEWORK
    => -1;
#else
    => ThreadPool.CompletedWorkItemCount;
#endif

  /// <summary>The managed bytes the process has allocated so far, every thread's.</summary>
  private static long Allocated()
#if NETFRAMEWORK
    => AppDomain.CurrentDomain.MonitoringTotalAllocatedMemorySize;
#else
    => GC.GetTotalAllocatedBytes(true);
#endif

  /// <summary>The value below which <paramref name="fraction" /> of the sorted samples fall.</summary>
  private static double Percentile(double[] sorted,
                                   double   fraction)
    => sorted[Math.Min(sorted.Length - 1,
                       (int)Math.Ceiling(fraction * sorted.Length) - 1)];

  private static string Framework()
#if NETFRAMEWORK
    => "net4.8";
#else
    => Version() + (RuntimeFeature.IsDynamicCodeCompiled
                      ? ""
                      : "-aot");

  private static string Version()
#if NET11_0_OR_GREATER
    => "net11.0";
#elif NET10_0_OR_GREATER
    => "net10.0";
#else
    => "net8.0";
#endif
#endif

  /// <summary>Where the benchmark and its server run, from the environment.</summary>
  /// <remarks>
  ///   A hybrid processor's two kinds of core run a call at very different speeds, and a run the
  ///   scheduler spreads over both measures where its threads landed. ARMONIK_BENCH_AFFINITY, a
  ///   mask of logical processors in hexadecimal, holds the benchmark to them, and its server too
  ///   unless ARMONIK_BENCH_SERVER_AFFINITY gives the server a mask of its own. That mask must share
  ///   no processor with the benchmark's, so that the two never compete for one. It is refused
  ///   without ARMONIK_BENCH_AFFINITY, which would leave the benchmark unpinned.
  ///   ARMONIK_BENCH_PRIORITY, a <see cref="ProcessPriorityClass" /> name, sets both priorities,
  ///   which a child does not inherit.
  /// </remarks>
  private static class Placement
  {
    private const string AffinityVariable       = "ARMONIK_BENCH_AFFINITY";
    private const string ServerAffinityVariable = "ARMONIK_BENCH_SERVER_AFFINITY";

    /// <summary>Pins the benchmark, once both masks are known to be usable together.</summary>
    internal static void ApplyToBenchmark(Process process)
    {
      var mask       = Mask(AffinityVariable);
      var serverMask = Mask(ServerAffinityVariable);
      if (serverMask is not null && mask is null)
      {
        throw new ArgumentException($"{ServerAffinityVariable} is set without {AffinityVariable}, which would leave the benchmark unpinned");
      }

      if ((mask & serverMask) is { } shared && shared != 0)
      {
        throw new ArgumentException($"{AffinityVariable} and {ServerAffinityVariable} share the logical processors 0x{shared:X}");
      }

      Apply(process,
            mask);
    }

    internal static void ApplyToServer(Process process)
      => Apply(process,
               Mask(ServerAffinityVariable) ?? Mask(AffinityVariable));

    private static long? Mask(string variable)
    {
      if (Environment.GetEnvironmentVariable(variable) is not { Length: > 0 } mask)
      {
        return null;
      }

      var digits = mask.StartsWith("0x",
                                   StringComparison.OrdinalIgnoreCase)
                     ? mask.Substring(2)
                     : mask;
      if (!long.TryParse(digits,
                         NumberStyles.HexNumber,
                         CultureInfo.InvariantCulture,
                         out var processors))
      {
        throw new ArgumentException($"{variable} is {mask}, not a mask of logical processors in hexadecimal");
      }

      if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows) && !RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
      {
        throw new PlatformNotSupportedException($"{variable} is set, and only Windows and Linux pin a process");
      }

      return processors;
    }

    private static void Apply(Process process,
                              long?   mask)
    {
      if (mask is { } processors)
      {
        process.ProcessorAffinity = new IntPtr(processors);
      }

      if (Environment.GetEnvironmentVariable("ARMONIK_BENCH_PRIORITY") is { Length: > 0 } priority)
      {
        if (!Enum.TryParse(priority,
                           true,
                           out ProcessPriorityClass priorityClass))
        {
          throw new ArgumentException($"ARMONIK_BENCH_PRIORITY is {priority}, not a ProcessPriorityClass name");
        }

        process.PriorityClass = priorityClass;
      }
    }
  }

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

  /// <summary>The test server, started over TLS, or in the clear, for the length of one measurement.</summary>
  private sealed class Server : IDisposable
  {
    // What the test server prints, which this host cannot take from it: the server is a net8.0
    // process, not an assembly a net4.8 host could reference.
    private const string EndpointPrefix          = "ENDPOINT ";
    private const string AuthorityPrefix         = "CA ";
    private const string ClientCertificatePrefix = "CLIENT_CERT ";
    private const string ClientKeyPrefix         = "CLIENT_KEY ";

    private readonly Process process_;

    private Server(Process                           process,
                   string                            endpoint,
                   string?                           authority,
                   (string Certificate, string Key)? client)
    {
      process_  = process;
      Endpoint  = endpoint;
      Authority = authority;
      Client    = client;
    }

    internal string Endpoint { get; }

    /// <summary>The authority's file, or none for a server in the clear.</summary>
    internal string? Authority { get; }

    /// <summary>The files of the certificate and key a client presents, for a server that
    /// requires one.</summary>
    internal (string Certificate, string Key)? Client { get; }

    /// <summary>The server process's CPU time so far, every thread's.</summary>
    internal TimeSpan Cpu
      => process_.TotalProcessorTime;

    /// <summary>Closes the server's stdin, which stops a TLS server and has it remove what it made;
    /// one in the clear made nothing, and is killed.</summary>
    public void Dispose()
    {
      if (Authority is null)
      {
        Kill(process_);
        process_.WaitForExit(10_000);
        process_.Dispose();
        return;
      }

      process_.StandardInput.Close();
      if (!process_.WaitForExit(10_000))
      {
        Kill(process_);
      }

      process_.Dispose();
      Delete(Authority,
             Client?.Certificate,
             Client?.Key);
    }

    /// <param name="mtls">Over TLS, the server also requires a client certificate, which it issues.</param>
    internal static Server Start(bool tls,
                                 bool mtls = false)
    {
      // ARMONIK_BENCH_SERVER names another build of the server, and ARMONIK_BENCH_SERVER_HOST the
      // dotnet that runs it.
      var assembly = Environment.GetEnvironmentVariable("ARMONIK_BENCH_SERVER") is { Length: > 0 } other
                       ? other
                       : Path.GetFullPath(typeof(Program).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
                                                         .Single(metadata => metadata.Key == "TestServerAssembly")
                                                         .Value!);
      var host = Environment.GetEnvironmentVariable("ARMONIK_BENCH_SERVER_HOST") is { Length: > 0 } dotnet
                   ? dotnet
                   : "dotnet";
      var process = Process.Start(new ProcessStartInfo(host,
                                                       !tls
                                                         ? $"\"{assembly}\" --plain-say"
                                                         : mtls
                                                           ? $"\"{assembly}\" --plain-say --mtls"
                                                           : $"\"{assembly}\" --plain-say --tls")
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
      string? authority   = null;
      string? certificate = null;
      string? key         = null;
      try
      {
        Placement.ApplyToServer(process);
        while (process.StandardOutput.ReadLine() is { } line)
        {
          if (line.StartsWith(AuthorityPrefix,
                              StringComparison.Ordinal))
          {
            authority = line.Substring(AuthorityPrefix.Length);
          }
          else if (line.StartsWith(ClientCertificatePrefix,
                                   StringComparison.Ordinal))
          {
            certificate = line.Substring(ClientCertificatePrefix.Length);
          }
          else if (line.StartsWith(ClientKeyPrefix,
                                   StringComparison.Ordinal))
          {
            key = line.Substring(ClientKeyPrefix.Length);
          }
          else if (line.StartsWith(EndpointPrefix,
                                   StringComparison.Ordinal))
          {
            // Kestrel names the address it bound, which the certificate names too.
            return new Server(process,
                              line.Substring(EndpointPrefix.Length),
                              tls
                                ? authority ?? throw new InvalidOperationException("the server named no authority")
                                : null,
                              tls && mtls
                                ? (certificate ?? throw new InvalidOperationException("the server named no client certificate"),
                                   key ?? throw new InvalidOperationException("the server named no client key"))
                                : null);
          }
        }

        throw new InvalidOperationException("the server ended, or was killed after a minute, without naming its endpoint");
      }
      catch
      {
        Kill(process);
        process.Dispose();
        // Killed, it did not remove its files itself.
        Delete(authority,
               certificate,
               key);
        throw;
      }
    }

    private static void Delete(params string?[] files)
    {
      foreach (var file in files)
      {
        if (file is not null)
        {
          File.Delete(file);
        }
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
