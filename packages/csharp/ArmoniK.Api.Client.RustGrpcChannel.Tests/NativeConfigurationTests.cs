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
using System.Threading.Tasks;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>A runtime started from the sources the engine reads, and a channel that names no endpoint of its own.</summary>
[TestFixture]
public class NativeConfigurationTests : EchoServerFixture
{
  private string directory_ = string.Empty;

  [SetUp]
  public void MakeADirectory()
  {
    directory_ = Path.Combine(Path.GetTempPath(),
                              "armonik-native-configuration-" + Guid.NewGuid()
                                                                    .ToString("N"));
    Directory.CreateDirectory(directory_);
  }

  [TearDown]
  public void RemoveTheDirectory()
    => Directory.Delete(directory_,
                        true);

  private string File(string name,
                      string content)
  {
    var path = Path.Combine(directory_,
                            name);
    System.IO.File.WriteAllText(path,
                                content);
    return path;
  }

  /// <summary>A call over a channel opened on the runtime's Endpoint, which only the configuration names.</summary>
  private async Task CallsThroughTheRuntimesEndpoint(NativeConfiguration configuration)
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(configuration))
                    .ConfigureAwait(false);
    await using var channel = runtime.Channel(string.Empty);

    var reply = await Client(channel)
                      .SayAsync(new EchoRequest
                                {
                                  Text = "configured",
                                })
                      .ResponseAsync.ConfigureAwait(false);

    Assert.That(reply.Text,
                Is.EqualTo("configured"));
  }

  /// <summary>A file's section under the prefix is the runtime's options, its other sections the host's.</summary>
  [Test]
  public Task AFileStatesTheRuntimesOptionsUnderThePrefix()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration().LoadConfigFromFiles(File("appsettings.json",
                                                                                           "{ \"Logging\": { \"LogLevel\": { \"Default\": \"Debug\" } }, " +
                                                                                           $"\"ArmoniK\": {{ \"Client\": {{ \"Grpc\": {{ \"Endpoint\": \"{Endpoint}\", " +
                                                                                           "\"ChannelDefaults\": { \"Grpc\": { \"UserAgent\": \"from-a-file\" } } } } } }")));

  /// <summary>YAML is read as JSON is, under a prefix the constructor names.</summary>
  [Test]
  public Task AYamlFileIsReadUnderTheNamedPrefix()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration("Engine").LoadConfigFromFiles(File("engine.yaml",
                                                                                                $"Engine:\n  Endpoint: {Endpoint}\n  MemoryCeiling: 1048576\n")));

  /// <summary>The command line reaches the engine as text, read by each key's type.</summary>
  [Test]
  public Task ACommandLineIsReadUnderThePrefix()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                           {
                                                                                             $"--ArmoniK:Client:Grpc:Endpoint={Endpoint}",
                                                                                             "--ArmoniK:Client:Grpc:ChannelDefaults:Transport:ConnectTimeoutSeconds=2.5",
                                                                                             "--ArmoniK:Client:Grpc:ChannelDefaults:Transport:Proxy=None",
                                                                                           }));

  /// <summary>With no prefix, a file's options are the whole file, and a command line's its whole tree.</summary>
  [Test]
  public Task WithNoPrefixAFileIsTheWholeDocument()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration(string.Empty).LoadConfigFromFiles(File("whole.json",
                                                                                                     "{ \"MemoryCeiling\": 1048576 }"))
                                                                            .LoadConfigFromCommandLine(new[]
                                                                                                       {
                                                                                                         $"--Endpoint={Endpoint}",
                                                                                                       }));

  /// <summary>An optional file that exists is read as any other.</summary>
  [Test]
  public Task AnOptionalFileThatExistsIsRead()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration().LoadConfigFromOptionalFiles(File("present.json",
                                                                                                  $"{{ \"ArmoniK\": {{ \"Client\": {{ \"Grpc\": {{ \"Endpoint\": \"{Endpoint}\" }} }} }} }}")));

  /// <summary>Nothing null is taken as a source.</summary>
  [Test]
  public void ANullSourceIsRefused()
    => Assert.Multiple(() =>
                       {
                         Assert.That(() => new NativeConfiguration(null!),
                                     Throws.ArgumentNullException);
                         Assert.That(() => new NativeConfiguration().LoadConfigFromFiles(null!),
                                     Throws.ArgumentNullException);
                         Assert.That(() => new NativeConfiguration().LoadConfigFromFiles("a.json",
                                                                                         null!),
                                     Throws.ArgumentNullException);
                         Assert.That(() => new NativeConfiguration().LoadConfigFromCommandLine(null!),
                                     Throws.ArgumentNullException);
                         Assert.That(() => new NativeConfiguration().LoadConfigFromObject(null!),
                                     Throws.ArgumentNullException);
                         Assert.That(() => NativeRuntime.Create((NativeConfiguration)null!),
                                     Throws.ArgumentNullException);
                       });

  /// <summary>A later source wins: an object set in code over a file that names another endpoint, a missing optional file contributing nothing.</summary>
  [Test]
  public Task ALaterSourceWinsOverAnEarlierOne()
    => CallsThroughTheRuntimesEndpoint(new NativeConfiguration().LoadConfigFromFiles(File("stale.json",
                                                                                          "{ \"ArmoniK\": { \"Client\": { \"Grpc\": { \"Endpoint\": \"http://127.0.0.1:1\" } } } }"))
                                                                .LoadConfigFromOptionalFiles(Path.Combine(directory_,
                                                                                                          "absent.json"))
                                                                .LoadConfigFromObject(new RuntimeOptions
                                                                                      {
                                                                                        Endpoint = Endpoint,
                                                                                      }));

  /// <summary>The environment, under the prefix the constructor names.</summary>
  [Test]
  public async Task TheEnvironmentIsReadUnderThePrefix()
  {
    const string name = "AKNATIVECONFIGURATION__Endpoint";
    Environment.SetEnvironmentVariable(name,
                                       Endpoint);
    try
    {
      await CallsThroughTheRuntimesEndpoint(new NativeConfiguration("AKNATIVECONFIGURATION").LoadConfigFromEnvironment())
        .ConfigureAwait(false);
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
  }

  /// <summary>The default prefix is the client's, <c>ArmoniK__Client__Grpc</c>, in the environment.</summary>
  [Test]
  public async Task TheEnvironmentIsReadUnderTheDefaultPrefix()
  {
    const string name = "ArmoniK__Client__Grpc__Endpoint";
    Environment.SetEnvironmentVariable(name,
                                       Endpoint);
    try
    {
      await CallsThroughTheRuntimesEndpoint(new NativeConfiguration().LoadConfigFromEnvironment())
        .ConfigureAwait(false);
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
  }

  private static RuntimeOptions WithWindow(int window)
    => new()
       {
         ChannelDefaults = new ChannelOptions
                           {
                             Grpc = new GrpcOptions
                                    {
                                      Host = new HostOptions
                                             {
                                               Receive = new HostReceiveOptions
                                                         {
                                                           Window = window,
                                                         },
                                             },
                                    },
                           },
       };

  /// <summary>A channel sizes its rings from the window the engine settles, whichever source stated the default.</summary>
  [Test]
  public async Task AChannelReadsItsWindowFromTheEngineWhateverSourceStatedTheDefault()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromObject(WithWindow(7))))
                    .ConfigureAwait(false);
    await using (var channel = runtime.Channel(Endpoint))
    {
      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(7),
                  "an object");
    }

    runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                 {
                                                                                                                   "--ArmoniK:Client:Grpc:ChannelDefaults:Grpc:Host:Receive:Window=6",
                                                                                                                 })))
                .ConfigureAwait(false);
    await using (var channel = runtime.Channel(Endpoint))
    {
      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(6),
                  "a command line");
    }

    runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromFiles(File("window.json",
                                                                                                             "{ \"ArmoniK\": { \"Client\": { \"Grpc\": { \"ChannelDefaults\": { \"Grpc\": { \"Host\": { \"Receive\": { \"Window\": 5 } } } } } } } }"))))
                .ConfigureAwait(false);
    await using (var channel = runtime.Channel(Endpoint))
    {
      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(5),
                  "a file");
    }
  }

  /// <summary>A channel's own window wins over the default, and with neither the engine's own is read back.</summary>
  [Test]
  public async Task AChannelsOwnWindowWinsAndWithNoneTheEnginesIsReadBack()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromObject(WithWindow(7))))
                    .ConfigureAwait(false);
    await using (var channel = runtime.Channel(Endpoint,
                                               2))
    {
      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(2));
    }

    runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromObject(new RuntimeOptions())))
                .ConfigureAwait(false);
    await using (var channel = runtime.Channel(Endpoint))
    {
      Assert.That(channel.DeliveryCredits,
                  Is.EqualTo(4));
    }
  }

  /// <summary>A default no ring can hold is refused when the channel is made, and leaves the runtime usable.</summary>
  [Test]
  public async Task ADefaultWindowNoRingCanHoldIsRefusedAtTheChannel()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                     {
                                                                                                                       $"--ArmoniK:Client:Grpc:ChannelDefaults:Grpc:Host:Receive:Window={NativeRuntime.MaxDeliveryCredits + 1}",
                                                                                                                     })))
                    .ConfigureAwait(false);

    Assert.That(() => runtime.Channel(Endpoint),
                Throws.InstanceOf<ArgumentOutOfRangeException>()
                      .With.Message.Contains("Grpc.Host.Receive.Window"));

    await using var channel = runtime.Channel(Endpoint,
                                              3);
    Assert.That(channel.DeliveryCredits,
                Is.EqualTo(3));
  }

  /// <summary>What the engine refuses in a source is refused when the runtime is created, by the source and the key, the value unquoted.</summary>
  [Test]
  public void AValueNotOfItsTypeIsRefusedAtTheCreateAndNotQuoted()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                             {
                                                                                                                               "--ArmoniK:Client:Grpc:MemoryCeiling=a-great-deal",
                                                                                                                             })))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("pairs: MemoryCeiling is refused")
                         .And.Message.Not.Contains("a-great-deal"));

  /// <summary>A command line states no list: it is refused by the list's path, with the sources that do state one.</summary>
  [Test]
  public void AListOnACommandLineIsRefusedByItsPath()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                             {
                                                                                                                               "--ArmoniK:Client:Grpc:ChannelDefaults:Grpc:Receive:Compression:0=Gzip",
                                                                                                                             })))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("pairs: ChannelDefaults.Grpc.Receive.Compression is refused")
                         .And.Message.Contains("a file, a document or an environment variable states"));

  /// <summary>An environment list that is not a JSON array is refused by its path, with the form it takes, and not quoted.</summary>
  [Test]
  public void AnEnvironmentListThatIsNotAJsonArrayIsRefusedAndNotQuoted()
  {
    const string prefix = "AKLISTREFUSED";
    const string name  = prefix + "__ChannelDefaults__Grpc__Receive__Compression";
    Environment.SetEnvironmentVariable(name,
                                       "Deflate,Zstd");

    try
    {
      Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration(prefix).LoadConfigFromEnvironment()))
                                .ConfigureAwait(false),
                  Throws.InstanceOf<InvalidOperationException>()
                        .With.Message.Contains("the environment: ChannelDefaults.Grpc.Receive.Compression is refused")
                        .And.Message.Contains("a JSON array")
                        .And.Message.Not.Contains("Deflate"));
    }
    finally
    {
      Environment.SetEnvironmentVariable(name,
                                         null);
    }
  }

  /// <summary>A file that does not exist is refused, by its path, unless it is optional.</summary>
  [Test]
  public void AMissingFileIsRefusedByItsPath()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromFiles(Path.Combine(directory_,
                                                                                                                                       "absent.json"))))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("absent.json is refused: the file does not exist"));

  /// <summary>With no prefix, every variable of the process would be a key, so the environment is refused.</summary>
  [Test]
  public void TheEnvironmentWithNoPrefixIsRefused()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration(string.Empty).LoadConfigFromEnvironment()))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("the environment is refused"));
}
