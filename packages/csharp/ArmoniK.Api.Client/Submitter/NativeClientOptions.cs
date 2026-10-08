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
    ///   The failures <see cref="GrpcClient" /> retries: the three statuses of its default, and the connections that
    ///   could not be made or ended under the call. The statuses ABORTED and UNKNOWN count as acceptances for the
    ///   engine's throttle.
    /// </summary>
    private static readonly IReadOnlyList<string> GrpcClientFailures = new[]
                                                                       {
                                                                         "Status.UNAVAILABLE",
                                                                         "Status.ABORTED",
                                                                         "Status.UNKNOWN",
                                                                         "Dial",
                                                                         "Connection",
                                                                       };

    /// <summary>
    ///   The channel options the engine reads for <paramref name="options" />
    /// </summary>
    /// <param name="options">The options of the client</param>
    /// <param name="floor">
    ///   When given, only what <paramref name="options" /> states beyond it is translated: an option equal to
    ///   <paramref name="floor" />'s is left to the sources below, so that it does not override them
    /// </param>
    /// <returns>The channel options; a group of options with nothing stated is left null</returns>
    internal static NativeChannelOptions Translate(GrpcClient  options,
                                                   GrpcClient? floor)
    {
      bool Stated<T>(Func<GrpcClient, T> read)
        => floor is null || !EqualityComparer<T>.Default.Equals(read(options),
                                                                read(floor));

      // The managed transport ignores every TLS option of an `http://` endpoint, and the engine
      // refuses them there. An empty endpoint is the engine's own, whose scheme is not known here.
      var clear = options.Endpoint is not null && options.Endpoint.StartsWith("http://",
                                                                              StringComparison.OrdinalIgnoreCase);
      var tls = new TlsOptions();
      if (!clear && (Stated(o => o.AllowUnsafeConnection) || Stated(o => o.CaCert)))
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

      if (!clear && (Stated(o => o.CertP12) || Stated(o => o.CertPem) || Stated(o => o.KeyPem)))
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

      if (!clear && Stated(o => o.OverrideTargetName) && !string.IsNullOrEmpty(options.OverrideTargetName))
      {
        tls.OverrideTargetName = options.OverrideTargetName;
      }

      var transport = new TransportOptions
                      {
                        Tls = IsEmpty(tls)
                                ? null
                                : tls,
                      };

      if (Stated(o => o.Proxy) || Stated(o => o.ProxyUsername) || Stated(o => o.ProxyPassword))
      {
        transport.Proxy = Proxy(options);
      }

      var keepalive = new TcpKeepaliveOptions();
      if (Stated(o => o.KeepAliveTime) && Positive(options.KeepAliveTime))
      {
        keepalive.IdleSeconds = options.KeepAliveTime.TotalSeconds;
      }

      if (Stated(o => o.KeepAliveTimeInterval) && Positive(options.KeepAliveTimeInterval))
      {
        keepalive.IntervalSeconds = options.KeepAliveTimeInterval.TotalSeconds;
      }

      if (keepalive.IdleSeconds is not null || keepalive.IntervalSeconds is not null)
      {
        transport.TcpKeepalive = keepalive;
      }

      var http2 = new Http2Options();
      if (Stated(o => o.MaxIdleTime) && Positive(options.MaxIdleTime))
      {
        http2.IdleTimeoutSeconds = options.MaxIdleTime.TotalSeconds;
      }

      // GrpcClient retries UNAVAILABLE, ABORTED and UNKNOWN, and has no option for the statuses. The engine's
      // own default is UNAVAILABLE alone, so the defaults of GrpcClient state the three, which the sources
      // below may still replace; a channel states none, since its options cannot differ here. Dial and
      // Connection are the failures that no server answered, which grpc-dotnet retries as UNAVAILABLE.
      IReadOnlyList<string>? failures = floor is null
                                          ? GrpcClientFailures
                                          : null;
      int?        maxAttempts           = null;
      double?     initialBackoffSeconds = null;
      double?     maxBackoffSeconds     = null;
      double?     backoffMultiplier     = null;

      if (Stated(o => o.MaxAttempts))
      {
        maxAttempts = options.MaxAttempts;
      }

      // Each bound when it is stated. The engine refuses a maximum below the initial one, though,
      // so the bound that is not stated is sent too when the other would pass it: grpc-dotnet draws
      // its first delay up to the initial backoff and caps the later ones at the maximum, so a
      // maximum below the initial one is raised to it, and an initial one above a stated maximum is
      // lowered to it.
      var initial = options.InitialBackOff.TotalSeconds;
      var maximum = options.MaxBackOff.TotalSeconds;
      var initialStated = Stated(o => o.InitialBackOff);
      var maximumStated = Stated(o => o.MaxBackOff);
      if (initialStated)
      {
        initialBackoffSeconds = initial;
      }

      if (maximumStated)
      {
        maxBackoffSeconds = initialStated
                              ? Math.Max(initial,
                                         maximum)
                              : maximum;
      }

      if (initial > maximum)
      {
        if (initialStated && !maximumStated)
        {
          maxBackoffSeconds = initial;
        }
        else if (maximumStated && !initialStated)
        {
          initialBackoffSeconds = maximum;
        }
      }

      if (Stated(o => o.BackoffMultiplier))
      {
        backoffMultiplier = options.BackoffMultiplier;
      }

      // One attempt is no retry, which the engine has an option of its own for: its `ExponentialBackoff` needs
      // two at least, and refuses a count below one. What else is stated of the retries goes with it, there
      // being none.
      RetryOptions? retry = maxAttempts is 1
                              ? new RetryOptions.None()
                              : failures is not null || maxAttempts is not null || initialBackoffSeconds is not null || maxBackoffSeconds is not null || backoffMultiplier is not null
                                ? new RetryOptions.ExponentialBackoff(failures,
                                                                      maxAttempts,
                                                                      initialBackoffSeconds,
                                                                      maxBackoffSeconds,
                                                                      backoffMultiplier)
                                : null;

      var grpc = new GrpcOptions
                 {
                   OutboundTraffic = new OutboundTrafficOptions
                                     {
                                       Retry = retry,
                                     },
                 };
      if (Stated(o => o.RequestTimeout) && Positive(options.RequestTimeout))
      {
        grpc.DefaultDeadlineSeconds = options.RequestTimeout.TotalSeconds;
      }

      return new NativeChannelOptions
             {
               Transport = transport,
               Http2     = http2,
               Grpc      = grpc,
             };
    }

    /// <summary>
    ///   The options that ask for no bound where the engine's sources set one, which the engine states by saying nothing
    /// </summary>
    /// <param name="options">The options of the client</param>
    /// <returns>The names of the options that cannot turn off what the defaults of <see cref="GrpcClient" /> set</returns>
    internal static IEnumerable<string> CannotBeDisabled(GrpcClient options)
    {
      if (!Positive(options.KeepAliveTime))
      {
        yield return nameof(GrpcClient.KeepAliveTime);
      }

      if (!Positive(options.KeepAliveTimeInterval))
      {
        yield return nameof(GrpcClient.KeepAliveTimeInterval);
      }

      if (!Positive(options.MaxIdleTime))
      {
        yield return nameof(GrpcClient.MaxIdleTime);
      }
    }

    // The proxy as the managed transport reads the three options: empty is the default, `none` and
    // `system` are words, and anything else is an address, which the credentials go with.
    private static ProxyOptions? Proxy(GrpcClient options)
    {
      switch (options.Proxy)
      {
        case "":
          return null;
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

    // A span that is not positive, the infinite one included, is no bound, which the engine states by
    // saying nothing.
    private static bool Positive(TimeSpan span)
      => span > TimeSpan.Zero;

    private static bool IsEmpty(TlsOptions tls)
      => tls.Server is null && tls.Client is null && tls.OverrideTargetName is null;
  }
}
