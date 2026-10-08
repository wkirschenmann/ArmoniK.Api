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
      // zero: set, it turns off what the sources below set, as left out it would leave it. The
      // keepalive counts whole seconds, rounded up so that a positive span is never zero.
      var keepalive = new TcpKeepaliveOptions();
      if (Stated(nameof(GrpcClient.KeepAliveTime)))
      {
        keepalive.IdleSeconds = Positive(options.KeepAliveTime)
                                  ? WholeSeconds(options.KeepAliveTime)
                                  : 0;
      }

      // The interval is sent as it is set, whatever it is, so that the engine refuses one it cannot
      // honour when the channel is created, naming the key. A keepalive that is off reads none of
      // it, and an interval that is not positive beside it is what a caller turning it off writes.
      if (Stated(nameof(GrpcClient.KeepAliveTimeInterval)) && (Positive(options.KeepAliveTimeInterval) || Positive(options.KeepAliveTime)))
      {
        keepalive.IntervalSeconds = WholeSeconds(options.KeepAliveTimeInterval);
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

      // Only the bounds that are set: the engine checks the pair once the options are merged, and
      // refuses an initial backoff above the maximum then, naming both keys.
      if (Stated(nameof(GrpcClient.InitialBackOff)))
      {
        retry.InitialBackoffSeconds = options.InitialBackOff.TotalSeconds;
      }

      if (Stated(nameof(GrpcClient.MaxBackOff)))
      {
        retry.MaxBackoffSeconds = options.MaxBackOff.TotalSeconds;
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

    // The whole seconds of a span, rounded up, and as the engine counts: a value out of its bounds is
    // the engine's to refuse.
    private static int WholeSeconds(TimeSpan span)
      => (int)Math.Min(Math.Ceiling(span.TotalSeconds),
                       int.MaxValue);

    // The seconds of a span, zero for one that is not positive: none.
    private static double Seconds(TimeSpan span)
      => Positive(span)
           ? span.TotalSeconds
           : 0;

    private static bool IsEmpty(TlsOptions tls)
      => tls.Server is null && tls.Client is null && tls.OverrideTargetName is null;
  }
}
