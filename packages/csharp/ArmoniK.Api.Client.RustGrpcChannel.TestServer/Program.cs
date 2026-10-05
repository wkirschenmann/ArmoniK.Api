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
using System.Linq;
using System.Net;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
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

  /// <summary>Precedes the path of the authority a `--tls` server's certificate chains to.</summary>
  /// <remarks>A `--tls` server stops when its stdin closes, and removes that file as it does.</remarks>
  public const string AuthorityPrefix = "CA ";

  public static async Task Main(string[] args)
  {
    var tls = args.Contains("--tls");
    var builder = WebApplication.CreateBuilder(args.Where(arg => arg != "--tls")
                                                   .ToArray());

    var (certificate, authority) = tls
                                     ? Certificate()
                                     : (null, null);
    try
    {
      await Serve(builder,
                  certificate,
                  tls)
        .ConfigureAwait(false);
    }
    finally
    {
      certificate?.Dispose();
      if (authority is not null)
      {
        File.Delete(authority);
      }
    }
  }

  private static async Task Serve(WebApplicationBuilder builder,
                                  X509Certificate2?     certificate,
                                  bool                  tls)
  {
    builder.WebHost.ConfigureKestrel(options =>
                                     {
                                       // gRPC lifts Kestrel's 30 MB body limit for streaming calls
                                       // only, and the benchmarks send a unary message of 150 MiB.
                                       options.Limits.MaxRequestBodySize = null;
                                       options.Listen(IPAddress.Loopback,
                                                      0,
                                                      listen =>
                                                      {
                                                        listen.Protocols = HttpProtocols.Http2;
                                                        if (certificate is not null)
                                                        {
                                                          listen.UseHttps(certificate);
                                                        }
                                                      });
                                     });
    // No limit on what a call sends, as ArmoniK's own workers have none: the benchmarks send a
    // message of 150 MiB.
    builder.Services.AddGrpc(options => options.MaxReceiveMessageSize = null);
    builder.Logging.ClearProviders();

    var server = builder.Build();
    server.MapGrpcService<EchoService>();
    await server.StartAsync()
                .ConfigureAwait(false);

    Console.WriteLine(EndpointPrefix + server.Urls.First());
    Console.Out.Flush();

    if (tls)
    {
      // Ended by its host closing stdin rather than by a kill, so that what it made goes with it:
      // the authority's file, and the key container Windows keeps for an imported key until the
      // certificate holding it is disposed.
      _ = Task.Run(async () =>
                   {
                     await Console.In.ReadToEndAsync()
                                  .ConfigureAwait(false);
                     await server.StopAsync()
                                 .ConfigureAwait(false);
                   });
    }

    await server.WaitForShutdownAsync()
                .ConfigureAwait(false);
  }

  /// <summary>
  ///   A certificate for the loopback address, signed by an authority of its own whose PEM is
  ///   written to a file and named on stdout, so a client can trust it without the machine's store.
  ///   The file's path comes back with it, for the server to remove when it stops.
  /// </summary>
  /// <remarks>
  ///   An authority and a leaf rather than one self-signed certificate: rustls refuses a trust
  ///   anchor that is also the end entity it is asked to verify.
  /// </remarks>
  private static (X509Certificate2 Certificate, string Authority) Certificate()
  {
    using var authorityKey = RSA.Create(2048);
    var authorityRequest = new CertificateRequest("CN=ArmoniK test authority",
                                                  authorityKey,
                                                  HashAlgorithmName.SHA256,
                                                  RSASignaturePadding.Pkcs1);
    authorityRequest.CertificateExtensions.Add(new X509BasicConstraintsExtension(true,
                                                                                 false,
                                                                                 0,
                                                                                 true));
    authorityRequest.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.KeyCertSign,
                                                                         true));
    using var authority = authorityRequest.CreateSelfSigned(DateTimeOffset.UtcNow.AddDays(-1),
                                                            DateTimeOffset.UtcNow.AddDays(1));

    using var leafKey = RSA.Create(2048);
    var leafRequest = new CertificateRequest("CN=localhost",
                                             leafKey,
                                             HashAlgorithmName.SHA256,
                                             RSASignaturePadding.Pkcs1);
    var names = new SubjectAlternativeNameBuilder();
    names.AddDnsName("localhost");
    names.AddIpAddress(IPAddress.Loopback);
    leafRequest.CertificateExtensions.Add(names.Build());
    leafRequest.CertificateExtensions.Add(new X509BasicConstraintsExtension(false,
                                                                            false,
                                                                            0,
                                                                            true));
    using var leaf = leafRequest.Create(authority,
                                        DateTimeOffset.UtcNow.AddHours(-1),
                                        DateTimeOffset.UtcNow.AddHours(23),
                                        RandomNumberGenerator.GetBytes(16));

    var path = Path.Combine(Path.GetTempPath(),
                            $"armonik-test-authority-{Guid.NewGuid():N}.pem");
    File.WriteAllText(path,
                      authority.ExportCertificatePem());
    Console.WriteLine(AuthorityPrefix + path);

    // Through a PKCS#12 round trip: on Windows, Schannel will not use a key that only lives in an
    // ephemeral CNG handle.
    return (new X509Certificate2(leaf.CopyWithPrivateKey(leafKey)
                                     .Export(X509ContentType.Pkcs12)), path);
  }
}
