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
using System.Reflection;
using System.Text.RegularExpressions;

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
  private static readonly IReadOnlyDictionary<string, string> Counterparts = new Dictionary<string, string>(StringComparer.Ordinal);

  /// <summary>A `GrpcClient` option this channel does not answer, and the task that carries it.</summary>
  /// <remarks>
  ///   The task is named so the entry says what ends the omission. `RequestTimeout` is a deadline
  ///   rather than a connection option, which is why its task is T6.2 and not the transport
  ///   unit's.
  /// </remarks>
  private static readonly IReadOnlyDictionary<string, string> Awaited = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                        {
                                                                          ["AllowUnsafeConnection"] = "T4.1",
                                                                          ["CaCert"]                = "T4.1",
                                                                          ["CertP12"]               = "T4.2",
                                                                          ["CertPem"]               = "T4.1",
                                                                          ["KeyPem"]                = "T4.1",
                                                                          ["OverrideTargetName"]    = "T4.1",
                                                                          ["KeepAliveTime"]         = "T4.1",
                                                                          ["KeepAliveTimeInterval"] = "T4.1",
                                                                          ["MaxIdleTime"]           = "T4.1",
                                                                          ["Proxy"]                 = "T5.1",
                                                                          ["ProxyUsername"]         = "T5.1",
                                                                          ["ProxyPassword"]         = "T5.1",
                                                                          ["MaxAttempts"]           = "T6.3",
                                                                          ["BackoffMultiplier"]     = "T6.3",
                                                                          ["InitialBackOff"]        = "T6.3",
                                                                          ["MaxBackOff"]            = "T6.3",
                                                                          ["RequestTimeout"]        = "T6.2",
                                                                        };

  /// <summary>A `GrpcClient` option this engine answers with something that is not an option.</summary>
  private static readonly IReadOnlyDictionary<string, string> Elsewhere = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                          {
                                                                            ["Endpoint"] = "the endpoint crosses the ABI as `ak_channel_create`'s own argument, which is what lets every option have a default",
                                                                            ["HttpMessageHandler"] = "grpc-dotnet chooses a handler, and this engine is the handler",
                                                                            ["ReusePorts"] = "a socket option of grpc-dotnet's handler, which this engine does not use",
                                                                          };

  /// <summary>An option of this channel that `GrpcClient` has no name for.</summary>
  private static readonly IReadOnlyDictionary<string, string> Ours = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                     {
                                                                       ["DeliveryCredits"] = "the delivery window, which only this ABI has",
                                                                       ["MaxSendsInFlight"] = "the send window, which only this ABI has",
                                                                       ["MaxReceiveMessageSize"] = "grpc-dotnet takes this per method rather than per channel",
                                                                       ["UserAgent"] = "grpc-dotnet writes its own and offers no option",
                                                                       ["Transport.ConnectTimeoutSeconds"] = "grpc-dotnet leaves the dial to its handler",
                                                                     };

  /// <summary>A `GrpcClient__` name the Rust reader knows and the .NET options do not declare.</summary>
  /// <remarks>
  ///   Three readers share the `GrpcClient__` namespace, not two: `Options.GrpcClient` here,
  ///   `ClientConfigArgs::from_env` in the Rust client, and this channel's schema. The two
  ///   vocabularies below are pinned rather than reconciled - which spelling wins is #736's to
  ///   decide - so what this catches is a name moving on one side alone. An unknown option is
  ///   ignored rather than refused, so drift here fails late and silently everywhere else.
  /// </remarks>
  private static readonly IReadOnlyDictionary<string, string> RustOnly = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                        {
                                                                          ["ConnectTimeout"] = "no .NET counterpart: grpc-dotnet leaves the dial to its handler",
                                                                          ["Timeout"] = "the .NET option is spelled RequestTimeout",
                                                                          ["RateLimit"] = "no .NET counterpart",
                                                                          ["TcpKeepalive"] = "the .NET option is spelled KeepAliveTime",
                                                                          ["TcpKeepaliveInterval"] = "the .NET option is spelled KeepAliveTimeInterval",
                                                                          ["TcpKeepaliveRetries"] = "no .NET counterpart",
                                                                          ["TcpNagleAlgorithm"] = "no .NET counterpart",
                                                                          ["Http2KeepAliveInterval"] = "no .NET counterpart: the handler owns HTTP/2 there",
                                                                          ["Http2KeepAliveTimeout"] = "no .NET counterpart: the handler owns HTTP/2 there",
                                                                          ["Http2KeepAliveWhileIdle"] = "no .NET counterpart: the handler owns HTTP/2 there",
                                                                          ["Http2MaxHeaderListSize"] = "no .NET counterpart: the handler owns HTTP/2 there",
                                                                          ["UserAgent"] = "no .NET counterpart: grpc-dotnet writes its own",
                                                                        };

  /// <summary>The two readers of `GrpcClient__` differ exactly where they are known to.</summary>
  [Test]
  public void TheRustReaderAndTheDotNetOptionsDifferWhereTheyAreKnownTo()
  {
    var rust = NamesTheRustReaderKnows();
    var declared = new HashSet<string>(Settable(typeof(Options.GrpcClient))
                                         .Select(property => property.Name),
                                       StringComparer.Ordinal);

    Assert.Multiple(() =>
                    {
                      Assert.That(rust,
                                  Is.Not.Empty,
                                  "the Rust reader's names were not found: the path or the call shape changed");
                      Assert.That(rust.Where(name => !declared.Contains(name) && !RustOnly.ContainsKey(name)),
                                  Is.Empty,
                                  "a GrpcClient__ name the Rust client reads that .NET neither declares nor excuses");
                      Assert.That(RustOnly.Keys.Where(name => !rust.Contains(name)),
                                  Is.Empty,
                                  "an entry here for a GrpcClient__ name the Rust client no longer reads");
                      Assert.That(RustOnly.Keys.Where(declared.Contains),
                                  Is.Empty,
                                  "a name excused as Rust-only that .NET does declare");
                    });
  }

  /// <summary>The `GrpcClient__` names `ClientConfigArgs::from_env` reads, from its source.</summary>
  /// <remarks>Read rather than mirrored, because a list kept here is the drift it exists to
  /// catch. The calls are the truth: a name reaches the reader through one of them or not at
  /// all.</remarks>
  private static IReadOnlyCollection<string> NamesTheRustReaderKnows()
  {
    var recorded = typeof(OptionVocabularyTests).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
                                                .FirstOrDefault(metadata => metadata.Key == "RustClientConfig")
                                                ?.Value;
    Assert.That(recorded,
                Is.Not.Null.And.Not.Empty,
                "the build recorded no RustClientConfig");

    var source = Path.GetFullPath(recorded!);
    Assert.That(File.Exists(source),
                Is.True,
                $"the Rust client's config is not where the build said: {source}");

    // Cast first: on .NET Framework a `MatchCollection` is only the non-generic `IEnumerable`, so
    // `Select` does not reach it and this half of the suite is the one nothing else compiles.
    return Regex.Matches(File.ReadAllText(source),
                         @"read_env(?:_bool)?\(""GrpcClient__(?<name>[A-Za-z0-9]+)""")
                .Cast<Match>()
                .Select(match => match.Groups["name"].Value)
                .Distinct(StringComparer.Ordinal)
                .ToList();
  }

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
  // the same one the JSON document nests.
  private static IEnumerable<string> Paths(Type type,
                                           string prefix)
  {
    foreach (var property in Settable(type))
    {
      var path = prefix.Length == 0
                   ? property.Name
                   : prefix + "." + property.Name;

      var held = Nullable.GetUnderlyingType(property.PropertyType) ?? property.PropertyType;

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
