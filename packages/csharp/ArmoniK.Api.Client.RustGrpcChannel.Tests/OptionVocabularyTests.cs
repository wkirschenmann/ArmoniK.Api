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
///     this instead of quietly having no counterpart. Phases 4 to 6 are what move a name out of
///     <see cref="Awaited" /> and into <see cref="Counterparts" />.
///   </para>
/// </remarks>
[TestFixture]
public class OptionVocabularyTests
{
  /// <summary>A `GrpcClient` option and the path of the option that answers it here.</summary>
  private static readonly IReadOnlyDictionary<string, string> Counterparts = new Dictionary<string, string>(StringComparer.Ordinal);

  /// <summary>A `GrpcClient` option whose counterpart a later phase brings, and which one.</summary>
  /// <remarks>
  ///   The phase is named so the entry says when it stops being an omission. `RequestTimeout` is
  ///   a deadline rather than a connection option, which is why it waits on T6.2 and not on the
  ///   transport unit.
  /// </remarks>
  private static readonly IReadOnlyDictionary<string, string> Awaited = new Dictionary<string, string>(StringComparer.Ordinal)
                                                                        {
                                                                          ["AllowUnsafeConnection"] = "T4.1",
                                                                          ["CaCert"]                = "T4.1",
                                                                          ["CertP12"]               = "T4.2",
                                                                          ["CertPem"]               = "T4.2",
                                                                          ["KeyPem"]                = "T4.2",
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
                                                                       ["DeliveryCredits"] = "how many events the engine may hold for an unread call, which only this ABI has",
                                                                       ["MaxSendsInFlight"] = "the send window, which only this ABI has",
                                                                       ["MaxReceiveMessageSize"] = "grpc-dotnet takes this per method rather than per channel",
                                                                       ["UserAgent"] = "grpc-dotnet writes its own and offers no option",
                                                                       ["Transport.ConnectTimeoutSeconds"] = "grpc-dotnet leaves the dial to its handler",
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
                                  "a GrpcClient option with no entry here: map it, name the phase that will, or say what answers it instead");
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
    TestContext.Out.WriteLine($"  awaiting a phase:         {Awaited.Count}");

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
