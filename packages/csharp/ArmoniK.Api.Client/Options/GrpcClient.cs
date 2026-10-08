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

using ArmoniK.Api.Client.Submitter;
using ArmoniK.Utils.DocAttribute;

using JetBrains.Annotations;

namespace ArmoniK.Api.Client.Options
{
  /// <summary>
  ///   Options for creating a gRPC Client with <see cref="GrpcChannelFactory" />
  /// </summary>
  /// <remarks>
  ///   The native transport knows which of the options below were assigned, or bound from a key of a
  ///   configuration, even to their defaults, and leaves the others to the engine's own sources.
  ///   Assigning an option the value it was just read with is not recorded, as the configuration binder does that
  ///   for a key that is absent; the binder 6.0 does it for every key, so a key present at the default is not
  ///   recorded there. Nothing here is synchronised.
  /// </remarks>
  [ExtractDocumentation("Options for GrpcClient")]
  [PublicAPI]
  public class GrpcClient
  {
    /// <summary>
    ///   Path to the section containing the values in the configuration object
    /// </summary>
    public const string SettingSection = nameof(GrpcClient);

    // The names of the options assigned, which the native transport sends over the engine's other sources.
    private readonly HashSet<string> set_ = new();

    // The option read last: the configuration binder assigns an absent key what it read, which no caller chose.
    private string? lastRead_;

    /// <summary>
    ///   Whether the caller set an option, by assigning it or by a key of the configuration it was bound from
    /// </summary>
    /// <param name="name">The name of the option</param>
    /// <returns>True when it was set, even to its default</returns>
    internal bool IsSet(string name)
      => set_.Contains(name);

    private T Read<T>(T      value,
                      string name)
    {
      lastRead_ = name;
      return value;
    }

    private void Write<T>(ref T    field,
                          T      value,
                          string name)
    {
      var echo = lastRead_ == name && EqualityComparer<T>.Default.Equals(field,
                                                                         value);
      lastRead_ = null;
      field     = value;
      if (!echo)
      {
        set_.Add(name);
      }
    }

    /// <summary>
    ///   Endpoint for sending requests
    /// </summary>
    public string? Endpoint { get; set; }

    private bool allowUnsafeConnection_;

    /// <summary>
    ///   Allow unsafe connections to the endpoint (without SSL), defaults to false
    /// </summary>
    public bool AllowUnsafeConnection
    {
      get => Read(allowUnsafeConnection_,
                  nameof(AllowUnsafeConnection));
      set => Write(ref allowUnsafeConnection_,
                   value,
                   nameof(AllowUnsafeConnection));
    }

    private string certPem_ = "";

    /// <summary>
    ///   Path to the certificate file in pem format
    /// </summary>
    public string CertPem
    {
      get => Read(certPem_,
                  nameof(CertPem));
      set => Write(ref certPem_,
                   value,
                   nameof(CertPem));
    }

    private string keyPem_ = "";

    /// <summary>
    ///   Path to the key file in pem format
    /// </summary>
    public string KeyPem
    {
      get => Read(keyPem_,
                  nameof(KeyPem));
      set => Write(ref keyPem_,
                   value,
                   nameof(KeyPem));
    }

    private string certP12_ = "";

    /// <summary>
    ///   Path to the certificate file in p12/pfx format
    /// </summary>
    public string CertP12
    {
      get => Read(certP12_,
                  nameof(CertP12));
      set => Write(ref certP12_,
                   value,
                   nameof(CertP12));
    }

    private string caCert_ = "";

    /// <summary>
    ///   Path to the Certificate Authority file in pem format
    /// </summary>
    public string CaCert
    {
      get => Read(caCert_,
                  nameof(CaCert));
      set => Write(ref caCert_,
                   value,
                   nameof(CaCert));
    }

    private string overrideTargetName_ = "";

    /// <summary>
    ///   Override the endpoint name during SSL verification. This option is only used when AllowUnsafeConnection is true and
    ///   only when the runtime is .NET Framework; the native transport applies it whenever it is set.
    ///   Automatic target name by default. Should be overriden by the right name to reduce performance cost.
    /// </summary>
    public string OverrideTargetName
    {
      get => Read(overrideTargetName_,
                  nameof(OverrideTargetName));
      set => Write(ref overrideTargetName_,
                   value,
                   nameof(OverrideTargetName));
    }


    /// <summary>
    ///   True if the options specify a client certificate
    /// </summary>
    public bool HasClientCertificate
      => !string.IsNullOrWhiteSpace(CertP12) || !(string.IsNullOrWhiteSpace(CertPem) || string.IsNullOrWhiteSpace(KeyPem));

    private TimeSpan keepAliveTime_ = TimeSpan.FromSeconds(30);

    /// <summary>
    ///   KeepAliveTime is the time after which the connection will be kept alive.
    ///   The native transport reads a value that is zero, negative or infinite as no keepalive.
    /// </summary>
    public TimeSpan KeepAliveTime
    {
      get => Read(keepAliveTime_,
                  nameof(KeepAliveTime));
      set => Write(ref keepAliveTime_,
                   value,
                   nameof(KeepAliveTime));
    }

    private TimeSpan keepAliveTimeInterval_ = TimeSpan.FromSeconds(30);

    /// <summary>
    ///   KeepAliveTimeInterval is the interval at which the connection will be kept alive.
    ///   The native transport refuses a value that is not positive, when the channel is created, unless
    ///   KeepAliveTime is not positive either, which turns the keepalive off.
    /// </summary>
    public TimeSpan KeepAliveTimeInterval
    {
      get => Read(keepAliveTimeInterval_,
                  nameof(KeepAliveTimeInterval));
      set => Write(ref keepAliveTimeInterval_,
                   value,
                   nameof(KeepAliveTimeInterval));
    }

