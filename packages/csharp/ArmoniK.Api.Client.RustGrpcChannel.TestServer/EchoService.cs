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
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What the tests call: an echo, a refusal, and a method that never answers.</summary>
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

  /// <summary>What the request carried, so a test can see it crossed rather than assume it.</summary>
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
