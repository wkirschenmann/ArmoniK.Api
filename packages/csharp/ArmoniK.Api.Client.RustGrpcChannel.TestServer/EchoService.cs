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
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

public class EchoService : Echo.EchoBase
{
  public override async Task<EchoReply> Say(EchoRequest request,
                                            ServerCallContext context)
  {
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
             SawMetadata = Saw(context.RequestHeaders),
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
}