    private TimeSpan maxIdleTime_ = TimeSpan.FromMinutes(5);

    /// <summary>
    ///   MaxIdleTime is the maximum idle time after which the connection will be closed.
    ///   The native transport reads a value that is zero, negative or infinite as no limit.
    /// </summary>
    public TimeSpan MaxIdleTime
    {
      get => Read(maxIdleTime_,
                  nameof(MaxIdleTime));
      set => Write(ref maxIdleTime_,
                   value,
                   nameof(MaxIdleTime));
    }

    private int maxAttempts_ = 5;

    /// <summary>
    ///   MaxAttempts is a property that gets and sets the maximum number of attempts to retry an operation.
    /// </summary>
    public int MaxAttempts
    {
      get => Read(maxAttempts_,
                  nameof(MaxAttempts));
      set => Write(ref maxAttempts_,
                   value,
                   nameof(MaxAttempts));
    }

    private double backoffMultiplier_ = 1.5;

    /// <summary>
    ///   The backoff will be multiplied by this multiplier after each retry attempt and will increase exponentially when the
    ///   multiplier is greater than 1.
    /// </summary>
    public double BackoffMultiplier
    {
      get => Read(backoffMultiplier_,
                  nameof(BackoffMultiplier));
      set => Write(ref backoffMultiplier_,
                   value,
                   nameof(BackoffMultiplier));
    }

    private TimeSpan initialBackOff_ = TimeSpan.FromSeconds(1);

    /// <summary>
    ///   InitialBackOff is a property that gets and sets the initial backOff time for retrying an operation.
    /// </summary>
    public TimeSpan InitialBackOff
    {
      get => Read(initialBackOff_,
                  nameof(InitialBackOff));
      set => Write(ref initialBackOff_,
                   value,
                   nameof(InitialBackOff));
    }

    private TimeSpan maxBackOff_ = TimeSpan.FromSeconds(5);

    /// <summary>
    ///   MaxBackOff is a property that gets and sets the maximum backOff time for retrying an operation.
    /// </summary>
    public TimeSpan MaxBackOff
    {
      get => Read(maxBackOff_,
                  nameof(MaxBackOff));
      set => Write(ref maxBackOff_,
                   value,
                   nameof(MaxBackOff));
    }

    private TimeSpan requestTimeout_ = Timeout.InfiniteTimeSpan;

    /// <summary>
    ///   Timeout for grpc requests. Defaults to no timeout.
    ///   The native transport reads a value that is zero, negative or infinite as no timeout.
    /// </summary>
    public TimeSpan RequestTimeout
    {
      get => Read(requestTimeout_,
                  nameof(RequestTimeout));
      set => Write(ref requestTimeout_,
                   value,
                   nameof(RequestTimeout));
    }

    /// <summary>
    ///   Which transport carries the calls: `Managed`, grpc-dotnet (the default), or `Native`, the
    ///   Rust engine. Only GrpcChannelFactory.CreateChannelBase honours it: CreateChannel makes a
    ///   grpc-dotnet channel whatever it says, with a warning when it is given a logger.
    ///   With `Native`, the other options of the engine are read from the environment under
    ///   `ArmoniK__Client__Grpc__` and from the command line.
    ///   The other options are sent when they are set, even to their defaults, over those sources, except
    ///   `HttpMessageHandler` and `ReusePorts`, which are not read.
    ///   An empty or false value of the TLS verification, the certificates and the target name sends nothing:
    ///   they have no neutral value. See NativeChannelFactory.
    /// </summary>
    public ClientTransport Transport { get; set; } = ClientTransport.Managed;

    /// <summary>
    ///   Which HttpMessageHandler to use.
    ///   Valid options:
    ///   - `HttpClientHandler`
    ///   - `WinHttpHandler`
    ///   - `GrpcWebHandler`
    ///   If the handler is not set, the best one will be used.
    /// </summary>
    public string HttpMessageHandler { get; set; } = "";

    private string proxy_ = "";

    /// <summary>
    ///   Proxy configuration.
    ///   If empty, the default proxy configuration is used.
    ///   If "none", proxy is disabled.
    ///   If "system", the system proxy is used
    ///   Otherwise, it is the URL of the proxy to use
    /// </summary>
    public string Proxy
    {
      get => Read(proxy_,
                  nameof(Proxy));
      set => Write(ref proxy_,
                   value,
                   nameof(Proxy));
    }

    private string proxyUsername_ = "";

    /// <summary>
    ///   Username used for proxy authentication
    /// </summary>
    public string ProxyUsername
    {
      get => Read(proxyUsername_,
                  nameof(ProxyUsername));
      set => Write(ref proxyUsername_,
                   value,
                   nameof(ProxyUsername));
    }

    private string proxyPassword_ = "";

    /// <summary>
    ///   Password used for proxy authentication
    /// </summary>
    public string ProxyPassword
    {
      get => Read(proxyPassword_,
                  nameof(ProxyPassword));
      set => Write(ref proxyPassword_,
                   value,
                   nameof(ProxyPassword));
    }

    /// <summary>
    ///   Enable the option SO_REUSE_UNICASTPORT upon socket opening to limit port exhaustion
    /// </summary>
    public bool ReusePorts { get; set; } = true;
  }
}
