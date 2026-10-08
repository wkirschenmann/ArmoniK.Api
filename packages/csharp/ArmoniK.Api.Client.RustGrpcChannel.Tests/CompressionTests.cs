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
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading.Tasks;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The compression of a call's messages, against grpc-dotnet's server.</summary>
/// <remarks>
///   The Rust engine's own tests hold what goes on the wire, frame by frame. This holds what a
///   .NET caller reaches through the generated options and the ABI: that the options are spelled
///   as the schema names them, that the engine reads them, and that the reference server inflates
///   what the engine compresses and the engine inflates what the reference server compresses.
/// </remarks>
[TestFixture]
public class CompressionTests : EchoServerFixture
{
  /// <summary>Large, and compressible.</summary>
  private static readonly string Text = new('a',
                                            200 * 1024);

  private static ChannelOptions Compressing(MessageEncoding?   send   = null,
                                            MessageEncoding[]? accept = null)
    => new()
       {
         Grpc = new GrpcOptions
                {
                  Send = send is null
                           ? null
                           : new GrpcSendOptions
                             {
                               Compression = send,
                             },
                  Receive = accept is null
                              ? null
                              : new GrpcReceiveOptions
                                {
                                  Compression = accept.ToList(),
                                },
                },
       };

  private async Task<(EchoReply Reply, Metadata Head, Metadata Trailers)> Say(ChannelOptions options,
                                                                               bool           asksForACompressedReply = false,
                                                                               string         replyIn                 = "yes")
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              options);
    var headers = new Metadata();
    if (asksForACompressedReply)
    {
      headers.Add("x-compress-response",
                  replyIn);
    }

    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = Text,
                },
                headers);
    var reply = await call.ResponseAsync.ConfigureAwait(false);
    var head = await call.ResponseHeadersAsync.ConfigureAwait(false);
    return (reply, head, call.GetTrailers());
  }

  /// <summary>How many bytes the request's body held on the wire, as the server saw it.</summary>
  private static long WireBytes(Metadata head)
    => long.Parse(head.GetValue("x-saw-content-length") ?? throw new AssertionException("the request stated no length"),
                  CultureInfo.InvariantCulture);

  /// <summary>The options are spelled as the schema names them.</summary>
  [Test]
  public void TheOptionsAreSpelledAsTheSchemaNamesThem()
  {
    Assert.That(Encoding.UTF8.GetString(Compressing(MessageEncoding.Gzip,
                                                    new[]
                                                    {
                                                      MessageEncoding.Gzip,
                                                    })
                                          .Encode()),
                Is.EqualTo(@"{""Grpc"":{""Send"":{""Compression"":""Gzip""},""Receive"":{""Compression"":[""Gzip""]}}}"));

    Assert.That(Encoding.UTF8.GetString(Compressing(MessageEncoding.Zstd,
                                                    new[]
                                                    {
                                                      MessageEncoding.Zstd,
                                                      MessageEncoding.Deflate,
                                                      MessageEncoding.Gzip,
                                                    })
                                          .Encode()),
                Is.EqualTo(@"{""Grpc"":{""Send"":{""Compression"":""Zstd""},""Receive"":{""Compression"":[""Zstd"",""Deflate"",""Gzip""]}}}"),
                "the order is the one stated");
  }

  /// <summary>A name the vocabulary does not declare is refused before it is sent.</summary>
  [Test]
  public void AnEncodingTheVocabularyDoesNotDeclareIsRefusedBeforeItIsSent()
  {
    Assert.That(() => Compressing((MessageEncoding)42)
                        .Encode(),
                Throws.TypeOf<ArgumentOutOfRangeException>()
                      .With.Message.Contains("Compression has to be a name MessageEncoding declares"));

    Assert.That(() => Compressing(accept: new[]
                                          {
                                            MessageEncoding.Gzip,
                                            (MessageEncoding)42,
                                          })
                        .Encode(),
                Throws.TypeOf<ArgumentOutOfRangeException>()
                      .With.Message.Contains("Compression has to be names MessageEncoding declares"));
  }

  /// <summary>A copy of the options has a list of its own.</summary>
  [Test]
  public void ACopyOfTheOptionsHasAListOfItsOwn()
  {
    var options = new GrpcReceiveOptions
                  {
                    Compression = new List<MessageEncoding>
                                  {
                                    MessageEncoding.Gzip,
                                  },
                  };
    var copy = new GrpcReceiveOptions(options);

    options.Compression.Add(MessageEncoding.Zstd);

    Assert.That(copy.Compression,
                Is.EqualTo(new[]
                           {
                             MessageEncoding.Gzip,
                           }));
  }

  /// <summary>A channel that sets neither states no encoding of its own, and accepts identity only.</summary>
  [Test]
  public async Task AChannelThatSetsNeitherSendsAsItIsAndAcceptsIdentityOnly()
  {
    var (reply, head, trailers) = await Say(new ChannelOptions())
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.Null.Or.Empty);
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("identity"));
                      Assert.That(WireBytes(head),
                                  Is.GreaterThanOrEqualTo(Text.Length),
                                  "the request went as it was");
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.Null.Or.Empty);
                    });
  }

  /// <summary>What the engine compresses, grpc-dotnet's server inflates.</summary>
  [Test]
  public async Task AMessageTheEngineCompressesIsInflatedByTheReferenceServer()
  {
    var (reply, head, trailers) = await Say(Compressing(MessageEncoding.Gzip))
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.EqualTo("gzip"));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("identity"),
                                  "sending an encoding does not accept it");
                      Assert.That(WireBytes(head),
                                  Is.LessThan(Text.Length / 10),
                                  "the request went compressed");
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.Null.Or.Empty);
                    });
  }

  /// <summary>What the reference server compresses, the engine inflates.</summary>
  [Test]
  public async Task AReplyTheReferenceServerCompressesIsInflatedByTheEngine()
  {
    var options = Compressing(accept: new[]
                                      {
                                        MessageEncoding.Gzip,
                                      });
    var (reply, head, trailers) = await Say(options,
                                            asksForACompressedReply: true)
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("gzip,identity"));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.Null.Or.Empty);
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.EqualTo("gzip"),
                                  "the reply went compressed");
                    });
  }

  /// <summary>Both at once.</summary>
  [Test]
  public async Task BothDirectionsCompressedWorkTogether()
  {
    var options = Compressing(MessageEncoding.Gzip,
                              new[]
                              {
                                MessageEncoding.Gzip,
                              });
    var (reply, head, trailers) = await Say(options,
                                            asksForACompressedReply: true)
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.EqualTo("gzip"));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("gzip,identity"));
                      Assert.That(WireBytes(head),
                                  Is.LessThan(Text.Length / 10));
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.EqualTo("gzip"));
                    });
  }

  /// <summary>The list is advertised in the order stated, and the reference server compresses in one of them.</summary>
  /// <remarks>The echo service names the encoding of its reply, gzip here, and the list names it.</remarks>
  [Test]
  public async Task TheListIsAdvertisedInOrderAndTheServerAnswersInAnEncodingOfIt()
  {
    var options = Compressing(accept: new[]
                                      {
                                        MessageEncoding.Zstd,
                                        MessageEncoding.Deflate,
                                        MessageEncoding.Gzip,
                                      });
    var (reply, head, trailers) = await Say(options,
                                            asksForACompressedReply: true)
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("zstd,deflate,gzip,identity"));
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.EqualTo("gzip"),
                                  "the reply went compressed, in the one the server knows");
                    });
  }

  /// <summary>A name listed twice is advertised once, at its first place.</summary>
  [Test]
  public async Task ANameListedTwiceIsAdvertisedOnce()
  {
    var options = Compressing(accept: new[]
                                      {
                                        MessageEncoding.Deflate,
                                        MessageEncoding.Gzip,
                                        MessageEncoding.Deflate,
                                        MessageEncoding.Gzip,
                                      });
    var (reply, head, _) = await Say(options)
                             .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("deflate,gzip,identity"));
                    });
  }

  /// <summary>An empty list is the default's: identity alone.</summary>
  [Test]
  public async Task AnEmptyListAcceptsIdentityOnly()
  {
    var (reply, head, _) = await Say(Compressing(accept: Array.Empty<MessageEncoding>()))
                             .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("identity"));
                    });
  }

  /// <summary>What the engine compresses with deflate, which is gRPC's zlib structure, grpc-dotnet inflates.</summary>
  /// <remarks>
  ///   grpc-dotnet's servers accept gzip and deflate by default, and its deflate is ZLibStream,
  ///   so this is the structure of RFC 1950 read by another implementation.
  /// </remarks>
  [Test]
  public async Task AMessageTheEngineCompressesWithDeflateIsInflatedByTheReferenceServer()
  {
    var (reply, head, _) = await Say(Compressing(MessageEncoding.Deflate))
                             .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.EqualTo("deflate"));
                      Assert.That(WireBytes(head),
                                  Is.LessThan(Text.Length / 10),
                                  "the request went compressed");
                    });
  }

  /// <summary>What the reference server compresses with deflate, the engine inflates.</summary>
  [Test]
  public async Task AReplyTheReferenceServerCompressesWithDeflateIsInflatedByTheEngine()
  {
    var (reply, head, trailers) = await Say(Compressing(accept: new[]
                                                                {
                                                                  MessageEncoding.Deflate,
                                                                }),
                                            asksForACompressedReply: true,
                                            replyIn: "deflate")
                                    .ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("deflate,identity"));
                      Assert.That(trailers.GetValue("x-sent-encoding"),
                                  Is.EqualTo("deflate"),
                                  "the reply went compressed");
                    });
  }

  /// <summary>
  ///   A server that does not accept the encoding fails the first call, and the channel then
  ///   sends as it is.
  /// </summary>
  /// <remarks>
  ///   grpc-dotnet's server accepts gzip and deflate and has no provider for zstd, which it
  ///   answers with UNIMPLEMENTED and the list it accepts: what the engine learns from.
  /// </remarks>
  [Test]
  public async Task AServerThatDoesNotAcceptTheEncodingFailsTheFirstCallAndNotTheNext()
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              Compressing(MessageEncoding.Zstd));
    var client = Client(channel);

    var refused = Assert.ThrowsAsync<RpcException>(async () => await client.SayAsync(new EchoRequest
                                                                                     {
                                                                                       Text = Text,
                                                                                     })
                                                                           .ResponseAsync.ConfigureAwait(false));

    Assert.That(refused!.StatusCode,
                Is.EqualTo(StatusCode.Unimplemented));

    for (var call = 0; call < 3; call++)
    {
      using var next = client.SayAsync(new EchoRequest
                                       {
                                         Text = Text,
                                       });
      var reply = await next.ResponseAsync.ConfigureAwait(false);
      var head = await next.ResponseHeadersAsync.ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(reply.Text,
                                    Is.EqualTo(Text));
                        Assert.That(head.GetValue("x-saw-encoding"),
                                    Is.Null.Or.Empty,
                                    "the channel stopped compressing");
                        Assert.That(WireBytes(head),
                                    Is.GreaterThanOrEqualTo(Text.Length),
                                    "the request went as it was");
                      });
    }
  }

  /// <summary>An encoding the server accepts goes on being used call after call.</summary>
  [Test]
  public async Task AnEncodingTheServerAcceptsGoesOnBeingUsed()
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              Compressing(MessageEncoding.Gzip));
    var client = Client(channel);

    for (var call = 0; call < 3; call++)
    {
      using var next = client.SayAsync(new EchoRequest
                                       {
                                         Text = Text,
                                       });
      await next.ResponseAsync.ConfigureAwait(false);
      var head = await next.ResponseHeadersAsync.ConfigureAwait(false);

      Assert.That(WireBytes(head),
                  Is.LessThan(Text.Length / 10),
                  "the request went compressed");
    }
  }

  /// <summary>
  ///   The configuration loader reads the encodings by their names and hands them to the channel:
  ///   the send encoding from the command line, the accepted ones, a list, from a file.
  /// </summary>
  /// <remarks>
  ///   A list is stated by a file, a document or an environment variable holding a JSON array: a
  ///   command line and pairs state none.
  /// </remarks>
  [Test]
  public async Task TheLoaderReadsTheEncodingsByTheirNames()
  {
    var directory = Path.Combine(Path.GetTempPath(),
                                 "armonik-compression-" + Guid.NewGuid()
                                                              .ToString("N"));
    Directory.CreateDirectory(directory);
    NativeRuntime runtime;

    try
    {
      var file = Path.Combine(directory,
                              "compression.json");
      File.WriteAllText(file,
                        @"{ ""ArmoniK"": { ""Client"": { ""Grpc"": { ""ChannelDefaults"": { ""Grpc"": { ""Receive"": { ""Compression"": [""Zstd"", ""Gzip""] } } } } } } }");

      runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromFiles(file)
                                                                                       .LoadConfigFromCommandLine(new[]
                                                                                                                  {
                                                                                                                    $"--ArmoniK:Client:Grpc:Endpoint={Endpoint}",
                                                                                                                    "--ArmoniK:Client:Grpc:ChannelDefaults:Grpc:Send:Compression=Gzip",
                                                                                                                  })))
                  .ConfigureAwait(false);

      await SendsInGzipAndAcceptsZstdThenGzip(runtime)
        .ConfigureAwait(false);
    }
    finally
    {
      Directory.Delete(directory,
                       true);
    }
  }

  /// <summary>The environment states the list as one variable holding a JSON array.</summary>
  [Test]
  public async Task TheLoaderReadsTheEncodingsFromTheEnvironment()
  {
    const string prefix = "AKCOMPRESSIONLIST";

    var variables = new[]
                    {
                      ("Endpoint", Endpoint),
                      ("ChannelDefaults__Grpc__Send__Compression", "Gzip"),
                      ("ChannelDefaults__Grpc__Receive__Compression", @"[""Zstd"", ""Gzip""]"),
                    };

    foreach (var (name, value) in variables)
    {
      Environment.SetEnvironmentVariable(prefix + "__" + name,
                                         value);
    }

    try
    {
      var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration(prefix).LoadConfigFromEnvironment()))
                      .ConfigureAwait(false);
      await SendsInGzipAndAcceptsZstdThenGzip(runtime)
        .ConfigureAwait(false);
    }
    finally
    {
      foreach (var (name, _) in variables)
      {
        Environment.SetEnvironmentVariable(prefix + "__" + name,
                                           null);
      }
    }
  }

  private async Task SendsInGzipAndAcceptsZstdThenGzip(NativeRuntime runtime)
  {
    await using var channel = runtime.Channel(string.Empty);
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = Text,
                });
    var reply = await call.ResponseAsync.ConfigureAwait(false);
    var head = await call.ResponseHeadersAsync.ConfigureAwait(false);

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo(Text));
                      Assert.That(head.GetValue("x-saw-encoding"),
                                  Is.EqualTo("gzip"));
                      Assert.That(head.GetValue("x-saw-accept-encoding"),
                                  Is.EqualTo("zstd,gzip,identity"));
                    });
  }
}
