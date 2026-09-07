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
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

using Corvus.Json;
using Corvus.Json.CodeGeneration;
using Corvus.Json.CodeGeneration.DocumentResolvers;

namespace ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator
{
  /// <summary>What the schema says an option is, once its references are resolved.</summary>
  internal sealed class Option
  {
    /// <summary>The name the document spells, which is also the C# name.</summary>
    public string Name { get; init; } = string.Empty;

    /// <summary>The `description` that applies here, which the schema has to state.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>The C# type of the property, nullable form excluded.</summary>
    public string Type { get; init; } = string.Empty;

    /// <summary>Whether <see cref="Type" /> is one of the generated classes.</summary>
    public bool IsGroup { get; init; }

    /// <summary>The keywords that bound the value, empty where the schema bounds nothing.</summary>
    public IReadOnlyList<Bound> Bounds { get; init; } = Array.Empty<Bound>();
  }

  /// <summary>One bound the schema states.</summary>
  /// <param name="Keyword">The schema keyword, verbatim.</param>
  /// <param name="Literal">Its value, as C# writes that number.</param>
  internal readonly record struct Bound(string Keyword,
                                        string Literal);

  /// <summary>A group of options, which is one generated class.</summary>
  internal sealed class OptionGroup
  {
    /// <summary>The class name: the schema's `title`, or the `$defs` entry's own name.</summary>
    public string Name { get; init; } = string.Empty;

    /// <summary>The `description` of the group, which the schema has to state.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>The options of the group, in the order Corvus returns them.</summary>
    public IReadOnlyList<Option> Options { get; init; } = Array.Empty<Option>();
  }

  /// <summary>
  ///   The option vocabulary a schema describes: every group, the root one first.
  /// </summary>
  /// <remarks>
  ///   Corvus resolves the document - `$ref`, `$defs`, and the draft's own rules about which
  ///   keywords apply where - and this reads the resolved graph. Nothing here parses a reference
  ///   or merges a subschema; what it does is decide which C# a resolved node becomes.
  /// </remarks>
  internal static class OptionVocabulary
  {
    private const string RefSubschema = "#/$ref";

    private const string SchemaUri = "schema://armonik/options.json";

    /// <summary>Reads <paramref name="schemaJson" /> and returns the groups it describes.</summary>
    /// <param name="schemaJson">A JSON schema, draft 2020-12.</param>
    /// <returns>The root group first, then every group it reaches, each once.</returns>
    /// <exception cref="NotSupportedException">
    ///   The schema uses a construct this generator does not turn into C#. Refusing is what keeps
    ///   a new option from being emitted as something that compiles and means nothing.
    /// </exception>
    public static async Task<IReadOnlyList<OptionGroup>> ReadAsync(string schemaJson)
    {
      using var document = JsonDocument.Parse(schemaJson);

      RefuseCycles(document);

      using PrepopulatedDocumentResolver resolver = new();
      resolver.AddDocument(SchemaUri,
                           document);

      VocabularyRegistry vocabularies = new();
      Corvus.Json.CodeGeneration.Draft202012.VocabularyAnalyser.RegisterAnalyser(resolver,
                                                                                vocabularies);

      JsonSchemaTypeBuilder builder = new(resolver,
                                          vocabularies);
      var root = await builder.AddTypeDeclarationsAsync(new JsonReference(SchemaUri),
                                                        Corvus.Json.CodeGeneration.Draft202012.VocabularyAnalyser.DefaultVocabulary,
                                                        false)
                              .ConfigureAwait(false);

      var groups = new List<OptionGroup>();
      var read = new HashSet<string>(StringComparer.Ordinal);

      Read(root,
           NameOf(root,
                  true),
           groups,
           read);

      return groups;
    }

