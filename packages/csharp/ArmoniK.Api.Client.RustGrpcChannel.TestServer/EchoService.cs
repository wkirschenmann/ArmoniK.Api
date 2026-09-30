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
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Http.Features;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

public class EchoService : Echo.EchoBase
{
  // Request entries under this prefix come back in Say's head, and Say's reply lists them as it
  // read them.
  private const string ReflectedPrefix = "x-reflect-";

  public override async Task<EchoReply> Say(EchoRequest request,
                                            ServerCallContext context)
  {
    // Header to header, past grpc-dotnet's Metadata, whose RequestHeaders joins a repeated key's
    // values into one and whose head loses an empty value. The indexer rather than Append, which
    // drops an empty value too.
    var http = context.GetHttpContext();
    foreach (var (key, values) in http.Request.Headers)
    {
      if (key.StartsWith(ReflectedPrefix,
                         StringComparison.Ordinal))
      {
        http.Response.Headers[key] = values;
      }
    }

    await context.WriteResponseHeadersAsync(new Metadata
                                            {
                                              {
                                                "x-answered", "yes"
                                              },
                                            })
                 .ConfigureAwait(false);

    return new EchoReply
           {
             Text        = request.Text,
             SawMetadata = Reflected(http.Request.Headers) ?? Saw(context.RequestHeaders),
           };
  }

  /// <summary>Answers each request message with the same text, as it arrives.</summary>
  public override async Task Chat(IAsyncStreamReader<EchoRequest>  requests,
                                  IServerStreamWriter<EchoReply>   responses,
                                  ServerCallContext                context)
  {
    while (await requests.MoveNext(context.CancellationToken)
                         .ConfigureAwait(false))
    {
      await responses.WriteAsync(new EchoReply
                                 {
                                   Text = requests.Current.Text,
                                 })
                     .ConfigureAwait(false);
    }
  }

  /// <summary>Answers one message per comma-separated part of the request.</summary>
  public override async Task Fan(EchoRequest                     request,
                                 IServerStreamWriter<EchoReply>  responses,
                                 ServerCallContext               context)
  {
    if (request.Text.Length == 0)
    {
      return;
    }

    foreach (var part in request.Text.Split(','))
    {
      await responses.WriteAsync(new EchoReply
                                 {
                                   Text = part,
                                 })
                     .ConfigureAwait(false);
    }
  }

  /// <summary>Reads every request message and answers once, naming what it saw.</summary>
  public override async Task<EchoReply> Collect(IAsyncStreamReader<EchoRequest> requests,
                                                ServerCallContext              context)
  {
    var seen = new List<string>();
    while (await requests.MoveNext(context.CancellationToken)
                         .ConfigureAwait(false))
    {
      seen.Add(requests.Current.Text);
    }

    return new EchoReply
           {
             Text = $"{seen.Count}:{string.Join(",", seen)}",
           };
  }

  public override Task<EchoReply> Refuse(EchoRequest request,
                                         ServerCallContext context)
  {
    var trailers = new Metadata
                   {
                     {
                       "x-reason", "policy"
                     },
                   };
    throw new RpcException(new Status(StatusCode.PermissionDenied,
                                      "not for you"),
                           trailers);
  }

  public override async Task<EchoReply> Never(EchoRequest request,
                                              ServerCallContext context)
  {
    await Task.Delay(Timeout.Infinite,
                     context.CancellationToken)
              .ConfigureAwait(false);
    return new EchoReply();
  }

  public override async Task<EchoReply> Reset(EchoRequest request,
                                              ServerCallContext context)
  {
    if (request.Text == "after the head")
    {
      await context.WriteResponseHeadersAsync(new Metadata())
                   .ConfigureAwait(false);
    }

    const int enhanceYourCalm = 0xb;
    context.GetHttpContext()
           .Features.Get<IHttpResetFeature>()!
           .Reset(enhanceYourCalm);

    await Task.Delay(Timeout.Infinite,
                     context.CancellationToken)
              .ConfigureAwait(false);
    return new EchoReply();
  }

  /// <summary>Ends the call without reading, so the client's next write meets a call that is over.</summary>
  public override Task<EchoReply> CollectRefused(IAsyncStreamReader<EchoRequest> requests,
                                                 ServerCallContext              context)
    => throw new RpcException(new Status(StatusCode.PermissionDenied,
                                          "not for you"));

  /// <summary>Response headers, then nothing, until the client gives up.</summary>
  /// <remarks>A stream that sends no message is what tells a read apart from no read: a client
  /// awaiting the headers here has nothing it could have read to get them.</remarks>
  public override async Task HeadOnly(EchoRequest                    request,
                                      IServerStreamWriter<EchoReply> responses,
                                      ServerCallContext              context)
  {
    await context.WriteResponseHeadersAsync(new Metadata
                                            {
                                              {
                                                "x-answered", "yes"
                                              },
                                            })
                 .ConfigureAwait(false);

    await Task.Delay(Timeout.Infinite,
                     context.CancellationToken)
              .ConfigureAwait(false);
  }

  /// <summary>The same for the duplex cardinality, which has its own reader machine.</summary>
  public override async Task HeadThenChat(IAsyncStreamReader<EchoRequest> requests,
                                          IServerStreamWriter<EchoReply>  responses,
                                          ServerCallContext               context)
  {
    await context.WriteResponseHeadersAsync(new Metadata
                                            {
                                              {
                                                "x-answered", "yes"
                                              },
                                            })
                 .ConfigureAwait(false);

    while (await requests.MoveNext(context.CancellationToken)
                         .ConfigureAwait(false))
    {
      await responses.WriteAsync(new EchoReply
                                 {
                                   Text = requests.Current.Text,
                                 })
                     .ConfigureAwait(false);
    }
  }

  private static string Saw(Metadata headers)
  {
    var binary = headers.GetValueBytes("x-trace-bin");
    if (binary is not null)
    {
      return BitConverter.ToString(binary)
                         .Replace("-",
                                  string.Empty)
                         .ToLowerInvariant();
    }

    return headers.GetValue("x-request") ?? string.Empty;
  }

  // One line per value, `key=value` with a binary value's bytes in hex, ordered by key. Null when
  // the request has no such entry.
  private static string? Reflected(IHeaderDictionary headers)
  {
    var lines = headers.Where(header => header.Key.StartsWith(ReflectedPrefix,
                                                              StringComparison.Ordinal))
                       .OrderBy(header => header.Key,
                                StringComparer.Ordinal)
                       .SelectMany(header => header.Value.Select(value => header.Key.EndsWith(Metadata.BinaryHeaderSuffix,
                                                                                              StringComparison.Ordinal)
                                                                            ? $"{header.Key}={BitConverter.ToString(FromBase64(value!))}"
                                                                            : $"{header.Key}={value}"))
                       .ToArray();
    return lines.Length == 0
             ? null
             : string.Join("\n",
                           lines);
  }

  // gRPC lets a sender leave base64's padding out, and Convert wants it.
  private static byte[] FromBase64(string base64)
    => Convert.FromBase64String(base64.PadRight((base64.Length + 3) / 4 * 4,
                                                '='));
}
