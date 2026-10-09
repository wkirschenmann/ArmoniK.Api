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
  /// <summary>What an option holds, which decides how it is copied, checked, bound and written.</summary>
  internal enum OptionKind
  {
    /// <summary>A number, a text or a flag.</summary>
    Value,

    /// <summary>One of the generated classes.</summary>
    Group,

    /// <summary>One of the generated records, which an alternative holds.</summary>
    Record,

    /// <summary>One of the generated closed hierarchies of records.</summary>
    Choice,

    /// <summary>One of the generated enums.</summary>
    Enumeration,

    /// <summary>A list of the names of one of the generated enums, or of text, in the order stated.</summary>
    EnumerationList,
  }

  /// <summary>What the schema says an option is, once its references are resolved.</summary>
  internal sealed class Option
  {
    /// <summary>The name the document spells, which is also the C# name.</summary>
    public string Name { get; init; } = string.Empty;

    /// <summary>The `description` that applies here, which the schema has to state.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>The C# type of the property, nullable form excluded.</summary>
    public string Type { get; init; } = string.Empty;

    /// <summary>What <see cref="Type" /> is.</summary>
    public OptionKind Kind { get; init; }

    /// <summary>Whether the schema requires it, which only a field of an alternative may be.</summary>
    public bool Required { get; init; }

    /// <summary>Whether the schema marks it `writeOnly`: a secret, which nothing renders prints.</summary>
    public bool Secret { get; init; }

    /// <summary>The keywords that bound the value, empty where the schema bounds nothing.</summary>
    public IReadOnlyList<Bound> Bounds { get; init; } = Array.Empty<Bound>();
  }

  /// <summary>One bound the schema states.</summary>
  /// <param name="Keyword">The schema keyword, verbatim.</param>
  /// <param name="Literal">Its value, as C# writes that number.</param>
  internal readonly record struct Bound(string Keyword,
                                        string Literal);

  /// <summary>A type the vocabulary declares, which is rendered once, under its name.</summary>
  internal abstract class OptionType
  {
    /// <summary>The type name: the schema's `title`, or the `$defs` entry's own name.</summary>
    public string Name { get; init; } = string.Empty;

    /// <summary>The `description` of the type, which the schema has to state.</summary>
    public string Description { get; init; } = string.Empty;
  }

  /// <summary>A group of options, which is one generated class.</summary>
  internal sealed class OptionGroup : OptionType
  {
    /// <summary>The options of the group, in the order the schema states them.</summary>
    public IReadOnlyList<Option> Options { get; init; } = Array.Empty<Option>();
  }

  /// <summary>
  ///   A group with a mandatory option, which is stated whole: one generated positional record, as an
  ///   immutable value an alternative holds.
  /// </summary>
  internal sealed class OptionRecord : OptionType
  {
    /// <summary>The options of the record, in the order the schema states them.</summary>
    public IReadOnlyList<Option> Options { get; init; } = Array.Empty<Option>();
  }

  /// <summary>
  ///   Alternatives that exclude one another, which are one generated closed hierarchy of records:
  ///   a value is exactly one of them, as a Rust enum is.
  /// </summary>
  internal sealed class OptionChoice : OptionType
  {
    /// <summary>The alternatives, in the order the schema states them.</summary>
    public IReadOnlyList<Alternative> Alternatives { get; init; } = Array.Empty<Alternative>();
  }

  /// <summary>What the key naming an alternative holds.</summary>
  internal enum AlternativeShape
  {
    /// <summary>The name alone, as a string, for an alternative that carries nothing.</summary>
    Unit,

    /// <summary>One value under the alternative's key, which C# names `Value`.</summary>
    Value,

    /// <summary>An object of fields under the alternative's key.</summary>
    Fields,
  }

  /// <summary>One alternative of a choice, which is one sealed record.</summary>
  internal sealed class Alternative
  {
    /// <summary>The key that names it, which is also the record's name.</summary>
    public string Name { get; init; } = string.Empty;

    /// <summary>The `description` of the alternative, which the schema has to state.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>What its key holds.</summary>
    public AlternativeShape Shape { get; init; }

    /// <summary>The record's parameters: none for a unit, `Value` alone for a value.</summary>
    public IReadOnlyList<Option> Fields { get; init; } = Array.Empty<Option>();
  }

  /// <summary>The names a value may be, which is one generated enum.</summary>
  internal sealed class OptionEnumeration : OptionType
  {
    /// <summary>The names, in the order the schema states them.</summary>
    public IReadOnlyList<Member> Members { get; init; } = Array.Empty<Member>();
  }

  /// <summary>One name an enumeration admits.</summary>
  /// <param name="Name">The name, as the document spells it and as C# declares it.</param>
  /// <param name="Description">The `description` of the name, which the schema has to state.</param>
  internal readonly record struct Member(string Name,
                                         string Description);

  /// <summary>
  ///   The option vocabulary a schema describes: every type, the root group first.
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

    /// <summary>Reads <paramref name="schemaJson" /> and returns the types it describes.</summary>
    /// <param name="schemaJson">A JSON schema, draft 2020-12.</param>
    /// <returns>The root group first, then every type it reaches, each once.</returns>
    /// <exception cref="NotSupportedException">
    ///   The schema uses a construct this generator does not turn into C#. Refusing is what keeps
    ///   a new option from being emitted as something that compiles and means nothing.
    /// </exception>
    public static async Task<IReadOnlyList<OptionType>> ReadAsync(string schemaJson)
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

      // The root is what the engine reads as one document, and a document is an object.
      if (IsChoice(root))
      {
        throw new NotSupportedException("the schema's root is a choice, and the document the engine reads is a group of options.");
      }

      var types = new List<OptionType>();
      var read = new HashSet<string>(StringComparer.Ordinal);

      Read(root,
           NameOf(root,
                  true),
           types,
           read);

      return types;
    }

    /// <summary>
    ///   The types of <paramref name="groups" /> that <paramref name="reused" /> does not hold: a
    ///   type of the same name there is one type, rendered from that other schema.
    /// </summary>
    /// <exception cref="NotSupportedException">
    ///   The root is held there, or a type held there differs from it - two types of one name,
    ///   one of which would not be what its schema states - or no type is held there, which is a
    ///   schema that reuses nothing and would declare again the types it was meant to reuse.
    /// </exception>
    public static IReadOnlyList<OptionType> Without(IReadOnlyList<OptionType> groups,
                                                    IReadOnlyList<OptionType> reused)
    {
      var held = reused.ToDictionary(group => group.Name,
                                     StringComparer.Ordinal);

      if (held.ContainsKey(groups[0].Name))
      {
        throw new NotSupportedException($"`{groups[0].Name}` is the root, and the reused schema renders it already.");
      }

      var kept = new List<OptionType>();
      foreach (var group in groups)
      {
        if (!held.TryGetValue(group.Name,
                              out var other))
        {
          kept.Add(group);
        }
        else if (!Same(group,
                       other))
        {
          throw new NotSupportedException($"`{group.Name}` differs from the type of that name the reused schema renders.");
        }
      }

      if (kept.Count == groups.Count)
      {
        throw new NotSupportedException("the reused schema holds none of these types, so nothing is reused.");
      }

      return kept;
    }

    private static bool Same(OptionType one,
                             OptionType other)
      => one.Description == other.Description && (one, other) switch
                                                 {
                                                   (OptionGroup mine, OptionGroup theirs) => Same(mine.Options,
                                                                                                  theirs.Options),
                                                   (OptionRecord mine, OptionRecord theirs) => Same(mine.Options,
                                                                                                    theirs.Options),
                                                   (OptionChoice mine, OptionChoice theirs) => mine.Alternatives.Count == theirs.Alternatives.Count &&
                                                                                               mine.Alternatives.Zip(theirs.Alternatives,
                                                                                                                     (a, b) => a.Name        == b.Name        &&
                                                                                                                               a.Description == b.Description &&
                                                                                                                               a.Shape       == b.Shape       && Same(a.Fields,
                                                                                                                                                                      b.Fields))
                                                                                                   .All(same => same),
                                                   (OptionEnumeration mine, OptionEnumeration theirs) => mine.Members.SequenceEqual(theirs.Members),
                                                   _                                                  => false,
                                                 };

    private static bool Same(IReadOnlyList<Option> mine,
                             IReadOnlyList<Option> theirs)
      => mine.Count == theirs.Count && mine.Zip(theirs,
                                                (one, other) => one.Name == other.Name && one.Type == other.Type && one.Kind == other.Kind &&
                                                                one.Required == other.Required && one.Secret == other.Secret && one.Description == other.Description &&
                                                                one.Bounds.SequenceEqual(other.Bounds))
                                           .All(same => same);

    // Depth first, and a type is read once: the root is emitted first, and a type reached twice
    // is one declaration rather than two.
    private static void Read(TypeDeclaration declaration,
                             string name,
                             List<OptionType> types,
                             HashSet<string> read)
    {
      if (!read.Add(declaration.LocatedSchema.Location.ToString()))
      {
        return;
      }

      // Two schemas of one name would be one type declared twice, which the C# compiler reports
      // as a duplicate member in a generated file nobody wrote.
      if (types.Any(type => type.Name == name))
      {
        throw new NotSupportedException($"`{declaration.LocatedSchema.Location}` is named `{name}`, which another schema already is.");
      }

      var nested = new List<(TypeDeclaration Declaration, string Name)>();

      // The root is the document, a group of options all optional; below it, a type with a
      // mandatory option is stated whole.
      types.Add(IsChoice(declaration)
                  ? ReadChoice(declaration,
                               name,
                               nested)
                  : types.Count > 0 && HasRequired(declaration)
                    ? ReadRecord(declaration,
                                 name,
                                 nested)
                    : ReadGroup(declaration,
                                name,
                                nested));

      foreach (var (subdeclaration, subname) in nested)
      {
        Read(subdeclaration,
             subname,
             types,
             read);
      }
    }

    private static bool HasRequired(TypeDeclaration declaration)
      => declaration.HasPropertyDeclarations && declaration.PropertyDeclarations.Any(property => property.RequiredOrOptional != RequiredOrOptional.Optional);

    // A record is a value a caller states whole, and an immutable one, so what it holds is a value
    // too: a number, a text or a flag.
    private static OptionRecord ReadRecord(TypeDeclaration declaration,
                                           string name,
                                           List<(TypeDeclaration Declaration, string Name)> nested)
    {
      var options = Declared(declaration)
                    .Select(property => ReadOption(property,
                                                   nested,
                                                   false))
                    .ToList();

      if (options.FirstOrDefault(option => option.Kind != OptionKind.Value) is { } held)
      {
        throw new NotSupportedException($"`{name}` holds `{held.Name}`, which is not a number, a text or a flag: a record this generator renders holds values only.");
      }

      return new OptionRecord
             {
               Name        = name,
               Description = Described(Description(declaration),
                                       $"`{name}`"),
               Options     = options,
             };
    }

    private static OptionGroup ReadGroup(TypeDeclaration declaration,
                                         string name,
                                         List<(TypeDeclaration Declaration, string Name)> nested)
    {
      // An object stating no properties is a class with nothing in it, and whatever it was meant
      // to carry - an open map, a shape stated some other way - a generated property cannot hold.
      // Checked here rather than at each property, so the root is checked too.
      if (!declaration.HasPropertyDeclarations)
      {
        throw new NotSupportedException($"`{declaration.LocatedSchema.Location}` states no properties, which this generator has no class for.");
      }

      // A class's options are nullable and a caller sets the ones they want, so a required one
      // would be a presence nothing on this side checks.
      if (declaration.PropertyDeclarations.FirstOrDefault(property => property.RequiredOrOptional != RequiredOrOptional.Optional) is { } required)
      {
        throw new NotSupportedException($"`{name}` requires `{required.JsonPropertyName}`, and an option of a class is one a caller may leave unset.");
      }

      return new OptionGroup
             {
               Name = name,
               Description = Described(Description(declaration),
                                       $"`{name}`"),
               Options = Declared(declaration)
                         .Select(property => ReadOption(property,
                                                        nested,
                                                        true))
                         .ToList(),
             };
    }

    // An option of a group or a field of an alternative: a value, or a type of its own that is
    // read after the one holding it.
    private static Option ReadOption(PropertyDeclaration property,
                                     List<(TypeDeclaration Declaration, string Name)> nested,
                                     bool holdsGroups)
    {
      var node = property.ReducedPropertyType;
      var target = Resolve(node);

      // The property's own, then the one its reference states: a `$defs` entry describes what it
      // is, and a property describes what it is for here. Read unreduced, because a property
      // that states nothing beside its `$ref` but a description reduces to what it references.
      var description = Described(Description(property.UnreducedPropertyType) ?? Description(target),
                                  $"`{property.JsonPropertyName}`");

      var required = property.RequiredOrOptional != RequiredOrOptional.Optional;

      // A constant is read where it names an alternative that carries nothing; anywhere else it would be a
      // constraint the engine enforces and the C# lets through.
      if (Keyword(target,
                  "const") is not null)
      {
        throw new NotSupportedException($"`{property.JsonPropertyName}` states a constant, which this generator reads only as the name of an alternative that carries nothing.");
      }

      var type = Keyword(target,
                         "type")
                 ?.GetString();

      // A list of the names of an enumeration, or of text.
      if (type == "array")
      {
        return ReadList(property,
                        target,
                        description,
                        required,
                        nested);
      }

      // A choice first: Corvus composes the properties of a `oneOf`'s alternatives into the
      // choice itself, so it would also pass for an object.
      if (IsChoice(target) || type == "object" || target.HasPropertyDeclarations)
      {
        // A type is bounded by what it declares, not by the option that holds one. Written on
        // the option it would bind that one embedding and not the next, which is the opposite of
        // what a `$defs` entry is for - and the keywords this generator checks bound a number or
        // a string, so on a type they asserted nothing and were dropped.
        var bounds = BoundsOf(node,
                              target);

        if (bounds.Count > 0)
        {
          throw new NotSupportedException($"`{property.JsonPropertyName}` names a type and states {string.Join(", ", bounds.Select(bound => $"`{bound.Keyword}`"))}, which bounds no type. State it on the values the type declares.");
        }

        var kind = !IsChoice(target)
                     ? HasRequired(target)
                         ? OptionKind.Record
                         : OptionKind.Group
                     : IsEnumeration(target)
                       ? OptionKind.Enumeration
                       : OptionKind.Choice;

        // A record is immutable and a class is not, so a record holding a class would be a value
        // a caller can change through a copy that was meant to be its own.
        if (kind == OptionKind.Group && !holdsGroups)
        {
          throw new NotSupportedException($"`{property.JsonPropertyName}` is a group, and an alternative holds none: a record is immutable, and a class in it would not be.");
        }

        // A class is written by the serializer's context, which is not taught to write a record.
        if (kind == OptionKind.Record && holdsGroups)
        {
          throw new NotSupportedException($"`{property.JsonPropertyName}` is stated whole, and a group holds no such type: only an alternative holds one.");
        }

        var typeName = NameOf(target,
                              false);
        nested.Add((target, typeName));

        return new Option
               {
                 Name        = property.JsonPropertyName,
                 Description = description,
                 Type        = typeName,
                 Kind        = kind,
                 Required    = required,
               };
      }

      return new Option
             {
               Name        = property.JsonPropertyName,
               Description = description,
               Type = CSharpType(type,
                                 Keyword(target,
                                         "format")
                                   ?.GetString(),
                                 property.JsonPropertyName),
               Required = required,
               Secret   = IsSecret(property.UnreducedPropertyType) || IsSecret(target),
               Bounds = BoundsOf(node,
                                 target),
             };
    }

    // An array of the names of an enumeration, or of text, is a settable list. Any other array is
    // refused: a list of numbers or of groups has bounds and copies this generator does not write,
    // and one that states none of its items is a shape nothing here can type.
    private static Option ReadList(PropertyDeclaration property,
                                   TypeDeclaration target,
                                   string description,
                                   bool required,
                                   List<(TypeDeclaration Declaration, string Name)> nested)
    {
      var name = property.JsonPropertyName;

      // A secret is printed as whether it is set, which a list has no way to say.
      if (IsSecret(property.UnreducedPropertyType) || IsSecret(target))
      {
        throw new NotSupportedException($"`{name}` is a secret list, which this generator has no way to print elided.");
      }

      var items = target.ArrayItemsType();

      if (items is null)
      {
        throw new NotSupportedException($"`{name}` is an array that states no `items`, and this generator types a list by its items.");
      }

      var item = Resolve(items.ReducedType);

      var text = IsText(item);

      if (!text && (!IsChoice(item) || !IsEnumeration(item)))
      {
        throw new NotSupportedException($"`{name}` is a list of something other than the names of an enumeration or text, which this generator has no property for.");
      }

      // The count of a list this generator checks is that of the names a record holds.
      if (BoundsOf(property.ReducedPropertyType,
                   target) is { Count: > 0 } bounds)
      {
        throw new NotSupportedException($"`{name}` is a list and states {string.Join(", ", bounds.Select(bound => $"`{bound.Keyword}`"))}, which this generator checks only on the names an alternative holds.");
      }

      string typeName;

      if (text)
      {
        typeName = "string";
      }
      else
      {
        typeName = NameOf(item,
                          false);
        nested.Add((item, typeName));
      }

      return new Option
             {
               Name        = name,
               Description = description,
               Type        = typeName,
               Kind        = OptionKind.EnumerationList,
               Required    = required,
             };
    }

    // What an alternative that is a list holds: the names of an enumeration, as `Value`.
    private static Option ReadNames(TypeDeclaration payload,
                                    TypeDeclaration node,
                                    string description,
                                    string what,
                                    List<(TypeDeclaration Declaration, string Name)> nested)
    {
      var items = payload.ArrayItemsType();

      if (items is null)
      {
        throw new NotSupportedException($"`{what}` is an array that states no `items`, and this generator types a list by its items.");
      }

      var item = Resolve(items.ReducedType);

      if (!IsChoice(item) || !IsEnumeration(item))
      {
        throw new NotSupportedException($"`{what}` is a list of something other than the names of an enumeration, which this generator has no list for.");
      }

      var typeName = NameOf(item,
                            false);
      nested.Add((item, typeName));

      return new Option
             {
               Name        = "Value",
               Description = description,
               Type        = typeName,
               Kind        = OptionKind.EnumerationList,
               Required    = true,
               Bounds = BoundsOf(node,
                                 payload),
             };
    }

    // Text with no constraint of its own: what a list of entries is made of when the engine, and
    // not the schema, says which entries are admissible.
    private static bool IsText(TypeDeclaration declaration)
      => !IsChoice(declaration) &&
         Keyword(declaration,
                 "type")
           ?.GetString() == "string" &&
         Keyword(declaration,
                 "format") is null &&
         Keyword(declaration,
                 "const") is null &&
         BoundsOf(declaration,
                  declaration).Count == 0;

    private static bool IsSecret(TypeDeclaration declaration)
      => Keyword(declaration,
                 "writeOnly") is { ValueKind: JsonValueKind.True };

    // Corvus returns the properties sorted by name, and the schema lists them as the Rust type
    // declares them: that is the order a caller reads them in, and the one a record takes them in,
    // where the name order would put a password before its username.
    private static IEnumerable<PropertyDeclaration> Declared(TypeDeclaration declaration)
    {
      var order = Keyword(declaration,
                          "properties") is { ValueKind: JsonValueKind.Object } properties
                    ? properties.EnumerateObject()
                                .Select(property => property.Name)
                                .ToList()
                    : new List<string>();

      return declaration.PropertyDeclarations.OrderBy(property => order.IndexOf(property.JsonPropertyName) is var at and >= 0
                                                                    ? at
                                                                    : int.MaxValue);
    }

    // A `oneOf` is how a Rust enum renders: names alone, or alternatives each tagged by its key.
    private static bool IsChoice(TypeDeclaration declaration)
      => Keyword(declaration,
                 "oneOf") is { ValueKind: JsonValueKind.Array };

    // An enum of unit variants renders each as a constant. One variant carrying anything renders
    // the others as constants too, beside objects of one key.
    private static bool IsEnumeration(TypeDeclaration declaration)
      => Keyword(declaration,
                 "oneOf")!.Value.EnumerateArray()
                          .All(branch => branch.ValueKind == JsonValueKind.Object && branch.TryGetProperty("const",
                                                                                                           out _));

    private static OptionType ReadChoice(TypeDeclaration declaration,
                                         string name,
                                         List<(TypeDeclaration Declaration, string Name)> nested)
    {
      var description = Described(Description(declaration),
                                  $"`{name}`");

      // Null where Corvus composes no alternative, which the count below then refuses.
      var branches = (declaration.OneOfCompositionTypes() ?? new Dictionary<IOneOfSubschemaValidationKeyword, IReadOnlyCollection<TypeDeclaration>>())
                     .SelectMany(keyword => keyword.Value)
                     .ToList();

      if (branches.Count != Keyword(declaration,
                                    "oneOf")!.Value.GetArrayLength())
      {
        throw new NotSupportedException($"`{name}` states alternatives this generator cannot read one for one.");
      }

      // An empty `oneOf` admits no value, and would render an enum or a record nothing can be.
      if (branches.Count == 0)
      {
        throw new NotSupportedException($"`{name}` states no alternative, so no value is one.");
      }

      if (IsEnumeration(declaration))
      {
        return new OptionEnumeration
               {
                 Name        = name,
                 Description = description,
                 Members = branches.Select(branch =>
                                           {
                                             var constant = Keyword(branch,
                                                                    "const");

                                             if (constant is not { ValueKind: JsonValueKind.String })
                                             {
                                               throw new NotSupportedException($"`{name}` admits a constant that is not a name, which no C# enum declares.");
                                             }

                                             var member = constant.Value.GetString()!;

                                             return new Member(member,
                                                               Described(Description(branch),
                                                                         $"`{name}.{member}`"));
                                           })
                                   .ToList(),
               };
      }

      return new OptionChoice
             {
               Name        = name,
               Description = description,
               Alternatives = branches.Select(branch => ReadAlternative(branch,
                                                                        name,
                                                                        nested))
                                      .ToList(),
             };
    }

    // An alternative is the name of one that carries nothing, as a string constant, or an object of
    // one key, required and alone, which names it and holds a value or an object of fields.
    private static Alternative ReadAlternative(TypeDeclaration branch,
                                               string choice,
                                               List<(TypeDeclaration Declaration, string Name)> nested)
    {
      if (Keyword(branch,
                  "const") is { } named)
      {
        return named.ValueKind == JsonValueKind.String
                 ? new Alternative
                   {
                     Name        = named.GetString()!,
                     Description = Described(Description(branch),
                                             $"`{choice}.{named.GetString()}`"),
                     Shape       = AlternativeShape.Unit,
                   }
                 : throw new NotSupportedException($"An alternative of `{choice}` is a constant that is not a name, which no C# record is named for.");
      }

      var properties = branch.HasPropertyDeclarations
                         ? branch.PropertyDeclarations.ToList()
                         : new List<PropertyDeclaration>();

      if (properties.Count != 1 || properties[0].RequiredOrOptional == RequiredOrOptional.Optional)
      {
        throw new NotSupportedException($"An alternative of `{choice}` is not an object of one required key, which is how an alternative names itself.");
      }

      var property = properties[0];
      var name = property.JsonPropertyName;
      var description = Described(Description(branch),
                                  $"`{choice}.{name}`");
      var node = property.ReducedPropertyType;
      var payload = Resolve(node);

      if (Keyword(payload,
                  "const") is not null)
      {
        throw new NotSupportedException($"`{choice}.{name}` is a constant under a key, and an alternative that carries nothing is written as its name alone.");
      }

      if (IsChoice(payload))
      {
        throw new NotSupportedException($"`{choice}.{name}` carries a choice bare, which this generator has no record for. Give it a field.");
      }

      var type = Keyword(payload,
                         "type")
                 ?.GetString();

      // The names of an enumeration, bare: an alternative that is a list, a value of its own, copied
      // and compared by what it names when the record is made.
      if (type == "array")
      {
        return new Alternative
               {
                 Name        = name,
                 Description = description,
                 Shape       = AlternativeShape.Value,
                 Fields = new[]
                          {
                            ReadNames(payload,
                                      node,
                                      description,
                                      $"{choice}.{name}",
                                      nested),
                          },
               };
      }

      if (type == "object" || payload.HasPropertyDeclarations)
      {
        return new Alternative
               {
                 Name        = name,
                 Description = description,
                 Shape       = AlternativeShape.Fields,
                 Fields = Declared(payload)
                          .Select(field => ReadOption(field,
                                                      nested,
                                                      false))
                          .ToList(),
               };
      }

      return new Alternative
             {
               Name        = name,
               Description = description,
               Shape       = AlternativeShape.Value,
               Fields = new[]
                        {
                          new Option
                          {
                            Name        = "Value",
                            Description = description,
                            Type = CSharpType(type,
                                              Keyword(payload,
                                                      "format")
                                                ?.GetString(),
                                              $"{choice}.{name}"),
                            Required = true,
                            Secret   = IsSecret(node) || IsSecret(payload),
                            Bounds = BoundsOf(node,
                                              payload),
                          },
                        },
             };
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

      // Followed through what a target contains, not only through what it is. An edge is recorded
      // at the node holding the `$ref` - `#/$defs/A/properties/B` - so a walk that looked the
      // target up as a key would find a step only where the target is itself a bare `$ref`. That
      // is the rare shape. The ordinary one is a type that reaches itself through its own
      // properties, which is what a recursive schema has.
      foreach (var (from, _) in references)
      {
        Follow(from,
               new HashSet<string>(StringComparer.Ordinal));
      }

      void Follow(string from,
                  ISet<string> seen)
      {
        var target = references[from];

        if (!seen.Add(target))
        {
          throw new NotSupportedException($"`{from}` is reached by a cycle of `$ref` through `{target}`, which names no type.");
        }

        foreach (var under in references.Keys.Where(held => held == target || held.StartsWith(target + "/", StringComparison.Ordinal))
                                        .ToList())
        {
          Follow(under,
                 seen);
        }

        seen.Remove(target);
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
      "minItems",
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
      var lower = keyword is "minimum" or "exclusiveMinimum" or "minLength" or "minItems";

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

          if (SubschemaLists.Contains(member.Name) || ReadLists.Contains(member.Name))
          {
            if (member.Value.ValueKind == JsonValueKind.Array)
            {
              foreach (var subschema in member.Value.EnumerateArray())
              {
                Schema(subschema);
              }
            }

            if (SubschemaLists.Contains(member.Name))
            {
              Report(member.Name);
            }

            continue;
          }

          // The one subschema of a list, which `ReadList` reads: what is unhandled inside it is
          // named like anything else.
          if (member.Name == "items")
          {
            Schema(member.Value);
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
                                                               "prefixItems",
                                                             };

    // The same shape, for the one composition this generator reads: `oneOf` is a choice or an
    // enumeration, and `ReadChoice` refuses any other shape of it.
    private static readonly HashSet<string> ReadLists = new(StringComparer.Ordinal)
                                                        {
                                                          "oneOf",
                                                        };

    // What a schema may say that this generator either reads or can ignore without losing a
    // constraint: the types and names it emits from, the documentation it carries over, and the
    // annotations that assert nothing. `format` is read for the C# type; `const` and `required`
    // for a choice, and the reading refuses them anywhere else - a constant outside an
    // alternative, a required option of a class. `default` is not here: a default is stated in
    // its option's description, and applying it is the engine's.
    private static readonly HashSet<string> Understood = new(StringComparer.Ordinal)
                                                         {
                                                           "$anchor",
                                                           "$comment",
                                                           "$id",
                                                           "$ref",
                                                           "$schema",
                                                           "const",
                                                           "deprecated",
                                                           "description",
                                                           "examples",
                                                           "exclusiveMaximum",
                                                           "exclusiveMinimum",
                                                           "format",
                                                           "maxLength",
                                                           "maximum",
                                                           "minItems",
                                                           "minLength",
                                                           "minimum",
                                                           "readOnly",
                                                           "required",
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
