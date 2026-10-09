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
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Client.RustGrpcChannel;

using Grpc.Core;

using JetBrains.Annotations;

using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.Submitter
{
  /// <summary>
  ///   Opens the channels of the native transport, on the one engine a process may run
  /// </summary>
  /// <remarks>
  ///   <para>
  ///     The engine, a <see cref="NativeRuntime" />, starts with the first channel and lives until
  ///     <see cref="ShutdownAsync" />. Its options come from three sources, a later one over an earlier one:
  ///     the defaults of <see cref="GrpcClient" />, the environment under <c>ArmoniK__Client__Grpc__</c>, and
  ///     the command line under <c>--ArmoniK:Client:Grpc:</c>. Those are read once, with the first channel.
  ///   </para>
  ///   <para>
  ///     Each channel states the options of its own <see cref="GrpcClient" /> that the caller set, even to their
  ///     defaults, and they win over the engine's sources: a certificate or a proxy differs from one channel to the
  ///     next. The TLS verification and the client certificate state the system's roots and no certificate when
  ///     they are set to nothing. <see cref="GrpcClient.OverrideTargetName" /> is not sent, and logs a warning.
  ///     The engine refuses a value outside its bounds, such as a keepalive interval of zero, when the channel is
  ///     opened.
  ///   </para>
  ///   <para>
  ///     The engine's own logs go to the <see cref="ILoggerFactory" /> of the call that starts it. The engine's
  ///     filter, <c>ArmoniK__Client__Grpc__Logging__Filter</c> in the environment, selects what reaches it.
  ///   </para>
  /// </remarks>
  [PublicAPI]
  public sealed class NativeChannelFactory
  {
    private readonly object gate_ = new();

    private NativeRuntime? runtime_;

    // The disposal of the last engine: a process holds one runtime at a time, so no new one starts
    // before it ends.
    private Task? shutdown_;

    private NativeChannelFactory()
    {
    }

    /// <summary>
    ///   The factory of the process
    /// </summary>
    public static NativeChannelFactory Instance { get; } = new();

    /// <summary>
    ///   Opens a channel to the endpoint of <paramref name="options" />, starting the engine if it is not running
    /// </summary>
    /// <param name="options">Options for the creation of the channel</param>
    /// <param name="commandLine">
    ///   The command line the engine reads its options from, when this call starts it; the process's own when null.
    ///   Ignored, and logged, when the engine is already running.
    /// </param>
    /// <param name="logger">Optional logger</param>
    /// <param name="loggerFactory">
    ///   Where the engine's own logs go, when this call starts it; ignored when it is running. The factory has to
    ///   outlive the engine, which writes to it until <see cref="ShutdownAsync" /> ends.
    /// </param>
    /// <returns>A <see cref="NativeChannel" />, which the caller disposes</returns>
    /// <exception cref="ArgumentNullException"><paramref name="options" /> is null</exception>
    /// <exception cref="ArgumentOutOfRangeException">An option is outside the bounds the engine admits</exception>
    /// <exception cref="InvalidOperationException">
    ///   The engine could not start: another runtime lives in this process, or a source is refused; or it is shutting
    ///   down; or <see cref="GrpcClient.NativeMetrics" /> asks for the build with the counters in a process that has
    ///   loaded the other
    /// </exception>
    /// <exception cref="RustEngineMissingException">The engine could not be loaded</exception>
    /// <remarks>
    ///   An empty <see cref="GrpcClient.Endpoint" /> is the <c>Endpoint</c> of the engine's options.
    /// </remarks>
    public ChannelBase CreateChannel(GrpcClient      options,
                                     string[]?       commandLine   = null,
                                     ILogger?        logger        = null,
                                     ILoggerFactory? loggerFactory = null)
    {
      if (options is null)
      {
        throw new ArgumentNullException(nameof(options));
      }

      if (!string.IsNullOrEmpty(options.HttpMessageHandler))
      {
        logger?.LogWarning("HttpMessageHandler is not read by the native transport");
      }

      if (!string.IsNullOrEmpty(options.OverrideTargetName))
      {
        logger?.LogWarning("OverrideTargetName is not read by the native transport: the certificate is verified against the host of the endpoint, so give another endpoint for another name");
      }

      var translated = NativeClientOptions.Translate(options,
                                                     true);

      // Opened under the lock, so that a shutdown cannot dispose the engine between the read and the open.
      lock (gate_)
      {
        if (shutdown_ is
            {
              IsCompleted: false,
            })
        {
          throw new InvalidOperationException("the native engine is shutting down and opens no channel until it has stopped");
        }

        if (runtime_ is null)
        {
          runtime_ = Start(commandLine,
                           logger,
                           loggerFactory,
                           options.NativeMetrics);
        }
        else
        {
          if (commandLine is not null)
          {
            logger?.LogWarning("The command line is read when the native engine starts, and it is running");
          }

          if (options.NativeMetrics && NativeLibrarySelection.Loaded != NativeEngineBuild.Metrics)
          {
            logger?.LogWarning("NativeMetrics is read when the native engine starts, and it is running without the counters");
          }
        }

        return runtime_.Channel(options.Endpoint ?? string.Empty,
                                translated);
      }
    }

    /// <summary>
    ///   Disposes the channels this factory opened and stops the engine, which the next channel starts again
    /// </summary>
    /// <returns>A task that ends once the engine has stopped</returns>
    public Task ShutdownAsync()
    {
      lock (gate_)
      {
        var stopping = runtime_;
        runtime_ = null;
        if (stopping is not null)
        {
          shutdown_ = stopping.DisposeAsync()
                              .AsTask();
          return shutdown_;
        }

        // A shutdown already under way is the one to wait for; one that has ended is no news.
        return shutdown_ is
               {
                 IsCompleted: false,
               }
                 ? shutdown_
                 : Task.CompletedTask;
      }
    }

    // The defaults of GrpcClient are the floor, so that an option no source states is what the managed
    // transport would use.
    private static NativeRuntime Start(string[]?       commandLine,
                                       ILogger?        logger,
                                       ILoggerFactory? loggerFactory,
                                       bool            nativeMetrics)
    {
      var configuration = new NativeConfiguration().LoadConfigFromObject(new RuntimeOptions
                                                                         {
                                                                           ChannelDefaults = NativeClientOptions.Translate(new GrpcClient(),
                                                                                                                           false),
                                                                         })
                                                   .LoadConfigFromEnvironment();
      if (commandLine is not null)
      {
        configuration.LoadConfigFromCommandLine(commandLine);
      }
      else
      {
        try
        {
          configuration.LoadConfigFromCommandLine(Environment.GetCommandLineArgs()
                                                             .Skip(1)
                                                             .ToArray());
        }
        catch (FormatException error)
        {
          // The process's own arguments may be in a syntax the provider refuses, such as `-x=1`.
          logger?.LogWarning(error,
                             "The command line of the process is not read for the native transport");
        }
      }

      // Last, so that a configuration that is refused leaves the choice of the build to whoever comes next.
      if (nativeMetrics)
      {
        NativeLibrarySelection.Select(NativeEngineBuild.Metrics);
      }

      return NativeRuntime.Create(configuration,
                                  loggerFactory);
    }
  }
}
