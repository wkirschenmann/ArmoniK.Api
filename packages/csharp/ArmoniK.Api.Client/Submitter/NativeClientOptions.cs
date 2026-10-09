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

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.RustGrpcChannel;

using NativeChannelOptions = ArmoniK.Api.Client.RustGrpcChannel.ChannelOptions;

namespace ArmoniK.Api.Client.Submitter
{
  /// <summary>
  ///   What a <see cref="GrpcClient" /> says, in the native engine's vocabulary
  /// </summary>
  internal static class NativeClientOptions
  {
    /// <summary>
    ///   The failures <see cref="GrpcClient" /> retries: the three statuses of its default, what grpc-dotnet maps to
    ///   UNAVAILABLE (a proxy's 502, 503 and 504 and a refused stream), and the connections that could not be made or
    ///   ended under the call. The statuses ABORTED and UNKNOWN count as acceptances for the engine's throttle.
    /// </summary>
    private static readonly IReadOnlyList<string> GrpcClientFailures = new[]
                                                                       {
                                                                         "Status.UNAVAILABLE",
                                                                         "Status.ABORTED",
                                                                         "Status.UNKNOWN",
                                                                         "Http.502",
                                                                         "Http.503",
                                                                         "Http.504",
                                                                         "Reset.REFUSED_STREAM",
                                                                         "Dial",
                                                                         "Connection",
                                                                       };

    /// <summary>
    ///   The channel options the engine reads for <paramref name="options" />
    /// </summary>
    /// <param name="options">The options of the client</param>
    /// <param name="onlySet">
    ///   True to translate only the options the caller set, even to their defaults, and leave the others to the
    ///   sources below; false to translate every option, as the defaults of a runtime
    /// </param>
    /// <returns>The channel options; a group of options with nothing stated is left null</returns>
    internal static NativeChannelOptions Translate(GrpcClient options,
                                                   bool       onlySet)
    {
      bool Stated(string name)
        => !onlySet || options.IsSet(name);

      // The managed transport ignores every TLS option of an `http://` endpoint, and the engine
      // refuses them there. An empty endpoint is the engine's own, whose scheme is not known here.
      var clear = options.Endpoint is not null && options.Endpoint.StartsWith("http://",
                                                                              StringComparison.OrdinalIgnoreCase);
      var tls = new TlsOptions();

      // Stated either way, even when it says that nothing is set: the system's roots and no
      // client certificate are values, which turn off what the sources below set.
      if (!clear && (Stated(nameof(GrpcClient.AllowUnsafeConnection)) || Stated(nameof(GrpcClient.CaCert))))
      {
        if (options.AllowUnsafeConnection)
        {
          tls.ServerCertificates = new ServerCertificates.None();
        }
        else if (!string.IsNullOrWhiteSpace(options.CaCert))
        {
          tls.ServerCertificates = new ServerCertificates.CaPem(options.CaCert);
        }
        else
        {
          tls.ServerCertificates = new ServerCertificates.System();
        }
      }

      if (!clear && (Stated(nameof(GrpcClient.CertP12)) || Stated(nameof(GrpcClient.CertPem)) || Stated(nameof(GrpcClient.KeyPem))))
      {
        if (!string.IsNullOrWhiteSpace(options.CertP12))
        {
          tls.ClientCertificate = new ClientCertificate.P12(options.CertP12);
        }
        else if (!string.IsNullOrWhiteSpace(options.CertPem) && !string.IsNullOrWhiteSpace(options.KeyPem))
        {
          tls.ClientCertificate = new ClientCertificate.Pem(options.CertPem,
                                                            options.KeyPem);
        }
        else
        {
          tls.ClientCertificate = new ClientCertificate.None();
        }
      }

      var transport = new TransportOptions
                      {
                        Tls = IsEmpty(tls)
                                ? null
                                : tls,
                      };

      // The credentials go with an address, as the managed transport has them, so they alone say nothing.
      if (Stated(nameof(GrpcClient.Proxy)))
      {
        transport.Proxy = Proxy(options);
      }

      // A span that is not positive, the infinite one included, is no keepalive: set, it turns off
      // what the sources below set, as left out it would leave it. A probe is its idle time and
      // what goes with it, so it is stated whole, from the time and the interval of the options
      // and not from one alone. The keepalive counts whole seconds, rounded up so that a positive
      // span is never zero.
      if (Stated(nameof(GrpcClient.KeepAliveTime)) || Stated(nameof(GrpcClient.KeepAliveTimeInterval)))
      {
        // The interval is sent as it is set, whatever it is, so that the engine refuses one it
        // cannot honour when the channel is created, naming the key.
        transport.TcpKeepalive = Positive(options.KeepAliveTime)
                                   ? new TcpKeepalive.Probe(WholeSeconds(options.KeepAliveTime))
                                     {
                                       IntervalSeconds = Stated(nameof(GrpcClient.KeepAliveTimeInterval))
                                                           ? WholeSeconds(options.KeepAliveTimeInterval)
                                                           : null,
                                     }
                                   : new TcpKeepalive.None();
      }

      var http2 = new Http2Options();
      if (Stated(nameof(GrpcClient.MaxIdleTime)))
      {
        http2.IdleTimeout = Positive(options.MaxIdleTime)
                              ? new Http2IdleTimeout.After(options.MaxIdleTime.TotalSeconds)
                              : new Http2IdleTimeout.None();
      }

      // GrpcClient retries UNAVAILABLE, ABORTED and UNKNOWN, and has no option for the statuses. The engine's
      // own default has UNAVAILABLE alone of the statuses, so the defaults of GrpcClient state the three, which
      // the sources below may still replace; a channel states none, since its options cannot differ here. The
      // proxy statuses, the refused stream, Dial and Connection are failures that no server answered with a
      // status, which grpc-dotnet retries as UNAVAILABLE.
      IReadOnlyList<string>? failures = onlySet
                                          ? null
                                          : GrpcClientFailures;
      int?        maxAttempts           = null;
      double?     initialBackoffSeconds = null;
      double?     maxBackoffSeconds     = null;
      double?     backoffMultiplier     = null;

      if (Stated(nameof(GrpcClient.MaxAttempts)))
      {
        maxAttempts = options.MaxAttempts;
      }

      // Only the bounds that are set: the engine checks the pair once the options are merged, and
      // refuses an initial backoff above the maximum then, naming both keys.
      if (Stated(nameof(GrpcClient.InitialBackOff)))
      {
        initialBackoffSeconds = options.InitialBackOff.TotalSeconds;
      }

      if (Stated(nameof(GrpcClient.MaxBackOff)))
      {
        maxBackoffSeconds = options.MaxBackOff.TotalSeconds;
      }

      if (Stated(nameof(GrpcClient.BackoffMultiplier)))
      {
        backoffMultiplier = options.BackoffMultiplier;
      }

      // One attempt is no retry, which the engine has an option of its own for: its `ExponentialBackoff` needs
      // two at least, and refuses a count below one. What else is stated of the retries goes with it, there
      // being none.
      RetryOptions? retry = maxAttempts is 1
                              ? new RetryOptions.None()
                              : failures is not null || maxAttempts is not null || initialBackoffSeconds is not null || maxBackoffSeconds is not null || backoffMultiplier is not null
                                ? new RetryOptions.ExponentialBackoff
                                  {
                                    FailureList           = failures,
                                    MaxAttempts           = maxAttempts,
                                    InitialBackoffSeconds = initialBackoffSeconds,
                                    MaxBackoffSeconds     = maxBackoffSeconds,
                                    BackoffMultiplier     = backoffMultiplier,
                                  }
                                : null;

      var grpc = new GrpcOptions
                 {
                   OutboundTraffic = new OutboundTrafficOptions
                                     {
                                       Retry = retry,
                                     },
                 };
      if (Stated(nameof(GrpcClient.RequestTimeout)))
      {
        grpc.Deadline = Positive(options.RequestTimeout)
                          ? new Deadline.Default(options.RequestTimeout.TotalSeconds)
                          : new Deadline.None();
      }

      return new NativeChannelOptions
             {
               Transport = transport,
               Http2     = http2,
               Grpc      = grpc,
             };
    }

