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
using System.CodeDom.Compiler;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;

namespace ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator
{
  /// <summary>The option vocabulary as the C# a caller fills in.</summary>
  internal static class CSharpSource
  {
    private const string Indent = "  ";

    // The repository's own header, which every file it tracks carries. Emitted because this file
    // is tracked like any other, and because a tool asked to add one would edit a file that then
    // stops being what the schema renders - which the build refuses.
    private const string Licence = @"// This file is part of the ArmoniK project
//
// Copyright (C) ANEO, 2021-2026. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the ""License"")
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an ""AS IS"" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
";

    /// <summary>Renders <paramref name="types" /> as one compilation unit.</summary>
    /// <param name="types">The vocabulary, its root group first.</param>
    /// <param name="namespaceName">The namespace the types are declared in.</param>
    /// <param name="schemaName">The schema file, named in the header so a reader can find it.</param>
    /// <param name="document">
    ///   Whether the engine reads the vocabulary as a JSON document, which the root then encodes.
    ///   One it takes as fields is rendered without the encoding and its serializer context.
    /// </param>
    /// <returns>The file's whole text, ending with one newline.</returns>
    /// <exception cref="NotSupportedException">A name or a bound has no C# this can emit.</exception>
    public static string Render(IReadOnlyList<OptionType> types,
                                string namespaceName,
                                string schemaName,
                                bool   document = true)
    {
      var root = types[0]
        .Name;

      // Ahead of any rendering: a name reaches identifiers, `nameof`s and string literals, and
      // one that is not a C# name would be spliced into all of them.
      var context = root + "JsonContext";
      var reading = root + "Configuration";

      // Declared beside the vocabulary and named after the root or a choice, so a type of one of
      // these names would be two types of one name in a file nobody wrote.
      var beside = new HashSet<string>(StringComparer.Ordinal)
                   {
                     reading,
                   };

      foreach (var choice in types.OfType<OptionChoice>())
      {
        beside.Add(Converter(choice.Name));
      }

      var named = new HashSet<string>(types.Select(type => type.Name),
                                      StringComparer.Ordinal);

      foreach (var type in types)
      {
        RefuseAName(type.Name,
                    "a schema");

        if (document && type.Name == context)
        {
          throw new NotSupportedException($"`{type.Name}` names a schema, and it is also the serializer context this renders for `{root}`.");
        }

        if (beside.Contains(type.Name))
        {
          throw new NotSupportedException($"`{type.Name}` names a schema, and it is also a type this renders beside the vocabulary.");
        }

        switch (type)
        {
          case OptionGroup group:
            RefuseTwins(group.Options.Select(option => option.Name),
                        $"options of `{group.Name}`");
            RefuseNames(group.Options,
                        group.Name,
                        $"an option of `{group.Name}`",
                        "an option of the group of the same name, and C# admits no member named after its own class");
            break;

          case OptionChoice choice:
            RefuseTwins(choice.Alternatives.Select(alternative => alternative.Name),
                        $"alternatives of `{choice.Name}`");

            foreach (var alternative in choice.Alternatives)
            {
              RefuseAName(alternative.Name,
                          $"an alternative of `{choice.Name}`");

              // Inside the choice, a nested record hides a type of the same name, and a field of
              // that type would name the record instead.
              if (named.Contains(alternative.Name))
              {
                throw new NotSupportedException($"`{alternative.Name}` is an alternative of `{choice.Name}`, and also a type of the vocabulary, which it would hide.");
              }

              RefuseTwins(alternative.Fields.Select(field => field.Name),
                          $"fields of `{choice.Name}.{alternative.Name}`");

              if (alternative.Name == choice.Name)
              {
                throw new NotSupportedException($"`{alternative.Name}` is an alternative of the choice of the same name, and C# admits no member named after its own type.");
              }

              RefuseNames(alternative.Fields,
                          alternative.Name,
                          $"a field of `{choice.Name}.{alternative.Name}`",
                          "a field of the alternative of the same name, and C# admits no member named after its own record");

              // The binding and the writing of an alternative declare locals of their own beside
              // one per field and one for the alternative, and two locals of one name do not compile.
              foreach (var name in alternative.Fields.Select(field => field.Name)
                                              .Append(alternative.Name))
              {
                if (Plumbing.Contains(Local(name)))
                {
                  throw new NotSupportedException($"`{name}` is in `{choice.Name}`, and its local would be one the rendered binding or writing declares already.");
                }
              }
            }

            break;

          case OptionEnumeration enumeration:
            RefuseTwins(enumeration.Members.Select(member => member.Name),
                        $"names of `{enumeration.Name}`");

            foreach (var member in enumeration.Members)
            {
              RefuseAName(member.Name,
                          $"a name of `{enumeration.Name}`");
            }

            break;
        }
      }

      using var text = new StringWriter
                       {
                         NewLine = "\n",
                       };
      var source = new IndentedTextWriter(text,
                                          Indent);

      var writes = document || types.OfType<OptionChoice>()
                                    .Any();
      var used = Used(types);
      var parses = used.Overlaps(new[]
                                 {
                                   "Int32",
                                   "Int64",
                                   "Double",
                                 });

      // The marker leads, and the licence follows it. A tool recognises generated code by finding
      // `<auto-generated>` in the file's first comment, so a licence header above it would hide
      // the file's own nature; below it, the header is still a header.
      Lines(source,
            $$"""
              // <auto-generated>
              //     Generated from {{schemaName}} by ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator.
              //     Edits here are lost. Change the Rust option types, render the schema again,
              //     and build: the schema is what this file is.
              // </auto-generated>

              {{Licence.TrimEnd('\r', '\n')}}

              #nullable enable

              using System;
              {{(parses ? "using System.Globalization;\n" : "")}}using System.Linq;
              {{(writes ? "using System.Text.Json;\n" : "")}}using System.Text.Json.Serialization;

              using Microsoft.Extensions.Configuration;

              namespace {{namespaceName}};
              """);

      // The context is rooted at the first group, which reaches every other type: it is what
      // serializes without reflection, which is what lets a trimmed or native-AOT host send a
      // document.
      if (document)
      {
        Lines(source,
              $$"""

                // Rooted at {{root}}, which reaches every group of the vocabulary, so the whole graph is
                // serialized without reflection - which is what lets a trimmed or native-AOT host use this.
                [JsonSerializable(typeof({{root}}))]
                internal partial class {{context}} : JsonSerializerContext
                {
                }
                """);
      }

      foreach (var type in types)
      {
        Blank(source);

        switch (type)
        {
          case OptionGroup group:
            AppendGroup(source,
                        group,
                        document && ReferenceEquals(group,
                                                    types[0]),
                        reading);
            break;

          case OptionChoice choice:
            AppendChoice(source,
                         choice,
                         reading);
            break;

          case OptionEnumeration enumeration:
            AppendEnumeration(source,
                              enumeration);
            break;
        }
      }

      Blank(source);
      AppendReading(source,
                    reading,
                    used);

      return text.ToString();
    }