    // Depth first, and a group is read once: the root is emitted first, and a group reached twice
    // is one class rather than two.
    private static void Read(TypeDeclaration declaration,
                             string name,
                             List<OptionGroup> groups,
                             HashSet<string> read)
    {
      if (!read.Add(declaration.LocatedSchema.Location.ToString()))
      {
        return;
      }

      // An object stating no properties is a class with nothing in it, and whatever it was meant
      // to carry - an open map, a shape stated some other way - a generated property cannot hold.
      // Checked here rather than at each property, so the root is checked too.
      if (!declaration.HasPropertyDeclarations)
      {
        throw new NotSupportedException($"`{declaration.LocatedSchema.Location}` states no properties, which this generator has no class for.");
      }

      var options = new List<Option>();
      var nested = new List<(TypeDeclaration Declaration, string Name)>();

      foreach (var property in declaration.PropertyDeclarations)
      {
        var node = property.ReducedPropertyType;
        var target = Resolve(node);

        var type = Keyword(target,
                           "type")
                   ?.GetString();

        if (type == "object" || target.HasPropertyDeclarations)
        {
          // A group is bounded by what it declares, not by the option that holds one. Written
          // on the option it would bind that one embedding and not the next, which is the
          // opposite of what a `$defs` entry is for - and the keywords this generator checks
          // bound a number or a string, so on a group they asserted nothing and were dropped.
          var bounds = BoundsOf(node,
                                target);

          if (bounds.Count > 0)
          {
            throw new NotSupportedException($"`{property.JsonPropertyName}` is a group and states {string.Join(", ", bounds.Select(bound => $"`{bound.Keyword}`"))}, which bounds no group. State it on the options the group declares.");
          }

          var groupName = NameOf(target,
                                 false);
          nested.Add((target, groupName));

          options.Add(new Option
                      {
                        Name = property.JsonPropertyName,
                        Description = Described(Description(node) ?? Description(target),
                                                $"`{property.JsonPropertyName}`"),
                        Type    = groupName,
                        IsGroup = true,
                      });
          continue;
        }

        options.Add(new Option
                    {
                      Name = property.JsonPropertyName,
                      // The property's own, then the one its reference states: a `$defs` entry
                      // describes what it is, and a property describes what it is for here.
                      Description = Described(Description(node) ?? Description(target),
                                              $"`{property.JsonPropertyName}`"),
                      Type = CSharpType(type,
                                        Keyword(target,
                                                "format")
                                          ?.GetString(),
                                        property.JsonPropertyName),
                      Bounds = BoundsOf(node,
                                        target),
                    });
      }

      // Two schemas of one name would be one class declared twice, which the C# compiler reports
      // as a duplicate member in a generated file nobody wrote.
      if (groups.Any(group => group.Name == name))
      {
        throw new NotSupportedException($"`{declaration.LocatedSchema.Location}` is named `{name}`, which another schema already is.");
      }

      groups.Add(new OptionGroup
                 {
                   Name = name,
                   Description = Described(Description(declaration),
                                           $"`{name}`"),
                   Options = options,
                 });

      foreach (var (subdeclaration, subname) in nested)
      {
        Read(subdeclaration,
             subname,
             groups,
             read);
      }
    }

    // A property whose schema is a `$ref` states its type there, and may state its own keywords
    // beside it - which draft 2020-12 allows and earlier drafts did not. So the reference is
    // followed for what the node does not say, and the node keeps what it does.
    //
    // No cycle guard: `RefuseCycles` has already run on the document, and it has to - a cycle
    // overflows the stack inside Corvus's own reduction, which `ReducedPropertyType` above
    // reaches long before anything here.
    private static TypeDeclaration Resolve(TypeDeclaration declaration)
      => declaration.SubschemaTypeDeclarations.TryGetValue(RefSubschema,
                                                           out var target)
           ? Resolve(target)
           : declaration;

