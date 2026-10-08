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
using System.Linq;
using System.Reflection;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What this channel's options are against the ones the existing client carries.</summary>
/// <remarks>
///   <para>
///     Two vocabularies describe one connection: `GrpcClient`, which the repository's own client
///     reads today, and `ChannelOptions`, which the engine reads. The second is a superset by
///     design - it carries options no grpc-dotnet channel has - so the interesting question is
///     never "are they equal" but "which of the first has no counterpart yet, and which of the
///     second is this engine's own".
///   </para>
///   <para>
///     Answered by a declared correspondence rather than by hand: every option of either side has
///     to appear here, so an option added to `GrpcClient` or to the schema and forgotten fails
///     this instead of quietly having no counterpart. A name leaves <see cref="Awaited" /> for
///     <see cref="Counterparts" /> when an option here answers it.
///   </para>
/// </remarks>
[TestFixture]
public class OptionVocabularyTests
{
  /// <summary>A `GrpcClient` option and the path of the option that answers it here.</summary>
  /// <remarks>
  ///   `CaCert` is a path on both sides, the `CaPem` alternative here. `AllowUnsafeConnection` is the
  ///   `Unverified` alternative, and `Proxy` with its credentials the `Url` one: what excludes another
  ///   is an alternative here, where `GrpcClient` has options that refuse or ignore one another.
  ///   `KeepAliveTime` and `KeepAliveTimeInterval` are a socket's keepalive in `GrpcClient`, which
  ///   sets them through `ServicePoint.SetTcpKeepAlive`.
  /// </remarks>
  private static readonly IReadOnlyDictionary<string, string> Counterparts = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                             {
                                                                               ["AllowUnsafeConnection"] = "Transport.Tls.Server.Unverified",
                                                                               ["CaCert"]                = "Transport.Tls.Server.CaPem",
                                                                               ["CertPem"]               = "Transport.Tls.Client.Pem.Certificate",
                                                                               ["CertP12"]               = "Transport.Tls.Client.P12.Path",
                                                                               ["KeyPem"]                = "Transport.Tls.Client.Pem.Key",
                                                                               ["OverrideTargetName"]    = "Transport.Tls.OverrideTargetName",
                                                                               ["KeepAliveTime"]         = "Transport.TcpKeepalive.IdleSeconds",
                                                                               ["KeepAliveTimeInterval"] = "Transport.TcpKeepalive.IntervalSeconds",
                                                                               ["Proxy"]                 = "Transport.Proxy.Url.Address",
                                                                               ["ProxyUsername"]         = "Transport.Proxy.Url.Username",
                                                                               ["ProxyPassword"]         = "Transport.Proxy.Url.Password",
                                                                               ["RequestTimeout"]        = "Grpc.DefaultDeadlineSeconds",
                                                                               ["MaxIdleTime"]           = "Http2.IdleTimeoutSeconds",
                                                                               ["MaxAttempts"]           = "Grpc.Retry.MaxAttempts",
                                                                               ["InitialBackOff"]        = "Grpc.Retry.InitialBackoffSeconds",
                                                                               ["MaxBackOff"]            = "Grpc.Retry.MaxBackoffSeconds",
                                                                               ["BackoffMultiplier"]     = "Grpc.Retry.BackoffMultiplier",
                                                                             };

  /// <summary>A `GrpcClient` option this channel does not answer, and the task that carries it.</summary>
  /// <remarks>The task is named so the entry says what ends the omission.</remarks>
  private static readonly IReadOnlyDictionary<string, string> Awaited = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                        {
                                                                        };

  /// <summary>A `GrpcClient` option this engine answers with something that is not an option.</summary>
  private static readonly IReadOnlyDictionary<string, string> Elsewhere = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                          {
                                                                            ["Endpoint"] = "a channel's endpoint is `ak_channel_create`'s own argument, or the runtime's Endpoint when that is empty",
                                                                            ["HttpMessageHandler"] = "grpc-dotnet chooses a handler, and this engine is the handler",
                                                                            ["Transport"] = "chooses between grpc-dotnet and this engine, so it is no option of the engine",
                                                                            ["ReusePorts"] = "a socket option of grpc-dotnet's handler, which this engine does not use",
                                                                          };

