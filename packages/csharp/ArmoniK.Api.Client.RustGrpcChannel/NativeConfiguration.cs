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
using System.Text;
using System.Text.Json;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>Where the engine reads a runtime's options from, for <see cref="NativeRuntime.Create(NativeConfiguration,ILoggerFactory)" />.</summary>
/// <remarks>
///   <para>
///     Each load adds a source, and the engine reads them in the order they were added when the
///     runtime is created, a later one over an earlier one option by option. The keys are those
///     of <see cref="RuntimeOptions" />, under a prefix: <c>ArmoniK__Client__Grpc</c> unless the
///     constructor names another, and none when it names the empty one. A prefix is a path, its
///     parts joined by <c>__</c> or <c>:</c>: the sections of a file, the start of a variable's name.
///   </para>
///   <para>
///     The files and the environment are read by the engine alone, so every host of it reads them
///     the same way. A key no option declares is ignored at the root of a document and refused
///     below it; a value that does not fit its key is refused when the runtime is created, by its
///     source and its path, and never quoted.
///   </para>
/// </remarks>
public sealed class NativeConfiguration
{
  /// <summary>The prefix the options are read under when the constructor names none.</summary>
  public const string DefaultPrefix = "ArmoniK__Client__Grpc";

  private readonly List<(ak_source_kind Kind, byte[] Value)> sources_ = new();

  /// <summary>Options read under <see cref="DefaultPrefix" />, from no source yet.</summary>
  public NativeConfiguration()
    : this(DefaultPrefix)
  {
  }

  /// <summary>Options read under <paramref name="prefix" />, from no source yet.</summary>
  /// <param name="prefix">
  ///   The section of a file, and the start of an environment variable's name, the options are
  ///   read from. Empty is none: a file's options are then the whole file, and the environment
  ///   is refused, since every variable of the process would be a key.
  /// </param>
  /// <exception cref="ArgumentNullException"><paramref name="prefix" /> is null.</exception>
  public NativeConfiguration(string prefix)
    => Prefix = prefix ?? throw new ArgumentNullException(nameof(prefix));

  /// <summary>The prefix the options are read under, empty for none.</summary>
  public string Prefix { get; }

  /// <summary>Adds files, JSON, YAML or TOML by their extension, each refused when it does not exist.</summary>
  /// <param name="files">Their paths, in the order they are read.</param>
  /// <returns>This configuration.</returns>
  /// <exception cref="ArgumentNullException">A path is null.</exception>
  public NativeConfiguration LoadConfigFromFiles(params string[] files)
    => Files(ak_source_kind.AK_SOURCE_FILE,
             files);

  /// <summary>Adds files, as <see cref="LoadConfigFromFiles" /> does, each contributing nothing when it does not exist.</summary>
  /// <param name="files">Their paths, in the order they are read.</param>
  /// <returns>This configuration.</returns>
  /// <exception cref="ArgumentNullException">A path is null.</exception>
  public NativeConfiguration LoadConfigFromOptionalFiles(params string[] files)
    => Files(ak_source_kind.AK_SOURCE_OPTIONAL_FILE,
             files);

  /// <summary>Adds the process environment: the variables whose name starts with the prefix and <c>__</c>.</summary>
  /// <returns>This configuration.</returns>
  /// <remarks>Read once, when the runtime is created. The rest of a name is the key's path, its parts
  /// joined by <c>__</c> and compared without case, and a value is text read by its key's type. A
  /// list option is one variable holding a JSON array, <c>["Zstd","Gzip"]</c>. An alternative that carries
  /// nothing is the variable's value, <c>ArmoniK__Client__Grpc__ChannelDefaults__Transport__Proxy=None</c>.</remarks>
  public NativeConfiguration LoadConfigFromEnvironment()
    => With(ak_source_kind.AK_SOURCE_ENVIRONMENT,
            Array.Empty<byte>());

  /// <summary>Adds a command line, in the syntaxes .NET's command-line configuration reads.</summary>
  /// <param name="args">The arguments, such as <c>--ArmoniK:Client:Grpc:Endpoint=http://host:5001</c>.</param>
  /// <returns>This configuration.</returns>
  /// <exception cref="ArgumentNullException"><paramref name="args" /> is null.</exception>
  /// <remarks>
  ///   The keys under the prefix, the whole command line with none, reach the engine as text, read
  ///   as the environment's values are: by their key's type, a key no option declares ignored at the
  ///   root and refused below it. A command line states no list, and the engine refuses a list
  ///   option on one by its path. An alternative that carries nothing is the key's value, <c>--ArmoniK:Client:Grpc:ChannelDefaults:Transport:Proxy=None</c>.
  /// </remarks>
  public NativeConfiguration LoadConfigFromCommandLine(string[] args)
  {
    if (args is null)
    {
      throw new ArgumentNullException(nameof(args));
    }

    var parsed = new ConfigurationBuilder().AddCommandLine(args)
                                           .Build();
    IConfiguration section = Prefix.Length == 0
                               ? parsed
                               : parsed.GetSection(Prefix.Replace("__",
                                                                  ConfigurationPath.KeyDelimiter));

    using var written = new MemoryStream();
    using (var writer = new Utf8JsonWriter(written))
    {
      writer.WriteStartObject();
      foreach (var (path, value) in Values(section,
                                           string.Empty))
      {
        writer.WriteString(path,
                           value);
      }

      writer.WriteEndObject();
    }

    return With(ak_source_kind.AK_SOURCE_PAIRS,
                written.ToArray());
  }

  /// <summary>Adds options set in code, of which only those set are read.</summary>
  /// <param name="options">The options, read now: a later change to them is not.</param>
  /// <returns>This configuration.</returns>
  /// <exception cref="ArgumentNullException"><paramref name="options" /> is null.</exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside its stated bounds.</exception>
  public NativeConfiguration LoadConfigFromObject(RuntimeOptions options)
  {
    if (options is null)
    {
      throw new ArgumentNullException(nameof(options));
    }

    return With(ak_source_kind.AK_SOURCE_DOCUMENT,
                options.Encode());
  }

  /// <summary>The sources, in the order they were added.</summary>
  internal IReadOnlyList<(ak_source_kind Kind, byte[] Value)> Sources
    => sources_;

  private NativeConfiguration Files(ak_source_kind kind,
                                    string[]       files)
  {
    if (files is null || files.Any(file => file is null))
    {
      throw new ArgumentNullException(nameof(files));
    }

    foreach (var file in files)
    {
      With(kind,
           Encoding.UTF8.GetBytes(file));
    }

    return this;
  }

  private NativeConfiguration With(ak_source_kind kind,
                                   byte[]         value)
  {
    sources_.Add((kind, value));
    return this;
  }

  // Every key holding a value, its path's parts joined by `__`, the separator the engine reads.
  private static IEnumerable<(string Path, string Value)> Values(IConfiguration section,
                                                                string         under)
  {
    foreach (var child in section.GetChildren())
    {
      var path = under.Length == 0
                   ? child.Key
                   : $"{under}__{child.Key}";
      if (child.Value is not null)
      {
        yield return (path, child.Value);
      }

      foreach (var nested in Values(child,
                                    path))
      {
        yield return nested;
      }
    }
  }
}