    /// <summary>Refuses a document whose `$ref`s lead in a circle.</summary>
    /// <param name="document">The schema, parsed.</param>
    /// <exception cref="NotSupportedException">A reference leads back to itself.</exception>
    /// <remarks>
    ///   Checked on the document rather than on the type graph because a cycle takes the process
    ///   down before the graph exists: Corvus reduces a `$ref` by following it, and a circle makes
    ///   that recursion overflow the stack - which .NET cannot catch, so nothing would name the
    ///   schema at fault. The generated `Validate()` recurses through nested groups too, so a
    ///   cycle admitted here would also be one a caller could trigger at run time.
    /// </remarks>
    private static void RefuseCycles(JsonDocument document)
    {
      var references = new Dictionary<string, string>(StringComparer.Ordinal);

      Collect(document.RootElement,
              "#");

      foreach (var (from, _) in references)
      {
        var followed = new HashSet<string>(StringComparer.Ordinal);
        var at       = from;

        while (references.TryGetValue(at,
                                      out var next))
        {
          if (!followed.Add(at))
          {
            throw new NotSupportedException($"`{from}` is reached by a cycle of `$ref` through `{at}`, which names no type.");
          }

          at = next;
        }
      }

      void Collect(JsonElement element,
                   string at)
      {
        switch (element.ValueKind)
        {
          case JsonValueKind.Object:
            foreach (var member in element.EnumerateObject())
            {
              if (member.Name == "$ref" && member.Value.ValueKind == JsonValueKind.String)
              {
                Record(at,
                       member.Value.GetString());
              }

              Collect(member.Value,
                      at + "/" + Escaped(member.Name));
            }

            break;

          // `allOf`, `prefixItems` and their kind hold subschemas in an array, and a `$ref` there
          // reaches Corvus exactly as one in an object does.
          case JsonValueKind.Array:
            var index = 0;

            foreach (var item in element.EnumerateArray())
            {
              Collect(item,
                      at + "/" + index.ToString(CultureInfo.InvariantCulture));
              index++;
            }

            break;
        }
      }

      void Record(string at,
                  string? target)
      {
        // Only a local pointer: one document is all the resolver holds, so any other form of
        // reference names nothing this generator could follow either way.
        if (target is null || !target.StartsWith("#", StringComparison.Ordinal))
        {
          return;
        }

        // A reference to a schema that contains this one closes a circle no chain of `$ref`s
        // would show: the edge leaves the ancestor by a different member each time round. It is
        // also what a self-referential root looks like, and the generated class would then hold
        // a property of its own type, whose `Validate()` recurses without end.
        if (at == target || at.StartsWith(target + "/",
                                          StringComparison.Ordinal))
        {
          throw new NotSupportedException($"`{at}` refers to `{target}`, which contains it - a cycle that names no type.");
        }

        references[at] = target;
      }
    }

    // A JSON pointer spells `~` as `~0` and `/` as `~1`, so a member name carrying either is not
    // the segment a `$ref` to it would write. Comparing the two unescaped forms would miss the
    // edge, and a missed edge is the stack overflow this guard exists to prevent.
    private static string Escaped(string segment)
      => segment.Replace("~",
                         "~0")
                .Replace("/",
                         "~1");

    private static string CSharpType(string? type,
                                     string? format,
                                     string option)
      => (type, format) switch
         {
           ("integer", "int32") => "int",
           ("integer", "int64") => "long",
           ("number", "double") => "double",
           ("string", null)     => "string",
           ("boolean", _)       => "bool",
           _ => throw new NotSupportedException($"`{option}` is `{type ?? "no type"}` with format `{format ?? "none"}`, which this generator does not turn into C#."),
         };

    /// <summary>The keywords that bound a value, which this generator turns into a check.</summary>
    /// <remarks>
    ///   Listed here so a keyword nobody handles is not silently dropped: one absent from a schema
    ///   states no bound, but one present and unlisted would be a constraint the C# lets through.
    ///   <see cref="Unhandled" /> is what refuses that case.
    /// </remarks>
    public static readonly string[] Keywords =
    {
      "minimum",
      "maximum",
      "exclusiveMinimum",
      "exclusiveMaximum",
      "minLength",
      "maxLength",
    };

    // Only the keywords the schema actually states: a bound nobody wrote is not a check.
    //
    // A property states its own keywords beside a `$ref` and the reference states its own, and
    // draft 2020-12 applies both - so a value has to satisfy both, and the stricter is the one to
    // check. Taking the property's alone would admit what the reference forbids.
    private static IReadOnlyList<Bound> BoundsOf(TypeDeclaration node,
                                                 TypeDeclaration target)
    {
      var bounds = new List<Bound>();

      foreach (var keyword in Keywords)
      {
        var here = Keyword(node,
                          keyword);
        var there = ReferenceEquals(node,
                                    target)
                      ? null
                      : Keyword(target,
                                keyword);

        var stated = (here, there) switch
                     {
                       (null, null)     => null,
                       (not null, null) => Literal(here!.Value),
                       (null, not null) => Literal(there!.Value),
                       _ => Stricter(keyword,
                                     here!.Value,
                                     there!.Value),
                     };

        if (stated is not null)
        {
          bounds.Add(new Bound(keyword,
                               stated));
        }
      }

      return bounds;
    }

