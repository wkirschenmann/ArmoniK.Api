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
using System.IO;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator
{
  /// <summary>Command line around <see cref="OptionVocabulary" /> and <see cref="CSharpSource" />.</summary>
  public static class Program
  {
    private const string Usage = @"Generates the C# options class of the native transport channel.

Usage:
  dotnet run --project packages/csharp/ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator -- \
    --schema <path to the JSON schema> --output <path to the .cs file> [--namespace <ns>] [--check]

Options:
  --schema <path>     The option schema, as printed by
                      `cargo run -p armonik-transport --features schema --example schema`.
  --output <path>     The file to write; its directory has to exist.
  --namespace <ns>    The namespace of the generated classes. Defaults to
                      ArmoniK.Api.Client.RustGrpcChannel.
  --check             Writes nothing and fails if --output is not what --schema renders.
  -h, --help          Prints this text.

The same schema always renders the same bytes, which is what makes --check a build step.";

    private const string DefaultNamespace = "ArmoniK.Api.Client.RustGrpcChannel";

    /// <summary>Reads the schema named on the command line and writes the options class.</summary>
    /// <param name="args">The command line arguments.</param>
    /// <returns>
    ///   0 on success, 1 on a bad command line, 2 on a schema this generator cannot read, 3 on an
    ///   output it cannot write, 4 when --check finds the output stale.
    /// </returns>
    public static async Task<int> Main(string[] args)
    {
      string? schemaPath = null;
      string? outputPath = null;
      var     namespaceName = DefaultNamespace;
      var     check         = false;

      // One rule, applied wherever an option takes a value: reading an argument consumes it.
      var read = 0;

      while (read < args.Length)
      {
        var argument = args[read++];

        switch (argument)
        {
          case "-h":
          case "--help":
            Console.Out.WriteLine(Usage);
            return 0;
          case "--schema" when read < args.Length:
            schemaPath = args[read++];
            break;
          case "--output" when read < args.Length:
            outputPath = args[read++];
            break;
          case "--namespace" when read < args.Length:
            namespaceName = args[read++];
            break;
          case "--check":
            check = true;
            break;
          // Named separately from the default, or an option written last with nothing after it
          // falls through its `when` and is reported as an argument nobody knows.
          case "--schema":
          case "--output":
          case "--namespace":
            Console.Error.WriteLine($"'{argument}' takes a value, and nothing follows it.");
            Console.Error.WriteLine(Usage);
            return 1;
          default:
            Console.Error.WriteLine($"Unexpected argument '{argument}'.");
            Console.Error.WriteLine(Usage);
            return 1;
        }
      }

      if (schemaPath is null || outputPath is null)
      {
        Console.Error.WriteLine("Both --schema and --output are required.");
        Console.Error.WriteLine(Usage);
        return 1;
      }

      string generated;

      try
      {
        // A build passes these as whatever its own layout spells, and `a/../b` in a message is a
        // path the reader has to resolve before recognising the file. Inside the try, because a
        // path can be refused for its shape and the reason belongs on one line like the others.
        schemaPath = Path.GetFullPath(schemaPath);
        outputPath = Path.GetFullPath(outputPath);

        var schemaJson = File.ReadAllText(schemaPath);

        var unhandled = OptionVocabulary.Unhandled(schemaJson);

        if (unhandled.Count > 0)
        {
          // A keyword the C# does not read is a constraint the engine enforces and this side does
          // not, which is a value a caller can set and only the far end of the ABI refuses.
          Console.Error.WriteLine($"'{schemaPath}' states {string.Join(", ", unhandled)}, which this generator reads as nothing. Teach it that keyword, or drop it from the schema.");
          return 2;
        }

        var groups = await OptionVocabulary.ReadAsync(schemaJson)
                                           .ConfigureAwait(false);

        generated = CSharpSource.Render(groups,
                                        namespaceName,
                                        Path.GetFileName(schemaPath));
      }
      catch (Exception e) when (IsFileFailure(e) || e is JsonException or NotSupportedException or InvalidOperationException)
      {
        Console.Error.WriteLine($"Cannot generate from '{schemaPath}': {e.Message}");
        return 2;
      }

      if (check)
      {
        string? committed;

        try
        {
          // Read as bytes: `File.ReadAllText` strips a byte order mark before a comparison could
          // see it, and this tool writes none - so a file that acquired one from an editor would
          // differ from what it renders and pass anyway. Line endings are normalised because a
          // checkout may have written them, which .gitattributes asks it not to.
          committed = File.Exists(outputPath)
                        ? new UTF8Encoding(false).GetString(File.ReadAllBytes(outputPath))
                                                 .Replace("\r\n",
                                                          "\n")
                        : null;
        }
        catch (Exception e) when (IsFileFailure(e))
        {
          Console.Error.WriteLine($"Cannot read '{outputPath}': {e.Message}");
          return 3;
        }

        if (committed == generated)
        {
          return 0;
        }

        Console.Error.WriteLine($"'{outputPath}' is not what '{schemaPath}' renders. Write it again with `dotnet run --project packages/csharp/ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator -- --schema {schemaPath} --output {outputPath}`.");
        return 4;
      }

      try
      {
        // No byte order mark, and the newlines the renderer chose: the file is compared byte for
        // byte with what --check renders, on every machine.
        File.WriteAllText(outputPath,
                          generated,
                          new UTF8Encoding(false));
      }
      catch (Exception e) when (IsFileFailure(e))
      {
        Console.Error.WriteLine($"Cannot write '{outputPath}': {e.Message}");
        return 3;
      }

      return 0;
    }

    // A path can fail on its shape, on its permissions or on the drive, and a caller wants the
    // reason on one line rather than a stack trace.
    private static bool IsFileFailure(Exception e)
      => e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException;
  }
}
