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
                                                                                           $"\"GrpcClient\": {{ \"Endpoint\": \"{Endpoint}\", " +
                                                                                           "\"ChannelDefaults\": { \"Grpc\": { \"UserAgent\": \"from-a-file\" } } } }")));

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
                                                                                             $"--GrpcClient:Endpoint={Endpoint}",
                                                                                             "--GrpcClient:ChannelDefaults:Transport:ConnectTimeoutSeconds=2.5",
                                                                                             "--GrpcClient:ChannelDefaults:Transport:Proxy:None=true",
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
                                                                                                  $"{{ \"GrpcClient\": {{ \"Endpoint\": \"{Endpoint}\" }} }}")));

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
                                                                                          "{ \"GrpcClient\": { \"Endpoint\": \"http://127.0.0.1:1\" } }"))
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

  /// <summary>What the engine refuses in a source is refused when the runtime is created, by the source and the key, the value unquoted.</summary>
  [Test]
  public void AValueNotOfItsTypeIsRefusedAtTheCreateAndNotQuoted()
    => Assert.That(async () => await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                             {
                                                                                                                               "--GrpcClient:MemoryCeiling=a-great-deal",
                                                                                                                             })))
                                 .ConfigureAwait(false),
                   Throws.InstanceOf<InvalidOperationException>()
                         .With.Message.Contains("pairs: MemoryCeiling is refused")
                         .And.Message.Not.Contains("a-great-deal"));

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
