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
using System.Globalization;
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

  private static ChannelOptions Compressing(MessageEncoding? send   = null,
                                            MessageEncoding? accept = null)
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
                                  Compression = accept,
                                },
                },
       };

  private async Task<(EchoReply Reply, Metadata Head, Metadata Trailers)> Say(ChannelOptions options,
                                                                               bool           asksForACompressedReply = false)
  {
    await using var channel = Runtime.Channel(Endpoint,
                                              options);
    var headers = new Metadata();
    if (asksForACompressedReply)
    {
      headers.Add("x-compress-response",
                  "yes");
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
    => Assert.That(Encoding.UTF8.GetString(Compressing(MessageEncoding.Gzip,
                                                       MessageEncoding.Gzip)
                                             .Encode()),
                   Is.EqualTo(@"{""Grpc"":{""Send"":{""Compression"":""Gzip""},""Receive"":{""Compression"":""Gzip""}}}"));

  /// <summary>A name the vocabulary does not declare is refused before it is sent.</summary>
  [Test]
  public void AnEncodingTheVocabularyDoesNotDeclareIsRefusedBeforeItIsSent()
    => Assert.That(() => Compressing((MessageEncoding)42)
                           .Encode(),
                   Throws.TypeOf<ArgumentOutOfRangeException>()
                         .With.Message.Contains("Compression has to be a name MessageEncoding declares"));

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
    var options = Compressing(accept: MessageEncoding.Gzip);
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
                              MessageEncoding.Gzip);
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

  /// <summary>The configuration loader reads the encodings by their names and hands them to the channel.</summary>
  [Test]
  public async Task TheLoaderReadsTheEncodingsByTheirNames()
  {
    var runtime = await RestartAsync(() => NativeRuntime.Create(new NativeConfiguration().LoadConfigFromCommandLine(new[]
                                                                                                                {
                                                                                                                  $"--GrpcClient:Endpoint={Endpoint}",
                                                                                                                  "--GrpcClient:ChannelDefaults:Grpc:Send:Compression=Gzip",
                                                                                                                  "--GrpcClient:ChannelDefaults:Grpc:Receive:Compression=Gzip",
                                                                                                                })))
                    .ConfigureAwait(false);
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
                                  Is.EqualTo("gzip,identity"));
                    });
  }
}