    // The proxy as the managed transport reads the three options: empty is the default configuration,
    // which is the system's, `none` and `system` are words, and anything else is an address, which the
    // credentials go with. The engine takes them as a pair and reads an empty half literally, so one
    // half set is a pair with the other empty, as the managed transport's NetworkCredential reads it.
    private static ProxyOptions Proxy(GrpcClient options)
    {
      switch (options.Proxy)
      {
        case "":
          return new ProxyOptions.System();
        case "none":
        case "None":
          return new ProxyOptions.None();
        case "system":
        case "System":
          return new ProxyOptions.System();
        default:
          return new ProxyOptions.Url(options.Proxy)
                 {
                   Credentials = string.IsNullOrEmpty(options.ProxyUsername) && string.IsNullOrEmpty(options.ProxyPassword)
                                   ? null
                                   : new ProxyCredentials(options.ProxyUsername ?? string.Empty,
                                                          options.ProxyPassword ?? string.Empty),
                 };
      }
    }

    private static bool Positive(TimeSpan span)
      => span > TimeSpan.Zero;

    // The whole seconds of a span, rounded up, and as the engine counts: a value out of its bounds is
    // the engine's to refuse.
    private static int WholeSeconds(TimeSpan span)
      => (int)Math.Min(Math.Ceiling(span.TotalSeconds),
                       int.MaxValue);

    private static bool IsEmpty(TlsOptions tls)
      => tls.ServerCertificates is null && tls.ClientCertificate is null;
  }
}