    // Of two bounds that both apply, the one that admits less: the larger of two lower bounds,
    // the smaller of two upper ones.
    //
    // Compared as integers where both are, because a `double` holds 53 bits of mantissa and two
    // bounds differing only past that would compare equal - so the value written stays exact,
    // as `Literal` keeps it, and the choice between two does too.
    private static string Stricter(string keyword,
                                   JsonElement here,
                                   JsonElement there)
    {
      var lower = keyword is "minimum" or "exclusiveMinimum" or "minLength";

      var greater = here.TryGetInt64(out var whole) && there.TryGetInt64(out var otherWhole)
                      ? whole > otherWhole
                      : here.GetDouble() > there.GetDouble();

      return Literal(greater == lower
                       ? here
                       : there);
    }

    // The schema's number as C# writes it. An integral bound is written as an integer, so a check
    // on an `int` does not silently widen to double - and read as one, because a `double` holds
    // only 53 bits of mantissa and a bound past that would move on its way through.
    private static string Literal(JsonElement value)
    {
      if (value.TryGetInt64(out var whole))
      {
        return whole.ToString(CultureInfo.InvariantCulture);
      }

      return value.GetDouble()
                  .ToString("R",
                            CultureInfo.InvariantCulture);
    }

    /// <summary>The keywords <paramref name="schemaJson" /> states and nobody handles.</summary>
    /// <param name="schemaJson">A JSON schema, draft 2020-12.</param>
    /// <returns>Each unhandled keyword once, in the order the document reaches them.</returns>
    /// <remarks>
    ///   <para>
    ///     A keyword this generator does not read is a constraint the engine enforces and the C#
    ///     does not, which is a value a caller can set and only the far side refuses. Naming them
    ///     is what turns that into a build failure rather than a run-time one.
    ///   </para>
    ///   <para>
    ///     What is reported is every keyword not in <see cref="Understood" /> rather than every
    ///     keyword in some list of assertions: a list of what to refuse can never be complete,
    ///     and a keyword nobody thought of is exactly the one to hear about. The walk follows the
    ///     document's structure, so a property named `pattern` is a name and not a keyword.
    ///   </para>
    /// </remarks>
    public static IReadOnlyList<string> Unhandled(string schemaJson)
    {
      using var document = JsonDocument.Parse(schemaJson);

      var unhandled = new List<string>();

      Schema(document.RootElement);

      return unhandled;

      void Report(string keyword)
      {
        if (!unhandled.Contains(keyword))
        {
          unhandled.Add(keyword);
        }
      }

      void Schema(JsonElement element)
      {
        if (element.ValueKind != JsonValueKind.Object)
        {
          return;
        }

        foreach (var member in element.EnumerateObject())
        {
          if (SubschemaMaps.Contains(member.Name) || ReportedMaps.Contains(member.Name))
          {
            if (member.Value.ValueKind == JsonValueKind.Object)
            {
              foreach (var subschema in member.Value.EnumerateObject())
              {
                Schema(subschema.Value);
              }
            }

            if (ReportedMaps.Contains(member.Name))
            {
              Report(member.Name);
            }

            continue;
          }

          if (SubschemaLists.Contains(member.Name))
          {
            if (member.Value.ValueKind == JsonValueKind.Array)
            {
              foreach (var subschema in member.Value.EnumerateArray())
              {
                Schema(subschema);
              }
            }

            Report(member.Name);
            continue;
          }

          // `additionalProperties` is what closes an object, and a generated class is closed -
          // but only its `false` form says so. Judged on its value, because the name alone does
          // not say which of the two it is.
          if (member.Name == "additionalProperties")
          {
            if (member.Value.ValueKind != JsonValueKind.False)
            {
              Report(member.Name);
              Schema(member.Value);
            }

            continue;
          }

          if (!Understood.Contains(member.Name))
          {
            Report(member.Name);
          }
        }

        // Absent, `additionalProperties` defaults to admitting anything, and a generated class
        // admits nothing it did not declare. Reported for an object, because for anything else
        // the keyword says nothing at all.
        if (IsObject(element) && !element.TryGetProperty("additionalProperties",
                                                         out _))
        {
          Report("additionalProperties");
        }
      }

      static bool IsObject(JsonElement element)
        => element.TryGetProperty("properties",
                                  out _)
           || (element.TryGetProperty("type",
                                      out var type)
               && type.ValueKind == JsonValueKind.String
               && type.GetString() == "object");
    }