  /// <summary>An option of this channel that `GrpcClient` has no name for.</summary>
  private static readonly IReadOnlyDictionary<string, string> Ours = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                     {
                                                                       ["Transport.ConnectEagerly"] = "grpc-dotnet connects through GrpcChannel.ConnectAsync, a call rather than an option",
                                                                       ["Grpc.Host.Receive.Window"] = "the delivery window, which only this ABI has",
                                                                       ["Grpc.Host.Receive.CoalescingBytes"] = "the delivery to the host, which only this ABI has",
                                                                       ["Grpc.Retry.Codes.GoogleRpc"] = "GrpcClient retries three statuses and has no option for them; this preset is the engine's default",
                                                                       ["Grpc.Retry.Codes.GrpcClient"] = "GrpcClient retries three statuses and has no option for them, which the translation states as this preset",
                                                                       ["Grpc.Retry.Codes.List"] = "GrpcClient retries three statuses and has no option for them",
                                                                       ["Grpc.Retry.CallReplayBytes"] = "grpc-dotnet's MaxRetryBufferPerCallSize, which GrpcClient does not set",
                                                                       ["Grpc.Retry.ChannelReplayBytes"] = "grpc-dotnet's MaxRetryBufferSize, which GrpcClient does not set",
                                                                       ["Grpc.Host.Send.Window"] = "the send window, which only this ABI has",
                                                                       ["Grpc.Send.MaxMessageSize"] = "grpc-dotnet's MaxSendMessageSize, which GrpcClient does not set",
                                                                       ["Grpc.Receive.MaxMessageSize"] = "grpc-dotnet takes this per method rather than per channel",
                                                                       ["Grpc.Send.Compression"] = "grpc-dotnet compresses a call whose metadata asks for it, and GrpcClient asks for none",
                                                                       ["Grpc.Receive.Compression"] = "grpc-dotnet's CompressionProviders, which GrpcClient does not set",
                                                                       ["Grpc.UserAgent"] = "grpc-dotnet writes its own and offers no option",
                                                                       ["Grpc.Rate.Limit.Calls"] = "grpc-dotnet has no rate limit",
                                                                       ["Grpc.Rate.Limit.PerSeconds"] = "grpc-dotnet has no rate limit",
                                                                       ["Transport.ConnectTimeoutSeconds"] = "grpc-dotnet leaves the dial to its handler",
                                                                       ["Transport.TcpKeepalive.Retries"] = "ServicePoint.SetTcpKeepAlive takes no count",
                                                                       ["Transport.Tls.Client.P12.Password"] = "GrpcClient opens its bundle with no password",
                                                                       ["Transport.Tls.Client.Store.Find.Thumbprint"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Client.Store.Find.SubjectName"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Client.Store.Find.FriendlyName"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Client.Store.Location"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Client.Store.Name"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Server.CaStore.Find.Thumbprint"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Server.CaStore.Find.SubjectName"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Server.CaStore.Find.FriendlyName"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Server.CaStore.Location"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Tls.Server.CaStore.Name"] = "the Windows store, which GrpcClient reads nothing from",
                                                                       ["Transport.Proxy.UrlWithCredentials"] = "GrpcClient's Proxy when its URL carries user:password@, an alternative here",
                                                                       ["Transport.Proxy.None"] = "GrpcClient's Proxy set to `none`, an alternative here rather than a value of an address",
                                                                       ["Transport.Proxy.System.Username"] = "GrpcClient's proxy credentials go with its own address only",
                                                                       ["Transport.Proxy.System.Password"] = "GrpcClient's proxy credentials go with its own address only",
                                                                       ["Http2.KeepAliveIntervalSeconds"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.KeepAliveTimeoutSeconds"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.KeepAliveWhileIdle"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.SimultaneousCallsPerConnection"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Receive.Fixed.StreamWindowSize"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Receive.Fixed.ConnectionWindowSize"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Receive.Adaptive"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Send.CoalescingBytes"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Send.StreamBufferSize"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Send.FramesPerWrite"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                       ["Http2.Send.MaxHeaderListSize"] = "grpc-dotnet's handler owns HTTP/2, and GrpcClient sets none of it",
                                                                     };

  /// <summary>No option is classified twice, which a union of the sets would forgive.</summary>
  /// <remarks>
  ///   An entry in two of them says two different things about one option - mapped and awaited,
  ///   say - and the union the tests below build takes either without noticing.
  /// </remarks>
  [Test]
  public void NoOptionIsClassifiedTwice()
  {
    var named = Counterparts.Keys.Concat(Awaited.Keys)
                            .Concat(Elsewhere.Keys)
                            .ToList();

    Assert.Multiple(() =>
                    {
                      Assert.That(named.GroupBy(name => name,
                                                StringComparer.Ordinal)
                                       .Where(same => same.Count() > 1)
                                       .Select(same => same.Key),
                                  Is.Empty,
                                  "a GrpcClient option classified twice");
                      Assert.That(Counterparts.Values.Intersect(Ours.Keys,
                                                                StringComparer.Ordinal),
                                  Is.Empty,
                                  "an option of this channel both mapped to a counterpart and called our own");
                    });
  }

  /// <summary>Every `GrpcClient` option is accounted for, one way or another.</summary>
  [Test]
  public void EveryOptionOfTheExistingClientIsAccountedFor()
  {
    var accounted = new HashSet<string>(Counterparts.Keys,
                                        StringComparer.Ordinal);
    accounted.UnionWith(Awaited.Keys);
    accounted.UnionWith(Elsewhere.Keys);

    var declared = Settable(typeof(Options.GrpcClient))
      .Select(property => property.Name)
      .ToList();

    Assert.Multiple(() =>
                    {
                      Assert.That(declared.Where(name => !accounted.Contains(name)),
                                  Is.Empty,
                                  "a GrpcClient option with no entry here: map it, name the task that carries it, or say what answers it instead");
                      Assert.That(accounted.Where(name => !declared.Contains(name)),
                                  Is.Empty,
                                  "an entry here for a GrpcClient option that no longer exists");
                    });
  }

  /// <summary>And every option of this channel is either a counterpart or this engine's own.</summary>
  [Test]
  public void EveryOptionOfThisChannelIsAccountedFor()
  {
    var accounted = new HashSet<string>(Counterparts.Values,
                                        StringComparer.Ordinal);
    accounted.UnionWith(Ours.Keys);

    var declared = Paths(typeof(ChannelOptions),
                         string.Empty)
      .ToList();

    Assert.Multiple(() =>
                    {
                      Assert.That(declared.Where(path => !accounted.Contains(path)),
                                  Is.Empty,
                                  "an option of this channel with no entry here: map it to its GrpcClient counterpart, or say why it has none");
                      Assert.That(accounted.Where(path => !declared.Contains(path)),
                                  Is.Empty,
                                  "an entry here for an option the schema no longer declares");
                    });
  }

  /// <summary>What the two vocabularies look like today, so a reader need not run the tests.</summary>
  /// <remarks>
  ///   Not an assertion about the counts, which would fail on every option added for the right
  ///   reasons. It reports, and the two tests above are what refuse.
  /// </remarks>
  [Test]
  public void TheStateOfTheAlignmentIsReported()
  {
    TestContext.Out.WriteLine($"GrpcClient options: {Settable(typeof(Options.GrpcClient)).Count()}");
    TestContext.Out.WriteLine($"  mapped to an option here: {Counterparts.Count}");
    TestContext.Out.WriteLine($"  awaiting a task:          {Awaited.Count}");

    // Key and Value rather than a deconstruction: `KeyValuePair` gained one in .NET Core, and
    // this assembly is exercised on .NET Framework as well.
    foreach (var awaited in Awaited.OrderBy(entry => entry.Value,
                                            StringComparer.Ordinal))
    {
      TestContext.Out.WriteLine($"    {awaited.Value}  {awaited.Key}");
    }

    TestContext.Out.WriteLine($"  answered by something else: {Elsewhere.Count}");
    TestContext.Out.WriteLine($"This channel's options: {Paths(typeof(ChannelOptions), string.Empty).Count()}, of which {Ours.Count} are this engine's own");

    Assert.Pass();
  }

  // A group becomes a prefix, which is the same path .NET's configuration reaches with `__` and
  // the same one the JSON document nests. So does an alternative of a choice, which the path
  // names: an alternative carrying nothing is the environment's value of the choice's path, and
  // the JSON document writes it as a string.
  private static IEnumerable<string> Paths(Type type,
                                           string prefix)
  {
    foreach (var property in Settable(type))
    {
      var path = prefix.Length == 0
                   ? property.Name
                   : prefix + "." + property.Name;

      var held = Nullable.GetUnderlyingType(property.PropertyType) ?? property.PropertyType;

      if (held.IsAbstract && held.Namespace == typeof(ChannelOptions).Namespace)
      {
        foreach (var alternative in held.GetNestedTypes()
                                        .Where(held.IsAssignableFrom))
        {
          var at = path + "." + alternative.Name;
          var fields = Settable(alternative)
            .ToList();

          if (fields.Count == 0 || (fields.Count == 1 && fields[0].Name == "Value"))
          {
            yield return at;
            continue;
          }

          foreach (var one in Paths(alternative,
                                    at))
          {
            yield return one;
          }
        }

        continue;
      }

      // A class of this namespace and not a value type of it: an enum option would otherwise be
      // recursed into, and an enum declares no instance property - so the option would vanish
      // from the paths instead of being reported as one nobody classified.
      if (held.IsClass && held != typeof(string) && held.Namespace == typeof(ChannelOptions).Namespace)
      {
        var nested = Paths(held,
                           path)
          .ToList();

        if (nested.Count == 0)
        {
          throw new InvalidOperationException($"`{path}` is a group of this namespace that declares no option, so it would leave the paths without being classified");
        }

        foreach (var one in nested)
        {
          yield return one;
        }

        continue;
      }

      yield return path;
    }
  }

  private static IEnumerable<PropertyInfo> Settable(Type type)
    => type.GetProperties(BindingFlags.Public | BindingFlags.Instance)
           .Where(property => property.CanRead && property.CanWrite);
}
