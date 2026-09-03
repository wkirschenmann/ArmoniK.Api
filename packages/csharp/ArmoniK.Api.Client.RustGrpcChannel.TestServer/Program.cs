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

/// <summary>
///   The echo server the client tests dial, as its own process.
/// </summary>
/// <remarks>
///   It binds a port the operating system chooses and prints it, so several test runs may go on at
///   once without agreeing a number in advance. Whoever started it reads that line and then kills
///   it; nothing here listens for a shutdown of its own.
/// </remarks>
public static class Program
{
  /// <summary>What the first line of standard output starts with, so a reader can find the port.</summary>
  public const string EndpointPrefix = "ENDPOINT ";

  public static async Task Main(string[] args)
  {
    var builder = WebApplication.CreateBuilder(args);

    // 127.0.0.1 and not localhost: Kestrel refuses a dynamic port on the latter, which resolves
    // to two addresses. HTTP/2 outright because the engine under test dials h2c with no upgrade.
    builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback,
                                                               0,
                                                               listen => listen.Protocols = HttpProtocols.Http2));
    builder.Services.AddGrpc();
    // The endpoint line is the only thing on stdout, so a reader needs no parsing to find it.
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