    // Keywords whose value holds one subschema per member, and which this generator reads.
    private static readonly HashSet<string> SubschemaMaps = new(StringComparer.Ordinal)
                                                            {
                                                              "properties",
                                                              "$defs",
                                                            };

    // The same shape, for keywords it does not read. Reported as well as walked, so the keyword
    // is named and so is anything unhandled inside it - the outer report alone would say that
    // something was dropped without saying what.
    private static readonly HashSet<string> ReportedMaps = new(StringComparer.Ordinal)
                                                           {
                                                             "patternProperties",
                                                             "dependentSchemas",
                                                           };

    // Keywords whose value is an array of subschemas. Reported as well as walked: each is a
    // composition this generator turns into no C#, and the subschemas are walked so that what is
    // inside one is reported too.
    private static readonly HashSet<string> SubschemaLists = new(StringComparer.Ordinal)
                                                             {
                                                               "allOf",
                                                               "anyOf",
                                                               "oneOf",
                                                               "prefixItems",
                                                             };

    // What a schema may say that this generator either reads or can ignore without losing a
    // constraint: the types and names it emits from, the documentation it carries over, and the
    // annotations that assert nothing. `format` is read for the C# type. `default` states what an
    // absent option means, which is the engine's business and not the class's.
    private static readonly HashSet<string> Understood = new(StringComparer.Ordinal)
                                                         {
                                                           "$anchor",
                                                           "$comment",
                                                           "$id",
                                                           "$ref",
                                                           "$schema",
                                                           "default",
                                                           "deprecated",
                                                           "description",
                                                           "examples",
                                                           "exclusiveMaximum",
                                                           "exclusiveMinimum",
                                                           "format",
                                                           "maxLength",
                                                           "maximum",
                                                           "minLength",
                                                           "minimum",
                                                           "readOnly",
                                                           "title",
                                                           "type",
                                                           "writeOnly",
                                                         };

    private static string NameOf(TypeDeclaration declaration,
                                 bool isRoot)
    {
      var title = Keyword(declaration,
                          "title")
                  ?.GetString();

      if (!string.IsNullOrEmpty(title))
      {
        return title!;
      }

      // No title: the location's last segment, which for a `$defs` entry is the name the document
      // gave it. That is what makes `#/$defs/TransportOptions` the class `TransportOptions`. The
      // root has no such segment - its location is the resolver's own URI, whose last segment
      // names this generator's plumbing rather than anything in the schema.
      if (isRoot)
      {
        throw new NotSupportedException("the schema states no `title`, which is what names the class it generates.");
      }

      var location = declaration.LocatedSchema.Location.ToString();
      var segment = location.Split('/')
                            .LastOrDefault();

      return string.IsNullOrEmpty(segment)
               ? throw new NotSupportedException($"`{location}` has no title and no name to take one from.")
               : segment!;
    }

    /// <summary>The description a node states, which it has to state.</summary>
    /// <param name="description">What the schema says, or null where it says nothing.</param>
    /// <param name="what">What the node is, for the message.</param>
    /// <returns>The description, never empty.</returns>
    /// <exception cref="NotSupportedException">There is none, or it is empty.</exception>
    /// <remarks>
    ///   An option's documentation is the schema's to carry: it is written once as a doc comment
    ///   on a Rust field and lands in a .NET caller's tooltip. Absent, a caller has to read the
    ///   engine's source to learn what an option does; empty, the doc comment exists and says
    ///   nothing. Both are refused rather than generated as a property with no documentation.
    /// </remarks>
    private static string Described(string? description,
                                    string what)
      => string.IsNullOrWhiteSpace(description)
           ? throw new NotSupportedException($"{what} states no `description`, and every option and group of this vocabulary documents itself.")
           : description!;

    private static string? Description(TypeDeclaration declaration)
      => Keyword(declaration,
                 "description")
        ?.GetString();

    private static JsonElement? Keyword(TypeDeclaration declaration,
                                        string keyword)
      => declaration.LocatedSchema.Schema.ValueKind == JsonValueKind.Object
         && declaration.LocatedSchema.Schema.TryGetProperty(keyword,
                                                            out var value)
           ? value
           : null;
  }
}
