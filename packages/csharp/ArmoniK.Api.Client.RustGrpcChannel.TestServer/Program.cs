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
using System.Linq;
using System.Net;
using System.Threading.Tasks;

using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

public static class Program
{
  public const string EndpointPrefix = "ENDPOINT ";

  public static async Task Main(string[] args)
  {
    var builder = WebApplication.CreateBuilder(args);

    builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback,
                                                               0,
                                                               listen => listen.Protocols = HttpProtocols.Http2));
    builder.Services.AddGrpc();
    builder.Logging.ClearProviders();

    var server = builder.Build();
    server.MapGrpcService<EchoService>();
    await server.StartAsync()
                .ConfigureAwait(false);

    Console.WriteLine(EndpointPrefix + server.Urls.First());
    Console.Out.Flush();

    await server.WaitForShutdownAsync()
                .ConfigureAwait(false);
  }
}
