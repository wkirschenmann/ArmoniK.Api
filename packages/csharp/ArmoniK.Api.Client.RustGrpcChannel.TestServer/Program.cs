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
using System.IO;
using System.Linq;
using System.Net;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Threading.Tasks;

using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.AspNetCore.Server.Kestrel.Https;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

public static class Program
{
  public const string EndpointPrefix = "ENDPOINT ";

  /// <summary>Precedes the path of the authority a `--tls` server's certificate chains to.</summary>
  /// <remarks>A `--tls` server stops when its stdin closes, and removes the files it wrote as it does.</remarks>
  public const string AuthorityPrefix = "CA ";

  /// <summary>Precedes the path of the PEM certificate a `--mtls` server accepts from a client,
  /// issued by the same authority.</summary>
  public const string ClientCertificatePrefix = "CLIENT_CERT ";

  /// <summary>Precedes the path of that certificate's PKCS#8 PEM key.</summary>
  public const string ClientKeyPrefix = "CLIENT_KEY ";

  public static async Task Main(string[] args)
  {
    // `--mtls` is `--tls` that also requires a client certificate issued by the server's authority.
    var mtls = args.Contains("--mtls");
    var tls  = mtls || args.Contains("--tls");
    // `--plain-say` answers Say with the reply and `x-answered` alone.
    var plainSay = args.Contains("--plain-say");

    // `--port N` listens there rather than at a port the system picks, for a test that starts the
    // server after it dialled.
    var portAt = Array.IndexOf(args,
                               "--port");
    var port = portAt < 0
                 ? 0
                 : int.Parse(args[portAt + 1]);
    var builder = WebApplication.CreateBuilder(args.Where((arg,
                                                           at) => arg != "--tls" && arg != "--mtls" && arg != "--plain-say" &&
                                                                  (portAt < 0 || (at != portAt && at != portAt + 1)))
                                                   .ToArray());
    builder.Services.AddSingleton(new SayShape(plainSay));

    var credentials = tls
                        ? Credentials.Create(mtls)
                        : null;
    try
    {
      await Serve(builder,
                  credentials,
                  port)
        .ConfigureAwait(false);
    }
    finally
    {
      credentials?.Dispose();
    }
  }

  private static async Task Serve(WebApplicationBuilder builder,
                                  Credentials?          credentials,
                                  int                   port)
  {
    builder.WebHost.ConfigureKestrel(options =>
                                     {
                                       // gRPC lifts Kestrel's 30 MB body limit for streaming calls
                                       // only, and the benchmarks send a unary message of 150 MiB.
                                       options.Limits.MaxRequestBodySize = null;
                                       options.Listen(IPAddress.Loopback,
                                                      port,
                                                      listen =>
                                                      {
                                                        listen.Protocols = HttpProtocols.Http2;
                                                        if (credentials is not null)
                                                        {
                                                          listen.UseHttps(credentials.Https());
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

    if (credentials is not null)
    {
      // Ended by its host closing stdin rather than by a kill, so that what it made goes with it:
      // the files it wrote, and the key container Windows keeps for an imported key until the
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
  ///   Under mTLS, also a client certificate issued by the same authority: the certificate and its
  ///   key go to PEM files named on stdout the same way. The server removes every file it wrote
  ///   when it stops, or when it fails to make them all.
  /// </summary>
  /// <remarks>
  ///   An authority and a leaf rather than one self-signed certificate: rustls refuses a trust
  ///   anchor that is also the end entity it is asked to verify.
  /// </remarks>
  private sealed class Credentials : IDisposable
  {
    private readonly X509Certificate2 authority_;
    private readonly X509Certificate2 certificate_;
    private readonly string[]         files_;
    private readonly bool             requireClient_;

    private Credentials(X509Certificate2 authority,
                        X509Certificate2 certificate,
                        bool             requireClient,
                        string[]         files)
    {
      authority_     = authority;
      certificate_   = certificate;
      requireClient_ = requireClient;
      files_         = files;
    }

    public void Dispose()
    {
      certificate_.Dispose();
      authority_.Dispose();
      Delete(files_);
    }

    private static void Delete(IEnumerable<string> files)
    {
      foreach (var file in files)
      {
        File.Delete(file);
      }
    }

    internal static Credentials Create(bool mtls)
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

      var authorityPem = authority.ExportCertificatePem();
      var files        = new List<string>();
      try
      {
        Console.WriteLine(AuthorityPrefix + Write(files,
                                                  "authority",
                                                  authorityPem));

        if (mtls)
        {
          using var clientKey = RSA.Create(2048);
          var clientRequest = new CertificateRequest("CN=ArmoniK test client",
                                                     clientKey,
                                                     HashAlgorithmName.SHA256,
                                                     RSASignaturePadding.Pkcs1);
          clientRequest.CertificateExtensions.Add(new X509BasicConstraintsExtension(false,
                                                                                    false,
                                                                                    0,
                                                                                    true));
          clientRequest.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension(new OidCollection
                                                                                    {
                                                                                      new Oid("1.3.6.1.5.5.7.3.2"),
                                                                                    },
                                                                                    false));
          using var client = clientRequest.Create(authority,
                                                  DateTimeOffset.UtcNow.AddHours(-1),
                                                  DateTimeOffset.UtcNow.AddHours(23),
                                                  RandomNumberGenerator.GetBytes(16));
          Console.WriteLine(ClientCertificatePrefix + Write(files,
                                                            "client",
                                                            client.ExportCertificatePem()));
          Console.WriteLine(ClientKeyPrefix + Write(files,
                                                    "client-key",
                                                    clientKey.ExportPkcs8PrivateKeyPem()));
        }

        // Through a PKCS#12 round trip: on Windows, Schannel will not use a key that only lives in an
        // ephemeral CNG handle.
        return new Credentials(X509Certificate2.CreateFromPem(authorityPem),
                               new X509Certificate2(leaf.CopyWithPrivateKey(leafKey)
                                                        .Export(X509ContentType.Pkcs12)),
                               mtls,
                               files.ToArray());
      }
      catch
      {
        Delete(files);
        throw;
      }
    }

    /// <summary>Kestrel's HTTPS options: the server's certificate, and under mTLS a client
    /// certificate it requires and accepts only when it chains to the authority.</summary>
    internal HttpsConnectionAdapterOptions Https()
    {
      var https = new HttpsConnectionAdapterOptions
                  {
                    ServerCertificate = certificate_,
                  };
      if (requireClient_)
      {
        https.ClientCertificateMode = ClientCertificateMode.RequireCertificate;
        https.ClientCertificateValidation = (presented,
                                             _,
                                             _) =>
                                            {
                                              using var chain = new X509Chain();
                                              chain.ChainPolicy.TrustMode      = X509ChainTrustMode.CustomRootTrust;
                                              chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
                                              chain.ChainPolicy.CustomTrustStore.Add(authority_);
                                              return chain.Build(presented);
                                            };
      }

      return https;
    }

    private static string Write(List<string> files,
                                string       what,
                                string       pem)
    {
      var path = Path.Combine(Path.GetTempPath(),
                              $"armonik-test-{what}-{Guid.NewGuid():N}.pem");
      File.WriteAllText(path,
                        pem);
      files.Add(path);
      return path;
    }
  }
}
