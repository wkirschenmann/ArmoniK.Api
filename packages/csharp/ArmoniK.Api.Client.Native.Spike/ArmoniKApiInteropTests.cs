using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Versions;

using Grpc.Core;
using Grpc.Net.Client;

using NUnit.Framework;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   The existing <c>ArmoniK.Api.Client</c>, on .NET Framework, over the Rust transport.
/// </summary>
/// <remarks>
///   The checklist proves the handler against a service written for it. This proves it against the
///   one that actually matters: the generated ArmoniK clients, unchanged, talking to
///   <c>ArmoniK.Api.Mock</c>. <c>ArmoniK.Api.Client</c> targets netstandard2.0, so net4.8 consumes
///   it as it stands; the only thing that differs from a normal run is which handler is under the
///   channel.
/// </remarks>
[TestFixture]
public class ArmoniKApiInteropTests
{
  private Process?     mock_;
  private GrpcChannel? channel_;

  [OneTimeSetUp]
  public void StartMock()
  {
    var assembly = FindMockAssembly();
    // Both at once: a port is only free until something takes it, and asking twice in a row for
    // "a free port" can hand back the same one twice.
    var (port, httpPort) = TwoFreePorts();

    mock_ = new Process
            {
              StartInfo = new ProcessStartInfo("dotnet",
                                               $"\"{assembly}\"")
                          {
                            UseShellExecute        = false,
                            RedirectStandardOutput = true,
                            RedirectStandardError  = true,
                            CreateNoWindow         = true,
                            WorkingDirectory       = Path.GetDirectoryName(assembly),
                          },
            };
    // Both ports, because the mock only opens a second listener when they differ, and only the
    // gRPC one is plain HTTP/2 without a certificate.
    mock_.StartInfo.EnvironmentVariables["Grpc__Port"] = port.ToString();
    mock_.StartInfo.EnvironmentVariables["Http__Port"] = httpPort.ToString();
    mock_.Start();
    mock_.BeginOutputReadLine();
    mock_.BeginErrorReadLine();
    mock_.OutputDataReceived += (_,
                                 line) => TestContext.Progress.WriteLine($"[mock] {line.Data}");
    mock_.ErrorDataReceived += (_,
                                line) => TestContext.Progress.WriteLine($"[mock] {line.Data}");

    var endpoint = $"http://127.0.0.1:{port}";
    WaitUntilListening(port);

    channel_ = GrpcChannel.ForAddress(endpoint,
                                      new GrpcChannelOptions
                                      {
                                        HttpHandler = new RustHttpHandler($"{{\"Endpoint\": \"{endpoint}\"}}"),
                                      });
  }

  [OneTimeTearDown]
  public void StopMock()
  {
    channel_?.Dispose();
    try
    {
      if (mock_ != null && !mock_.HasExited)
      {
        mock_.Kill();
        mock_.WaitForExit(5000);
      }
    }
    catch (InvalidOperationException)
    {
      // Already gone.
    }

    mock_?.Dispose();
  }

  [Test]
  public void AGeneratedArmoniKClientWorksOverTheRustTransport()
  {
    // A blocking call through a generated client: the shape an existing .NET Framework application
    // uses, with nothing about it changed.
    var versions = new Versions.VersionsClient(channel_);
    var reply = versions.ListVersions(new ListVersionsRequest());

    Assert.That(reply,
                Is.Not.Null);
    TestContext.WriteLine($"mock reports api={reply.Api} core={reply.Core}");
    Assert.That(reply.Api,
                Is.Not.Empty);
  }

  [Test]
  public async Task AGeneratedArmoniKClientWorksAsynchronouslyToo()
  {
    var versions = new Versions.VersionsClient(channel_);
    var reply = await versions.ListVersionsAsync(new ListVersionsRequest());

    Assert.That(reply.Api,
                Is.Not.Empty);
  }

  /// <summary>The mock, built for net8.0, wherever this repository put it.</summary>
  private static string FindMockAssembly()
  {
    var here = new DirectoryInfo(AppDomain.CurrentDomain.BaseDirectory);
    while (here != null && here.Name != "csharp")
    {
      here = here.Parent;
    }

    if (here == null)
    {
      throw new DirectoryNotFoundException("could not find packages/csharp from the test directory");
    }

    var mock = Path.Combine(here.FullName,
                            "ArmoniK.Api.Mock",
                            "bin");
    var candidate = Directory.Exists(mock)
                      ? Directory.GetFiles(mock,
                                           "ArmoniK.Api.Mock.dll",
                                           SearchOption.AllDirectories)
                                 .OrderByDescending(File.GetLastWriteTimeUtc)
                                 .FirstOrDefault()
                      : null;

    if (candidate == null)
    {
      throw new FileNotFoundException("build it first: dotnet build packages/csharp/ArmoniK.Api.Mock",
                                      "ArmoniK.Api.Mock.dll");
    }

    return candidate;
  }

  /// <summary>Two distinct ports nothing is listening on, released together.</summary>
  private static (int Grpc, int Http) TwoFreePorts()
  {
    var first = new TcpListener(IPAddress.Loopback,
                                0);
    var second = new TcpListener(IPAddress.Loopback,
                                 0);
    first.Start();
    second.Start();
    var ports = (((IPEndPoint)first.LocalEndpoint).Port, ((IPEndPoint)second.LocalEndpoint).Port);
    // Held until both are known, so the second cannot be handed the first.
    first.Stop();
    second.Stop();
    return ports;
  }

  private static void WaitUntilListening(int port)
  {
    var deadline = DateTime.UtcNow.AddSeconds(60);
    while (DateTime.UtcNow < deadline)
    {
      try
      {
        using var probe = new TcpClient();
        probe.Connect(IPAddress.Loopback,
                      port);
        return;
      }
      catch (SocketException)
      {
        Thread.Sleep(200);
      }
    }

    throw new TimeoutException($"the mock never listened on {port}");
  }
}