    private static string Converter(string choice)
      => choice + "JsonConverter";

    // The locals a rendered `Bind` or `WriteValue` declares for itself.
    private static readonly HashSet<string> Plumbing = new(StringComparer.Ordinal)
                                                       {
                                                         "alternative",
                                                         "entry",
                                                         "section",
                                                         "writer",
                                                         "written",
                                                       };

    // A configuration matches keys without case, so two names that differ only in case would be
    // one key there, and the binding would read whichever it tests first.
    private static void RefuseTwins(IEnumerable<string> names,
                                    string what)
    {
      var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

      foreach (var name in names)
      {
        if (!seen.Add(name))
        {
          throw new NotSupportedException($"`{name}` is one of two {what} that differ at most in case, which a configuration reads as one key.");
        }
      }
    }

    // C# refuses a member named after the type that declares it, and the message it gives names a
    // file nobody wrote.
    private static void RefuseNames(IEnumerable<Option> options,
                                    string owner,
                                    string what,
                                    string same)
    {
      foreach (var option in options)
      {
        RefuseAName(option.Name,
                    what);

        if (option.Name == owner)
        {
          throw new NotSupportedException($"`{option.Name}` is {same}.");
        }
      }
    }

    // The type of a group's property, nullable form excluded: a list is a settable one.
    private static string PropertyType(Option option)
      => option.Kind == OptionKind.EnumerationList
           ? ListOf(option.Type)
           : option.Type;

    private static string ListOf(string type)
      => $"global::System.Collections.Generic.List<{type}>";

