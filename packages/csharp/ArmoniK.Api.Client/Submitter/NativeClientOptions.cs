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
      if (!clear && (Stated(nameof(GrpcClient.AllowUnsafeConnection)) || Stated(nameof(GrpcClient.CaCert))))
      {
        if (options.AllowUnsafeConnection)
        {
          tls.Server = new ServerVerification.Unverified();
        }
        else if (!string.IsNullOrWhiteSpace(options.CaCert))
        {
          tls.Server = new ServerVerification.CaPem(options.CaCert);
        }
      }

      if (!clear && (Stated(nameof(GrpcClient.CertP12)) || Stated(nameof(GrpcClient.CertPem)) || Stated(nameof(GrpcClient.KeyPem))))
      {
        if (!string.IsNullOrWhiteSpace(options.CertP12))
        {
          tls.Client = new ClientCertificate.P12(options.CertP12);
        }
        else if (!string.IsNullOrWhiteSpace(options.CertPem) && !string.IsNullOrWhiteSpace(options.KeyPem))
        {
          tls.Client = new ClientCertificate.Pem(options.CertPem,
                                                 options.KeyPem);
        }
      }

      if (!clear && Stated(nameof(GrpcClient.OverrideTargetName)) && !string.IsNullOrEmpty(options.OverrideTargetName))
      {
        tls.OverrideTargetName = options.OverrideTargetName;
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

      // A span that is not positive, the infinite one included, is none, which the engine reads as
      // zero: stated, it turns off what the sources below set, as left out it would leave it.
      var keepalive = new TcpKeepaliveOptions();
      if (Stated(nameof(GrpcClient.KeepAliveTime)))
      {
        keepalive.IdleSeconds = Seconds(options.KeepAliveTime);
      }

      if (Stated(nameof(GrpcClient.KeepAliveTimeInterval)) && Positive(options.KeepAliveTimeInterval))
      {
        keepalive.IntervalSeconds = options.KeepAliveTimeInterval.TotalSeconds;
      }

      if (keepalive.IdleSeconds is not null || keepalive.IntervalSeconds is not null)
      {
        transport.TcpKeepalive = keepalive;
      }

      var http2 = new Http2Options();
      if (Stated(nameof(GrpcClient.MaxIdleTime)))
      {
        http2.IdleTimeoutSeconds = Seconds(options.MaxIdleTime);
      }

      var retry = new RetryOptions();

      // GrpcClient retries UNAVAILABLE, ABORTED and UNKNOWN, and has no option for the codes. The engine's
      // own default is UNAVAILABLE alone, so the defaults of GrpcClient state the preset, which the
      // sources below may still replace; a channel states none, since its options cannot differ here.
      if (!onlySet)
      {
        retry.Codes = new RetryCodes.GrpcClient();
      }

      if (Stated(nameof(GrpcClient.MaxAttempts)))
      {
        retry.MaxAttempts = options.MaxAttempts;
      }

      // Each bound when it is stated. The engine refuses a maximum below the initial one, though,
      // so the bound that is not stated is sent too when the other would pass it: grpc-dotnet draws
      // its first delay up to the initial backoff and caps the later ones at the maximum, so a
      // maximum below the initial one is raised to it, and an initial one above a stated maximum is
      // lowered to it.
      var initial = options.InitialBackOff.TotalSeconds;
      var maximum = options.MaxBackOff.TotalSeconds;
      var initialStated = Stated(nameof(GrpcClient.InitialBackOff));
      var maximumStated = Stated(nameof(GrpcClient.MaxBackOff));
      if (initialStated)
      {
        retry.InitialBackoffSeconds = initial;
      }

      if (maximumStated)
      {
        retry.MaxBackoffSeconds = initialStated
                                    ? Math.Max(initial,
                                               maximum)
                                    : maximum;
      }

      if (initial > maximum)
      {
        if (initialStated && !maximumStated)
        {
          retry.MaxBackoffSeconds = initial;
        }
        else if (maximumStated && !initialStated)
        {
          retry.InitialBackoffSeconds = maximum;
        }
      }

      if (Stated(nameof(GrpcClient.BackoffMultiplier)))
      {
        retry.BackoffMultiplier = options.BackoffMultiplier;
      }

      var grpc = new GrpcOptions
                 {
                   Retry = retry,
                 };
      if (Stated(nameof(GrpcClient.RequestTimeout)))
      {
        grpc.DefaultDeadlineSeconds = Seconds(options.RequestTimeout);
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
    // credentials go with.
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
          return new ProxyOptions.Url(options.Proxy,
                                      string.IsNullOrEmpty(options.ProxyUsername)
                                        ? null
                                        : options.ProxyUsername,
                                      string.IsNullOrEmpty(options.ProxyPassword)
                                        ? null
                                        : options.ProxyPassword);
      }
    }

    private static bool Positive(TimeSpan span)
      => span > TimeSpan.Zero;

    // The seconds of a span, zero for one that is not positive: none.
    private static double Seconds(TimeSpan span)
      => Positive(span)
           ? span.TotalSeconds
           : 0;

    private static bool IsEmpty(TlsOptions tls)
      => tls.Server is null && tls.Client is null && tls.OverrideTargetName is null;
  }
}
