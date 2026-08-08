using System;
using System.Diagnostics;
using System.IO;
using System.Threading;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   The `spike_server` example, started once for the whole test run.
/// </summary>
/// <remarks>
///   The same raw gRPC service the Rust integration tests use, so the two halves of the checklist
///   cannot drift apart. It binds an ephemeral port and prints it; this reads that line before
///   letting any test run.
/// </remarks>
internal sealed class SpikeServer : IDisposable
{
  private readonly Process process_;

  internal SpikeServer()
  {
    var executable = Environment.GetEnvironmentVariable("ARMONIK_SPIKE_SERVER") ??
                     Path.Combine(AppDomain.CurrentDomain.BaseDirectory,
                                  "spike_server.exe");
    if (!File.Exists(executable))
    {
      throw new FileNotFoundException($"the spike server is missing; build it with `cargo build -p armonik-transport-ffi --example spike_server`",
                                      executable);
    }

    process_ = new Process
               {
                 StartInfo = new ProcessStartInfo(executable)
                             {
                               UseShellExecute        = false,
                               RedirectStandardOutput = true,
                               RedirectStandardError  = true,
                               CreateNoWindow         = true,
                             },
               };
    process_.Start();

    // Drained on a thread of its own, and never blocking the test run. A redirected pipe nobody
    // reads fills at about 4 KiB and then blocks the writer forever, which for a server that logs
    // means it stops answering for reasons that look nothing like the cause.
    process_.ErrorDataReceived += (_,
                                   line) =>
                                  {
                                    if (line.Data != null)
                                    {
                                      Console.Error.WriteLine($"[spike_server] {line.Data}");
                                    }
                                  };
    process_.BeginErrorReadLine();

    var announcement = process_.StandardOutput.ReadLine();
    if (announcement == null || !announcement.StartsWith("listening ",
                                                         StringComparison.Ordinal))
    {
      throw new InvalidOperationException($"the spike server did not announce an address: {announcement ?? "<nothing>"}");
    }

    Address  = announcement.Substring("listening ".Length)
                           .Trim();
    Endpoint = $"http://{Address}";

    // Same reason as stderr, but drained synchronously on a thread of its own: the address above was
    // read with `ReadLine`, and mixing that with `BeginOutputReadLine` on one stream is refused.
    Drain(process_.StandardOutput);
  }

  /// <summary>Read a stream to its end on a background thread, echoing what comes out.</summary>
  private static void Drain(System.IO.StreamReader stream)
    => new Thread(() =>
                  {
                    try
                    {
                      string? line;
                      while ((line = stream.ReadLine()) != null)
                      {
                        Console.Error.WriteLine($"[spike_server] {line}");
                      }
                    }
                    catch (System.IO.IOException)
                    {
                      // The server was killed; the pipe going with it is the expected end.
                    }
                  })
       {
         IsBackground = true,
         Name         = "spike-server-output",
       }.Start();

  /// <summary>The `host:port` the server bound.</summary>
  internal string Address { get; }

  /// <summary>The endpoint, as a URL.</summary>
  internal string Endpoint { get; }

  public void Dispose()
  {
    try
    {
      if (!process_.HasExited)
      {
        process_.Kill();
        process_.WaitForExit(5000);
      }
    }
    catch (InvalidOperationException)
    {
      // Already gone.
    }

    process_.Dispose();
  }
}