    private static void AppendGroup(IndentedTextWriter source,
                                    OptionGroup group,
                                    bool encodes,
                                    string reading)
    {
      Document(source,
               group.Description);

      // Public, because this is what a caller fills in. Not partial: the whole surface is
      // rendered here - the properties, the bounds, the copy, the binding and the encoding - so a
      // second part would be something the schema does not decide, and the place for that is
      // this generator.
      source.WriteLine($"public sealed class {group.Name}");
      source.WriteLine("{");
      source.Indent++;

      AppendConstructors(source,
                         group);

      for (var n = 0; n < group.Options.Count; n++)
      {
        var option = group.Options[n];
        if (n > 0)
        {
          Blank(source);
        }

        Document(source,
                 option.Description);

        // The name is spelled rather than left to the serializer's policy: this document is a
        // contract with the engine, which refuses an option it does not know rather than
        // ignoring it, and a policy set elsewhere would be enough to break it.
        Lines(source,
              $$"""
                [JsonPropertyName("{{option.Name}}")]
                [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
                public {{PropertyType(option)}}? {{option.Name}} { get; set; }
                """);
      }

      Blank(source);
      AppendValidate(source,
                     group.Options,
                     """
                     /// <summary>Refuses an option outside the range the engine accepts.</summary>
                     /// <exception cref="ArgumentOutOfRangeException">An option is outside its stated bounds.</exception>
                     public void Validate()
                     """);

      AppendBindGroup(source,
                      group,
                      reading);

      if (encodes)
      {
        AppendEncode(source,
                     group);
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // A configuration's keys are read here rather than by `ConfigurationBinder`, which cannot make
    // a choice: its type is abstract, and which record to make is what the section's one key says.
    // Every key is matched, so a misspelt one is refused as the engine refuses it in a document.
    private static void AppendBindGroup(IndentedTextWriter source,
                                        OptionGroup group,
                                        string reading)
    {
      Lines(source,
            $$"""

              /// <summary>The options <paramref name="section" /> states, each key matched to one without case.</summary>
              /// <param name="section">The section, whose every key has to name an option.</param>
              /// <returns>The options, unset where the section states nothing.</returns>
              /// <exception cref="InvalidOperationException">A key names no option, or holds what its option does not admit.</exception>
              internal static {{group.Name}} Bind(IConfigurationSection section)
              {
                var bound = new {{group.Name}}();

                foreach (var entry in {{reading}}.Entries(section))
                {
              """);

      source.Indent += 2;

      AppendMatches(source,
                    group.Options,
                    option => $"bound.{option.Name} = {Reader(option, "entry", reading)};",
                    $"throw {reading}.Unknown(entry,\n{new string(' ', $"throw {reading}.Unknown(".Length)}\"{group.Name}\");",
                    reading);

      source.Indent -= 2;

      Lines(source,
            """
                }

                return bound;
              }
              """);
    }

    // One `if` per key, the last `else` refusing a key none of them is.
    private static void AppendMatches(IndentedTextWriter source,
                                      IReadOnlyList<Option> options,
                                      Func<Option, string> assigns,
                                      string refuses,
                                      string reading)
    {
      for (var n = 0; n < options.Count; n++)
      {
        Lines(source,
              $$"""
                {{(n == 0 ? "if" : "else if")}} ({{reading}}.Is(entry,
                {{new string(' ', (n == 0 ? "if (" : "else if (").Length + reading.Length + ".Is(".Length)}}"{{options[n].Name}}"))
                {
                  {{assigns(options[n])}}
                }
                """);
      }

      Lines(source,
            options.Count == 0
              ? refuses
              : $$"""
                  else
                  {
                    {{refuses.Replace("\n", "\n  ")}}
                  }
                  """);
    }

    // What a key's text becomes: the type's own binding for a group or a choice, the reading's for
    // the rest. A key left empty is unset, as `ConfigurationBinder` leaves it - except a text,
    // which is the empty text.
    private static string Reader(Option option,
                                 string section,
                                 string reading)
      => option.Kind switch
         {
           OptionKind.Group or OptionKind.Choice => $"{reading}.Holds({section}) ? {option.Type}.Bind({section}) : null",
           OptionKind.Enumeration                => $"{reading}.Enumeration<{option.Type}>({section})",
           OptionKind.EnumerationList            => $"{reading}.Enumerations<{option.Type}>({section})",
           _ => option.Type switch
                {
                  "string" => $"{reading}.Text({section})",
                  "int"    => $"{reading}.Int32({section})",
                  "long"   => $"{reading}.Int64({section})",
                  "double" => $"{reading}.Double({section})",
                  "bool"   => $"{reading}.Boolean({section})",
                  _        => throw new NotSupportedException($"`{option.Name}` is a `{option.Type}`, which this generator reads from no configuration."),
                },
         };

    // A choice is a closed hierarchy: an abstract record whose constructor is private, so the
    // sealed records nested in it are every alternative there is and a switch over them is
    // complete. Records, because an alternative is a value: two that say the same are equal, and
    // none changes once made, so a copy of the group holding one shares it safely.
    private static void AppendChoice(IndentedTextWriter source,
                                     OptionChoice choice,
                                     string reading)
    {
      Document(source,
               choice.Description);

      Lines(source,
            $$"""
              [JsonConverter(typeof({{Converter(choice.Name)}}))]
              public abstract record {{choice.Name}}
              {
                private {{choice.Name}}()
                {
                }
              """);

      source.Indent++;

      foreach (var alternative in choice.Alternatives)
      {
        Blank(source);
        AppendAlternative(source,
                          choice,
                          alternative);
      }

      Lines(source,
            """

            /// <summary>Refuses a field outside the range the engine accepts.</summary>
            /// <exception cref="ArgumentOutOfRangeException">A field is outside its stated bounds.</exception>
            public abstract void Validate();
            """);

      AppendBindChoice(source,
                       choice,
                       reading);

      source.Indent--;
      source.WriteLine("}");

      Blank(source);
      AppendConverter(source,
                      choice);
    }

    // Required first, so a caller writes what the alternative needs and names the rest.
    private static List<Option> Ordered(IEnumerable<Option> fields)
      => fields.OrderBy(field => field.Required
                                   ? 0
                                   : 1)
               .ToList();

    private static void AppendAlternative(IndentedTextWriter source,
                                          OptionChoice choice,
                                          Alternative alternative)
    {
      Document(source,
               alternative.Description);

      var parameters = Ordered(alternative.Fields);

      foreach (var field in parameters)
      {
        Element(source,
                "param",
                string.Join("\n",
                            Paragraphs(field.Description)),
                $"name=\"{field.Name}\"");
      }

      var head = $"public sealed record {alternative.Name}";

      if (parameters.Count == 0)
      {
        source.WriteLine($"{head} : {choice.Name}");
      }
      else
      {
        var under = new string(' ',
                               head.Length + 1);

        for (var n = 0; n < parameters.Count; n++)
        {
          var field = parameters[n];
          var parameter = field.Required
                            ? $"{field.Type} {field.Name}"
                            : $"{field.Type}? {field.Name} = null";

          source.WriteLine((n == 0
                              ? head + "("
                              : under) + parameter + (n == parameters.Count - 1
                                                        ? $") : {choice.Name}"
                                                        : ","));
        }
      }

      source.WriteLine("{");
      source.Indent++;

      // A required reference refused as it is passed: the type says it is never null, and every
      // reader of the record trusts the type.
      foreach (var field in parameters.Where(field => field.Required && (field.Kind == OptionKind.Choice || field.Type == "string")))
      {
        Document(source,
                 field.Description);
        source.WriteLine($"public {field.Type} {field.Name} {{ get; init; }} = {field.Name} ?? throw new ArgumentNullException(nameof({field.Name}));");
        Blank(source);
      }

      AppendValidate(source,
                     alternative.Fields,
                     """
                     /// <inheritdoc />
                     public override void Validate()
                     """);

      if (alternative.Fields.Any(field => field.Secret))
      {
        AppendPrintMembers(source,
                           alternative);
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // A record prints every property, in ToString and so in a log or a debugger. A secret is printed
    // as whether it is set, as the engine's Debug print of a password does. `global::` because a
    // choice may have an alternative named `System`.
    private static void AppendPrintMembers(IndentedTextWriter source,
                                           Alternative alternative)
    {
      Lines(source,
            """

            /// <summary>The fields, a secret one elided.</summary>
            protected override bool PrintMembers(global::System.Text.StringBuilder builder)
            {
            """);

      source.Indent++;

      for (var n = 0; n < alternative.Fields.Count; n++)
      {
        var field = alternative.Fields[n];
        var printed = field.Secret
                        ? $"{field.Name} is null ? \"null\" : \"***\""
                        : $"(object?){field.Name}";

        source.WriteLine($"builder.Append(\"{(n == 0 ? "" : ", ")}{field.Name} = \");");
        source.WriteLine($"builder.Append({printed});");
      }

      Lines(source,
            """

            return true;
            """);

      source.Indent--;
      source.WriteLine("}");
    }

    private static void AppendBindChoice(IndentedTextWriter source,
                                         OptionChoice choice,
                                         string reading)
    {
      Lines(source,
            $$"""

              /// <summary>The alternative <paramref name="section" /> names by its one key, matched without case.</summary>
              /// <param name="section">The section, which has to hold one key.</param>
              /// <returns>The alternative, with the fields its key states.</returns>
              /// <exception cref="InvalidOperationException">
              ///   The section names no alternative or two, or one with a field it does not admit.
              /// </exception>
              internal static {{choice.Name}} Bind(IConfigurationSection section)
              {
                var alternative = {{reading}}.Alternative(section,
                {{new string(' ', $"var alternative = {reading}.Alternative(".Length)}}"{{choice.Name}}");
              """);

      source.Indent++;

      foreach (var alternative in choice.Alternatives)
      {
        Lines(source,
              $$"""

                if ({{reading}}.Is(alternative,
                {{new string(' ', "if (".Length + reading.Length + ".Is(".Length)}}"{{alternative.Name}}"))
                {
                """);

        source.Indent++;

        switch (alternative.Shape)
        {
          case AlternativeShape.Unit:
            Lines(source,
                  $$"""
                    {{reading}}.Chosen(alternative);

                    return new {{alternative.Name}}();
                    """);
            break;

          case AlternativeShape.Value:
            source.WriteLine($"return new {alternative.Name}({Required(alternative.Fields[0], Reader(alternative.Fields[0], "alternative", reading), reading, "a value")});");
            break;

          case AlternativeShape.Fields:
            var parameters = Ordered(alternative.Fields);

            foreach (var field in parameters)
            {
              source.WriteLine($"{field.Type}? {Local(field.Name)} = null;");
            }

            Lines(source,
                  $$"""

                    foreach (var entry in {{reading}}.Entries(alternative))
                    {
                    """);

            source.Indent++;

            AppendMatches(source,
                          alternative.Fields,
                          field => $"{Local(field.Name)} = {Reader(field, "entry", reading)};",
                          $"throw {reading}.Unknown(entry,\n{new string(' ', $"throw {reading}.Unknown(".Length)}\"{choice.Name}.{alternative.Name}\");",
                          reading);

            source.Indent--;
            source.WriteLine("}");
            Blank(source);

            if (parameters.Count == 0)
            {
              source.WriteLine($"return new {alternative.Name}();");
              break;
            }

            var call = $"return new {alternative.Name}(";
            var under = new string(' ',
                                   call.Length);

            for (var n = 0; n < parameters.Count; n++)
            {
              var field = parameters[n];
              var argument = field.Required
                               ? $"{Local(field.Name)} ?? throw {reading}.Missing(alternative,\n{under}{new string(' ', $"{Local(field.Name)} ?? throw {reading}.Missing(".Length)}\"{field.Name}\")"
                               : Local(field.Name);

              Lines(source,
                    (n == 0
                       ? call
                       : under) + argument + (n == parameters.Count - 1
                                                ? ");"
                                                : ","));
            }

            break;
        }

        source.Indent--;
        source.WriteLine("}");
      }

      Lines(source,
            $$"""

              throw {{reading}}.Unknown(alternative,
              {{new string(' ', $"throw {reading}.Unknown(".Length)}}"{{choice.Name}}");
              """);

      source.Indent--;
      source.WriteLine("}");
    }

    // A required value read where only text that is there can be one: an empty text is refused,
    // since the alternative cannot be made without it. Text needs no such check - empty is a text.
    private static string Required(Option field,
                                   string read,
                                   string reading,
                                   string what)
      => field.Type == "string"
           ? read
           : $"{read} ?? throw {reading}.Missing(alternative, \"{what}\")";

    // The engine reads an alternative as an object of one key naming it, which no serializer
    // policy writes from a record: written here, by the schema's names. Only written, because
    // options go one way, to the engine.
    private static void AppendConverter(IndentedTextWriter source,
                                        OptionChoice choice)
    {
      var converter = Converter(choice.Name);

      Lines(source,
            $$"""
              /// <summary>Writes a <see cref="{{choice.Name}}" /> as the engine reads one: an object whose one key names the alternative.</summary>
              internal sealed class {{converter}} : JsonConverter<{{choice.Name}}>
              {
                /// <inheritdoc />
                /// <remarks>Options go to the engine and nothing reads them back, so this reads nothing.</remarks>
                public override {{choice.Name}}? Read(ref Utf8JsonReader reader,
                {{new string(' ', $"public override {choice.Name}? Read(".Length)}}Type typeToConvert,
                {{new string(' ', $"public override {choice.Name}? Read(".Length)}}JsonSerializerOptions options)
                  => throw new NotSupportedException("{{choice.Name}} is written to the engine, and never read back.");

                /// <inheritdoc />
                public override void Write(Utf8JsonWriter writer,
                                           {{choice.Name}} value,
                                           JsonSerializerOptions options)
                  => WriteValue(writer,
                                value);

                /// <summary>Writes <paramref name="written" />, as the converter of a choice holding one does too.</summary>
                /// <param name="writer">Where it is written.</param>
                /// <param name="written">The alternative.</param>
                internal static void WriteValue(Utf8JsonWriter writer,
                                                {{choice.Name}} written)
                {
                  writer.WriteStartObject();

                  switch (written)
                  {
              """);

      source.Indent += 3;

      for (var n = 0; n < choice.Alternatives.Count; n++)
      {
        var alternative = choice.Alternatives[n];
        var local = Local(alternative.Name);

        if (n > 0)
        {
          Blank(source);
        }

        source.WriteLine(alternative.Shape == AlternativeShape.Unit
                           ? $"case {choice.Name}.{alternative.Name}:"
                           : $"case {choice.Name}.{alternative.Name} {local}:");
        source.WriteLine("{");
        source.Indent++;

        switch (alternative.Shape)
        {
          case AlternativeShape.Unit:
            Call(source,
                 "writer.WriteBoolean",
                 $"\"{alternative.Name}\"",
                 "true");
            break;

          case AlternativeShape.Value:
            AppendWrite(source,
                        alternative.Name,
                        alternative.Fields[0],
                        $"{local}.Value");
            break;

          case AlternativeShape.Fields:
            source.WriteLine($"writer.WriteStartObject(\"{alternative.Name}\");");

            // A block stands apart from the statements around it.
            var apart = false;

            foreach (var field in alternative.Fields)
            {
              if (field.Required)
              {
                if (apart)
                {
                  Blank(source);
                  apart = false;
                }

                AppendWrite(source,
                            field.Name,
                            field,
                            $"{local}.{field.Name}");
                continue;
              }

              var value = Local(field.Name);
              Blank(source);
              source.WriteLine($"if ({local}.{field.Name} is {field.Type} {value})");
              source.WriteLine("{");
              source.Indent++;
              AppendWrite(source,
                          field.Name,
                          field,
                          value);
              source.Indent--;
              source.WriteLine("}");
              apart = true;
            }

            if (apart)
            {
              Blank(source);
            }

            source.WriteLine("writer.WriteEndObject();");
            break;
        }

        source.WriteLine("break;");
        source.Indent--;
        source.WriteLine("}");
      }

      source.Indent -= 3;

      Lines(source,
            """
                  }

                  writer.WriteEndObject();
                }
              }
              """);
    }

    // One value under its name, as the type writes it: a choice by its own converter, and an
    // enumeration by its name, which is the one the schema spells.
    private static void AppendWrite(IndentedTextWriter source,
                                    string name,
                                    Option option,
                                    string value)
    {
      var key = $"\"{name}\"";

      switch (option.Kind)
      {
        case OptionKind.Choice:
          source.WriteLine($"writer.WritePropertyName({key});");
          Call(source,
               $"{Converter(option.Type)}.WriteValue",
               "writer",
               value);
          return;

        case OptionKind.Enumeration:
          Call(source,
               "writer.WriteString",
               key,
               $"{value}.ToString()");
          return;
      }

      Call(source,
           option.Type switch
           {
             "string"                   => "writer.WriteString",
             "int" or "long" or "double" => "writer.WriteNumber",
             "bool"                     => "writer.WriteBoolean",
             _                          => throw new NotSupportedException($"`{option.Name}` is a `{option.Type}`, which this generator writes nowhere."),
           },
           key,
           value);
    }

    // A call with its arguments one per line, lined up under the first.
    private static void Call(IndentedTextWriter source,
                             string method,
                             params string[] arguments)
    {
      var under = new string(' ',
                             method.Length + 1);

      for (var n = 0; n < arguments.Length; n++)
      {
        source.WriteLine((n == 0
                            ? method + "("
                            : under) + arguments[n] + (n == arguments.Length - 1
                                                         ? ");"
                                                         : ","));
      }
    }

    // An enumeration is a C# enum whose names are the schema's, written as those names: the
    // engine reads a name, and a number would be a value it refuses.
    private static void AppendEnumeration(IndentedTextWriter source,
                                          OptionEnumeration enumeration)
    {
      Document(source,
               enumeration.Description);

      source.WriteLine($"[JsonConverter(typeof(JsonStringEnumConverter<{enumeration.Name}>))]");
      source.WriteLine($"public enum {enumeration.Name}");
      source.WriteLine("{");
      source.Indent++;

      for (var n = 0; n < enumeration.Members.Count; n++)
      {
        if (n > 0)
        {
          Blank(source);
        }

        Document(source,
                 enumeration.Members[n].Description);
        source.WriteLine($"{enumeration.Members[n].Name},");
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // What a rendered `Bind` reads a key's text with, and only what one calls: a member nothing
    // calls is code no build exercises. A key matches without case, as a configuration matches it;
    // a value is read as the invariant culture writes it, as `ConfigurationBinder` reads one; and a
    // value is never quoted, since it may be a password.
    private static void AppendReading(IndentedTextWriter source,
                                      string reading,
                                      ISet<string> used)
    {
      source.WriteLine("/// <summary>How the text of a configuration becomes the options above, and why it may not.</summary>");
      source.WriteLine($"internal static class {reading}");
      source.WriteLine("{");
      source.Indent++;

      var first = true;

      foreach (var (name, _, member) in ReadingMembers)
      {
        if (!used.Contains(name))
        {
          continue;
        }

        if (!first)
        {
          Blank(source);
        }

        Lines(source,
              member);
        first = false;
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // The members of the reading a set of types calls, with every member those call in turn.
    private static HashSet<string> Used(IEnumerable<OptionType> types)
    {
      var used = new HashSet<string>(StringComparer.Ordinal);

      void Reads(Option option)
        => used.Add(option.Kind switch
                    {
                      OptionKind.Enumeration     => "Enumeration",
                      OptionKind.EnumerationList => "Enumerations",
                      OptionKind.Value => option.Type switch
                                          {
                                            "int"    => "Int32",
                                            "long"   => "Int64",
                                            "double" => "Double",
                                            "bool"   => "Boolean",
                                            _        => "Text",
                                          },
                      // A group or a choice is read by its own `Bind`, once the key holds one.
                      _ => "Holds",
                    });

      foreach (var type in types)
      {
        switch (type)
        {
          case OptionGroup group:
            used.UnionWith(new[]
                           {
                             "Is",
                             "Entries",
                             "Unknown",
                           });

            foreach (var option in group.Options)
            {
              Reads(option);
            }

            break;

          case OptionChoice choice:
            used.UnionWith(new[]
                           {
                             "Alternative",
                             "Is",
                             "Unknown",
                           });

            foreach (var alternative in choice.Alternatives)
            {
              if (alternative.Shape == AlternativeShape.Unit)
              {
                used.Add("Chosen");
              }

              if (alternative.Shape == AlternativeShape.Fields)
              {
                used.Add("Entries");
              }

              foreach (var field in alternative.Fields)
              {
                Reads(field);

                // Text is never missing: an empty one is a text.
                if (field.Required && !(alternative.Shape == AlternativeShape.Value && field.Type == "string"))
                {
                  used.Add("Missing");
                }
              }
            }

            break;
        }
      }

      for (var grew = true; grew;)
      {
        grew = false;

        foreach (var (name, calls, _) in ReadingMembers)
        {
          if (used.Contains(name))
          {
            foreach (var called in calls)
            {
              grew |= used.Add(called);
            }
          }
        }
      }

      return used;
    }

    // Each member of the reading, what it calls, and its text, in the order a file declares them.
    private static readonly (string Name, string[] Calls, string Text)[] ReadingMembers =
    {
      ("Is",
       Array.Empty<string>(),
       """
       /// <summary>Whether <paramref name="section" /> is the key <paramref name="name" />, without case.</summary>
       internal static bool Is(IConfigurationSection section,
                               string                name)
         => string.Equals(section.Key,
                          name,
                          StringComparison.OrdinalIgnoreCase);
       """),
      ("Entries",
       Array.Empty<string>(),
       """
       /// <summary>The keys of a section that holds options, a key set to null left out as unset.</summary>
       /// <exception cref="InvalidOperationException">It holds a value instead.</exception>
       internal static IConfigurationSection[] Entries(IConfigurationSection section)
         => string.IsNullOrEmpty(section.Value)
              ? section.GetChildren()
                       .Where(entry => entry.Value is not null || entry.GetChildren()
                                                                       .Any())
                       .ToArray()
              : throw new InvalidOperationException($"{section.Path} holds a value, and it names options.");
       """),
      ("Holds",
       Array.Empty<string>(),
       """
       /// <summary>Whether a key names anything, an empty one leaving its group or choice unset.</summary>
       internal static bool Holds(IConfigurationSection section)
         => !string.IsNullOrEmpty(section.Value) || section.GetChildren()
                                                           .Any();
       """),
      ("Alternative",
       new[]
       {
         "Entries",
       },
       """
       /// <summary>The one key of a section naming an alternative of <paramref name="choice" />.</summary>
       /// <exception cref="InvalidOperationException">It holds none, or more than one.</exception>
       internal static IConfigurationSection Alternative(IConfigurationSection section,
                                                         string                choice)
       {
         var entries = Entries(section);

         return entries.Length == 1
                  ? entries[0]
                  : throw new InvalidOperationException($"{section.Path} names {entries.Length} alternatives of {choice}, and it has to name one.");
       }
       """),
      ("Chosen",
       new[]
       {
         "Boolean",
       },
       """
       /// <summary>Refuses an alternative chosen with anything but `true`.</summary>
       internal static void Chosen(IConfigurationSection section)
       {
         if (Boolean(section) != true)
         {
           throw new InvalidOperationException($"{section.Path} has to be true: an alternative is chosen with true.");
         }
       }
       """),
      ("Text",
       Array.Empty<string>(),
       """
       /// <summary>The text of a key that holds a value.</summary>
       /// <exception cref="InvalidOperationException">It holds options instead.</exception>
       internal static string Text(IConfigurationSection section)
         => section.Value ?? throw new InvalidOperationException($"{section.Path} holds options, and it names a value.");
       """),
      ("Int32",
       new[]
       {
         "Text",
         "Unreadable",
       },
       """
       /// <summary>An integer, or none where the text is empty.</summary>
       internal static int? Int32(IConfigurationSection section)
       {
         var text = Text(section);

         return text.Length == 0
                  ? null
                  : int.TryParse(text,
                                 NumberStyles.Integer,
                                 CultureInfo.InvariantCulture,
                                 out var value)
                    ? value
                    : throw Unreadable(section,
                                       "an integer");
       }
       """),
      ("Int64",
       new[]
       {
         "Text",
         "Unreadable",
       },
       """
       /// <summary>An integer, or none where the text is empty.</summary>
       internal static long? Int64(IConfigurationSection section)
       {
         var text = Text(section);

         return text.Length == 0
                  ? null
                  : long.TryParse(text,
                                  NumberStyles.Integer,
                                  CultureInfo.InvariantCulture,
                                  out var value)
                    ? value
                    : throw Unreadable(section,
                                       "an integer");
       }
       """),
      ("Double",
       new[]
       {
         "Text",
         "Unreadable",
       },
       """
       /// <summary>A number, or none where the text is empty.</summary>
       internal static double? Double(IConfigurationSection section)
       {
         var text = Text(section);

         return text.Length == 0
                  ? null
                  : double.TryParse(text,
                                    NumberStyles.Float,
                                    CultureInfo.InvariantCulture,
                                    out var value)
                    ? value
                    : throw Unreadable(section,
                                       "a number");
       }
       """),
      ("Boolean",
       new[]
       {
         "Text",
         "Unreadable",
       },
       """
       /// <summary>A flag, or none where the text is empty.</summary>
       internal static bool? Boolean(IConfigurationSection section)
       {
         var text = Text(section);

         return text.Length == 0
                  ? null
                  : bool.TryParse(text,
                                  out var value)
                    ? value
                    : throw Unreadable(section,
                                       "true or false");
       }
       """),
      ("Enumeration",
       new[]
       {
         "Text",
         "Unreadable",
       },
       """
       /// <summary>A name <typeparamref name="T" /> declares, without case, or none where the text is empty.</summary>
       /// <remarks>Matched by name, never parsed: `Enum.TryParse` reads digits as any value of the type.</remarks>
       internal static T? Enumeration<T>(IConfigurationSection section)
         where T : struct, Enum
       {
         var text = Text(section);

         if (text.Length == 0)
         {
           return null;
         }

         var names = Enum.GetNames(typeof(T));
         var name = names.FirstOrDefault(declared => string.Equals(declared,
                                                                   text,
                                                                   StringComparison.OrdinalIgnoreCase));

         return name is null
                  ? throw Unreadable(section,
                                     "one of " + string.Join(", ",
                                                             names))
                  : (T)Enum.Parse(typeof(T),
                                  name);
       }
       """),
      ("Enumerations",
       new[]
       {
         "Enumeration",
         "Unreadable",
       },
       """
       /// <summary>The names of a key that holds a list, each one <typeparamref name="T" /> declares, or none where it holds nothing.</summary>
       /// <remarks>A key without entries is unset, so a configuration cannot state an empty list; two layers that both state an index are merged by it.</remarks>
       /// <exception cref="InvalidOperationException">It holds a value instead of entries, or an entry names nothing <typeparamref name="T" /> declares.</exception>
       internal static global::System.Collections.Generic.List<T>? Enumerations<T>(IConfigurationSection section)
         where T : struct, Enum
       {
         if (!string.IsNullOrEmpty(section.Value))
         {
           throw new InvalidOperationException($"{section.Path} holds a value, and it names a list: each name goes under an index of its own.");
         }

         var entries = section.GetChildren()
                              .ToList();

         return entries.Count == 0
                  ? null
                  : entries.Select(entry => Enumeration<T>(entry) ?? throw Unreadable(entry,
                                                                                    "a name"))
                           .ToList();
       }
       """),
      ("Unknown",
       Array.Empty<string>(),
       """
       /// <summary>A key nothing declares, refused by its path.</summary>
       internal static InvalidOperationException Unknown(IConfigurationSection section,
                                                         string                owner)
         => new($"{section.Path} names nothing {owner} declares.");
       """),
      ("Missing",
       Array.Empty<string>(),
       """
       /// <summary>A field the alternative cannot be made without.</summary>
       internal static InvalidOperationException Missing(IConfigurationSection section,
                                                         string                what)
         => new($"{section.Path} has to state {what}.");
       """),
      ("Unreadable",
       Array.Empty<string>(),
       """
       private static InvalidOperationException Unreadable(IConfigurationSection section,
                                                           string                what)
         => new($"{section.Path} has to be {what}.");
       """),
    };

    // Only the root group: the document the engine reads is one object, and a group of it is not
    // a document. `Validate` runs first, because the engine answers a bad one with a status that
    // names neither the option nor the bound.
    private static void AppendEncode(IndentedTextWriter source,
                                     OptionGroup group)
      => Lines(source,
               $$"""

                 /// <summary>The document the engine reads, as UTF-8.</summary>
                 /// <returns>The options as JSON, without the ones left unset.</returns>
                 /// <exception cref="ArgumentOutOfRangeException">An option is outside its bounds.</exception>
                 /// <remarks>
                 ///   Checked before it is written, not after it is refused: the engine answers a bad
                 ///   document with a status naming neither the option nor the bound.
                 /// </remarks>
                 internal byte[] Encode()
                 {
                   Validate();

                   return JsonSerializer.SerializeToUtf8Bytes(this,
                                                              {{group.Name}}JsonContext.Default.{{group.Name}});
                 }
                 """);

    // A caller's instance is theirs, and what a channel reads has to stay what it read: a value
    // taken twice from a settable property is two values if anything sets it in between. So a
    // holder takes a copy, and the copy is deep - a group left shared would be the same hole one
    // level down.
    private static void AppendConstructors(IndentedTextWriter source,
                                           OptionGroup group)
    {
      Lines(source,
            $$"""
              /// <summary>Options nobody has set.</summary>
              public {{group.Name}}()
              {
              }

              /// <summary>A copy of <paramref name="other" />, sharing nothing with it.</summary>
              /// <param name="other">The options to copy.</param>
              /// <exception cref="ArgumentNullException"><paramref name="other" /> is null.</exception>
              public {{group.Name}}({{group.Name}} other)
              {
                if (other is null)
                {
                  throw new ArgumentNullException(nameof(other));
                }
              """);

      source.Indent++;

      if (group.Options.Count > 0)
      {
        Blank(source);
      }

      foreach (var option in group.Options)
      {
        // A choice is a record and an enumeration a value: neither changes once made, so both
        // are shared rather than copied. A list changes, and is copied: its names are values.
        if (option.Kind is not (OptionKind.Group or OptionKind.EnumerationList))
        {
          source.WriteLine($"{option.Name} = other.{option.Name};");
          continue;
        }

        // The branches start one indent right of `other`, which follows `Name = `.
        var under = new string(' ',
                               option.Name.Length + " = ".Length + Indent.Length);
        Lines(source,
              $"""
               {option.Name} = other.{option.Name} is null
               {under}? null
               {under}: new {PropertyType(option)}(other.{option.Name});
               """);
      }

      source.Indent--;
      source.WriteLine("}");
      Blank(source);
    }

    // Every bound the schema states, checked here. The schema cannot say all of it in the type -
    // an `int` admits zero and the schema does not - so what the type lets through, this refuses.
    private static void AppendValidate(IndentedTextWriter source,
                                       IReadOnlyList<Option> options,
                                       string signature)
    {
      var checks = options.Where(option => option.Kind is OptionKind.Enumeration or OptionKind.EnumerationList ||
                                           (option.Kind == OptionKind.Value && (option.Bounds.Count > 0 || option.Type == "double")))
                          .ToList();
      var nested = options.Where(option => option.Kind is OptionKind.Group or OptionKind.Choice)
                          .ToList();

      Lines(source,
            signature);
      source.WriteLine("{");
      source.Indent++;

      if (checks.Count == 0 && nested.Count == 0)
      {
        source.WriteLine("// The schema bounds nothing here.");
      }

      // Lined up under the first argument of the throw below.
      var under = new string(' ',
                             "throw new ArgumentOutOfRangeException(".Length);

      // One check per option rather than one per keyword: two `is` patterns in a block would
      // declare the same local twice, and a caller who set a window to zero wants the range it
      // had to be in, not the first half of it.
      for (var n = 0; n < checks.Count; n++)
      {
        var option = checks[n];
        var value  = Local(option.Name);

        var refuses = new List<string>();
        var says    = new List<string>();

        // A list is checked by each of its names, and says so in the plural. Its item is a name
        // of its own so that it cannot be the local of an option.
        if (option.Kind == OptionKind.EnumerationList)
        {
          var item = value == "item"
                       ? "member"
                       : "item";

          if (n > 0)
          {
            Blank(source);
          }

          var undeclared = value + "Undeclared";

          // The chain's second link lines up under the first: `var x = list.Where(...)`.
          var chain = new string(' ',
                                 "var ".Length + undeclared.Length + " = ".Length + value.Length);

          Lines(source,
                $$"""
                  if ({{option.Name}} is { } {{value}})
                  {
                    var {{undeclared}} = {{value}}.Where({{item}} => !Enum.IsDefined(typeof({{option.Type}}), {{item}}))
                    {{chain}}.ToList();

                    if ({{undeclared}}.Count > 0)
                    {
                      throw new ArgumentOutOfRangeException(nameof({{option.Name}}),
                      {{under}}string.Join(", ",
                      {{under}}            {{undeclared}}),
                      {{under}}"{{option.Name}} has to be names {{option.Type}} declares.");
                    }
                  }
                  """);
          continue;
        }

        // An enum is a number, and a cast makes any number one of its type.
        if (option.Kind == OptionKind.Enumeration)
        {
          refuses.Add($"!Enum.IsDefined(typeof({option.Type}), {value})");
          says.Add($"a name {option.Type} declares");
        }

        foreach (var bound in option.Bounds)
        {
          var (refusal, said) = Check(bound,
                                      value,
                                      option.Type);
          refuses.Add(refusal);
          says.Add(said);
        }

        // A JSON number is finite and a `double` is not. NaN passes every bound - each comparison
        // against it is false - and System.Text.Json refuses to write any of the three, so a
        // caller who set one would get its exception rather than one naming the option.
        if (option.Type == "double")
        {
          refuses.Add($"double.IsNaN({value})");
          refuses.Add($"double.IsInfinity({value})");
          says.Add("finite");
        }

        var condition = string.Join(" || ",
                                    refuses);
        var test = refuses.Count > 1
                     ? $"({condition})"
                     : condition;

        if (n > 0)
        {
          Blank(source);
        }

        // A secret is refused without being quoted, since the exception's message carries the value.
        Lines(source,
              option.Secret
                ? $$"""
                    if ({{option.Name}} is {{option.Type}} {{value}} && {{test}})
                    {
                      throw new ArgumentOutOfRangeException(nameof({{option.Name}}),
                      {{under}}"{{option.Name}} has to be {{string.Join(" and ", says)}}.");
                    }
                    """
                : $$"""
                    if ({{option.Name}} is {{option.Type}} {{value}} && {{test}})
                    {
                      throw new ArgumentOutOfRangeException(nameof({{option.Name}}),
                      {{under}}{{value}},
                      {{under}}"{{option.Name}} has to be {{string.Join(" and ", says)}}.");
                    }
                    """);
      }

      if (nested.Count > 0 && checks.Count > 0)
      {
        Blank(source);
      }

      // A type's own bounds are its own to state, and it says so where it is declared.
      foreach (var option in nested)
      {
        source.WriteLine(option.Required
                           ? $"{option.Name}.Validate();"
                           : $"{option.Name}?.Validate();");
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // A description is one paragraph of what the option is, and any others of what a caller has to
    // know about it - which is the same split `<summary>` and `<remarks>` make.
    private static void Document(IndentedTextWriter source,
                                 string description)
    {
      // At least one paragraph, because `OptionVocabulary` refuses a description that is absent
      // or blank - so there is no case here where an option or a group gets no documentation.
      var paragraphs = Paragraphs(description);

      Element(source,
              "summary",
              paragraphs[0]);

      if (paragraphs.Count > 1)
      {
        Element(source,
                "remarks",
                string.Join("\n",
                            paragraphs.Skip(1)));
      }
    }

    private static List<string> Paragraphs(string description)
      => Newlines(description)
         .Split(new[]
                {
                  "\n\n",
                },
                StringSplitOptions.None)
         .Select(paragraph => paragraph.Trim())
         .Where(paragraph => paragraph.Length > 0)
         .ToList();

    // Every line ending becomes a newline: what C# ends a line on, not what a reader would call
    // one. A terminator left inside a description puts the rest of it outside the `///` and into
    // the code, and the three Unicode ones arrive by a paste as easily as by anything else.
    //
    // `\r\n` first: normalising `\r` alone would turn `\r\r\n` into a `\r\n` that reaches the
    // file, which `--check` normalises on the committed side only and so could never match again.
    private static string Newlines(string text)
      => text.Replace("\r\n",
                      "\n")
             .Replace("\r",
                      "\n")
             // C# ends a line on these three as well, and no Replace of a visible character can
             // stand for them: written as escapes so this file stays ASCII.
             .Replace("\u0085",
                      "\n")
             .Replace("\u2028",
                      "\n")
             .Replace("\u2029",
                      "\n");

    private static void Element(IndentedTextWriter source,
                                string element,
                                string text,
                                string? attributes = null)
    {
      var lines = Xml(text)
                  .Split('\n')
                  .Select(line => line.TrimEnd())
                  .ToList();
      var open = attributes is null
                   ? element
                   : $"{element} {attributes}";

      if (lines.Count == 1)
      {
        source.WriteLine($"/// <{open}>{lines[0]}</{element}>");
        return;
      }

      source.WriteLine($"/// <{open}>");

      // Nothing after the `///` on an empty line: .editorconfig trims trailing whitespace, so an
      // editor that honours it would rewrite a file that has to stay what is rendered.
      foreach (var line in lines)
      {
        source.WriteLine(line.Length == 0
                           ? "///"
                           : "///   " + line);
      }

      source.WriteLine($"/// </{element}>");
    }

    // One line at a time, because the writer indents a line only when it starts one: a newline
    // inside a single write would leave the next line at the margin. The literals above take this
    // file's own line endings, which a checkout may make `\r\n`.
    private static void Lines(IndentedTextWriter source,
                              string block)
    {
      foreach (var line in block.Replace("\r\n",
                                         "\n")
                                .Split('\n'))
      {
        if (line.Length == 0)
        {
          Blank(source);
        }
        else
        {
          source.WriteLine(line);
        }
      }
    }

    // Unindented, for the same trailing-whitespace reason as the empty `///` line.
    private static void Blank(IndentedTextWriter source)
      => source.WriteLineNoTabs(string.Empty);

    // The text is prose written in Rust doc comments, where a backtick is code and the three XML
    // characters are themselves.
    private static string Xml(string text)
    {
      var escaped = text.Replace("&",
                                 "&amp;")
                        .Replace("<",
                                 "&lt;")
                        .Replace(">",
                                 "&gt;");

      var rendered = new StringBuilder(escaped.Length);
      var open     = true;

      foreach (var character in escaped)
      {
        if (character != '`')
        {
          rendered.Append(character);
          continue;
        }

        rendered.Append(open
                          ? "<c>"
                          : "</c>");
        open = !open;
      }

      // An odd number of backticks would leave a tag open, and prose the compiler refuses to read
      // is worse than prose with a backtick in it.
      return open
               ? rendered.ToString()
               : escaped;
    }

    // What a keyword becomes: the condition that refuses a value it excludes, and the same thing
    // in words for the message.
    //
    // The type decides as much as the keyword does. Draft 2020-12 lets a schema state a keyword
    // that does not apply to its instance type - `minLength` asserts nothing about a number - and
    // emitting it anyway would be `.Length` on an `int`. So a keyword the type has no check for
    // stops the generator rather than the compiler.
    private static (string Refuses, string Says) Check(Bound bound,
                                                       string value,
                                                       string type)
    {
      var numeric = type is "int" or "long" or "double";
      var text    = type is "string";

      return bound.Keyword switch
             {
               "minimum" when numeric          => ($"{value} < {bound.Literal}", $"at least {bound.Literal}"),
               "maximum" when numeric          => ($"{value} > {bound.Literal}", $"at most {bound.Literal}"),
               "exclusiveMinimum" when numeric => ($"{value} <= {bound.Literal}", $"greater than {bound.Literal}"),
               "exclusiveMaximum" when numeric => ($"{value} >= {bound.Literal}", $"less than {bound.Literal}"),
               "minLength" when text           => ($"{value}.Length < {bound.Literal}", $"at least {Characters(bound.Literal)} long"),
               "maxLength" when text           => ($"{value}.Length > {bound.Literal}", $"at most {Characters(bound.Literal)} long"),
               _ => throw new NotSupportedException($"`{bound.Keyword}` bounds no `{type}`, so this generator states no check for one there."),
             };
    }

    private static string Characters(string count)
      => count == "1"
           ? "1 character"
           : count + " characters";

    // A name is PascalCase, so lowercasing its first letter gives an identifier that differs from
    // the property and from every other option's - two names differing only there could not both
    // be PascalCase. What it can be is a keyword, and `@` is what makes one an identifier again.
    private static string Local(string name)
    {
      var local = char.ToLowerInvariant(name[0]) + name.Substring(1);

      return Reserved.Contains(local)
               ? "@" + local
               : local;
    }

    /// <summary>Refuses a name that is not a C# name of this vocabulary.</summary>
    /// <param name="name">The name the schema spells.</param>
    /// <param name="what">What it names, for the message.</param>
    /// <exception cref="NotSupportedException">The name has no C# form this emits.</exception>
    /// <remarks>
    ///   A name reaches an identifier, a `nameof`, a string literal in an attribute and a string
    ///   literal in a message, and none of the four is quoted on the way. PascalCase letters and
    ///   digits is what all of them take verbatim, and it is what this vocabulary produces: an
    ///   option is a Rust field renamed. Anything else is a compile error in a file nobody wrote,
    ///   which is a poor way to learn that a schema said something this cannot write.
    /// </remarks>
    private static void RefuseAName(string name,
                                    string what)
    {
      if (!PascalCase.IsMatch(name))
      {
        throw new NotSupportedException($"`{name}` names {what}, and this generator writes a name of PascalCase letters and digits.");
      }

      if (Members.Contains(name))
      {
        throw new NotSupportedException($"`{name}` names {what}, which is also a member every generated class has.");
      }
    }

    // `\A` and `\z` rather than `^` and `$`: outside multiline mode `$` still matches before a
    // single trailing newline, so `^...$` admits a name ending in one - which reaches a string
    // literal in the emitted C# and does not close it.
    private static readonly Regex PascalCase = new(@"\A[A-Z][A-Za-z0-9]*\z",
                                                   RegexOptions.CultureInvariant);

    // What a generated type already declares, or inherits and would shadow without saying so.
    // `Encode` is the root's, and a group has none - listed for every class all the same, because
    // which group is the root is no reason for an option to be spelled one way here. `Bind` is
    // every group's and choice's; `Deconstruct`, `EqualityContract` and `PrintMembers` are what C#
    // declares on every record.
    private static readonly HashSet<string> Members = new(StringComparer.Ordinal)
                                                      {
                                                        "Bind",
                                                        "Deconstruct",
                                                        "Encode",
                                                        "EqualityContract",
                                                        "Equals",
                                                        "GetHashCode",
                                                        "GetType",
                                                        "PrintMembers",
                                                        "ToString",
                                                        "Validate",
                                                      };

    private static readonly HashSet<string> Reserved = new(StringComparer.Ordinal)
                                                       {
                                                         "abstract",
                                                         "as",
                                                         "base",
                                                         "bool",
                                                         "break",
                                                         "byte",
                                                         "case",
                                                         "catch",
                                                         "char",
                                                         "checked",
                                                         "class",
                                                         "const",
                                                         "continue",
                                                         "decimal",
                                                         "default",
                                                         "delegate",
                                                         "do",
                                                         "double",
                                                         "else",
                                                         "enum",
                                                         "event",
                                                         "explicit",
                                                         "extern",
                                                         "false",
                                                         "finally",
                                                         "fixed",
                                                         "float",
                                                         "for",
                                                         "foreach",
                                                         "goto",
                                                         "if",
                                                         "implicit",
                                                         "in",
                                                         "int",
                                                         "interface",
                                                         "internal",
                                                         "is",
                                                         "lock",
                                                         "long",
                                                         "namespace",
                                                         "new",
                                                         "null",
                                                         "object",
                                                         "operator",
                                                         "out",
                                                         "override",
                                                         "params",
                                                         "private",
                                                         "protected",
                                                         "public",
                                                         "readonly",
                                                         "ref",
                                                         "return",
                                                         "sbyte",
                                                         "sealed",
                                                         "short",
                                                         "sizeof",
                                                         "stackalloc",
                                                         "static",
                                                         "string",
                                                         "struct",
                                                         "switch",
                                                         "this",
                                                         "throw",
                                                         "true",
                                                         "try",
                                                         "typeof",
                                                         "uint",
                                                         "ulong",
                                                         "unchecked",
                                                         "unsafe",
                                                         "ushort",
                                                         "using",
                                                         "virtual",
                                                         "void",
                                                         "volatile",
                                                         "while",
                                                       };
  }
}
