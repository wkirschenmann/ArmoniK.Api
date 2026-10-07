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
using System.Threading.Tasks;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Request headers past what nginx takes.</summary>
/// <remarks>
///   nginx closes a connection whose request carries a header past `large_client_header_buffers`,
///   and every call on it with it. `Http2.Send.MaxHeaderListSize` refuses such a request on the
///   channel, so that it ends alone, with nothing sent and no connection used for it.
/// </remarks>
[TestFixture]
public class NginxHeaderLimitTests : EchoServerFixture
{
  private const string Buffers = "large_client_header_buffers 4 1k;";

  private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

  private NginxProcess? nginx_;

  [TearDown]
  public void StopNginx()
  {
    if (nginx_ is null)
    {
      return;
    }

    try
    {
      TestContext.WriteLine(nginx_.ErrorLog());
    }
    finally
    {
      nginx_.Dispose();
      nginx_ = null;
    }
  }

  /// <summary>The call is refused on the channel and the call beside it goes on.</summary>
  [Test]
  public async Task ARequestPastTheLimitEndsResourceExhaustedAndLeavesTheOtherCallsAlone()
  {
    await using var channel = Open(1000);
    var             client  = Client(channel);

    using var other = await Chat(client)
                        .ConfigureAwait(false);
    var error = Assert.ThrowsAsync<RpcException>(() => Say(client,
                                                           2000)
                                                   .ResponseAsync);

    Assert.Multiple(() =>
                    {
                      Assert.That(error!.StatusCode,
                                  Is.EqualTo(StatusCode.ResourceExhausted));
                      Assert.That(error.Status.Detail,
                                  Does.Contain("header list"));
                    });
    await Exchange(other,
                   "still here")
      .ConfigureAwait(false);
    Assert.That((await Say(client,
                           10)
                       .ResponseAsync.ConfigureAwait(false)).Text,
                Is.EqualTo("x"),
                "a request within the limit is served");
    Assert.That(nginx_!.ErrorLog(),
                Does.Not.Contain("too large"),
                "nginx was never shown the header");
  }

  /// <summary>Without the limit, the same request is what ends the call beside it.</summary>
  [Test]
  public async Task WithoutTheLimitTheSameRequestEndsTheOtherCallToo()
  {
    await using var channel = Open(null);
    var             client  = Client(channel);

    using var other = await Chat(client)
                        .ConfigureAwait(false);
    Assert.ThrowsAsync<RpcException>(() => Say(client,
                                               2000)
                                       .ResponseAsync);

    Assert.ThrowsAsync<RpcException>(() => Exchange(other,
                                                    "gone"));
    Assert.That(nginx_!.ErrorLog(),
                Does.Contain("too large header field"));
  }

  private NativeChannel Open(int? limit)
  {
    if (NginxProcess.Executable is null)
    {
      Assert.Ignore($"{NginxProcess.Variable} names no nginx");
    }

    nginx_ = NginxProcess.Start(Endpoint,
                                Buffers);
    return Runtime.Channel(nginx_.Endpoint,
                           new ChannelOptions
                           {
                             Http2 = new Http2Options
                                     {
                                       Send = new Http2SendOptions
                                              {
                                                MaxHeaderListSize = limit,
                                              },
                                     },
                           });
  }

  private static AsyncUnaryCall<EchoReply> Say(Echo.EchoClient client,
                                               int             padding)
    => client.SayAsync(new EchoRequest
                       {
                         Text = "x",
                       },
                       new Metadata
                       {
                         {
                           "x-pad", new string('a',
                                               padding)
                         },
                       },
                       DateTime.UtcNow + Patience);

  private static async Task<AsyncDuplexStreamingCall<EchoRequest, EchoReply>> Chat(Echo.EchoClient client)
  {
    var call = client.Chat(new Metadata
                           {
                             {
                               "x-call", "other"
                             },
                           },
                           DateTime.UtcNow + Patience);
    await Exchange(call,
                   "first")
      .ConfigureAwait(false);
    return call;
  }

  private static async Task Exchange(AsyncDuplexStreamingCall<EchoRequest, EchoReply> call,
                                     string                                           text)
  {
    await call.RequestStream.WriteAsync(new EchoRequest
                                        {
                                          Text = text,
                                        })
              .ConfigureAwait(false);
    Assert.That(await call.ResponseStream.MoveNext(default)
                          .ConfigureAwait(false),
                Is.True,
                $"an answer to `{text}`");
    Assert.That(call.ResponseStream.Current.Text,
                Is.EqualTo(text));
  }
}
