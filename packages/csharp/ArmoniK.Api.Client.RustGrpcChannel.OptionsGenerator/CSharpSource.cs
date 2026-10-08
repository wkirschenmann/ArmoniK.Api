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

      // Declared beside the vocabulary and named after a choice, so a type of one of these names
      // would be two types of one name in a file nobody wrote.
      var beside = new HashSet<string>(types.OfType<OptionChoice>()
                                            .Select(choice => Converter(choice.Name)),
                                       StringComparer.Ordinal);

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

              // The writing of an alternative declares locals of its own beside one per field and
              // one for the alternative, and two locals of one name do not compile.
              foreach (var name in alternative.Fields.Select(field => field.Name)
                                              .Append(alternative.Name))
              {
                if (Plumbing.Contains(Local(name)))
                {
                  throw new NotSupportedException($"`{name}` is in `{choice.Name}`, and its local would be one the rendered writing declares already.");
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
                          $"a name of `{enumeration.Name}`",
                          true);
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

      // A list's `Validate` checks its names with LINQ.
      var lists = types.OfType<OptionGroup>()
                       .Any(group => group.Options.Any(option => option.Kind == OptionKind.EnumerationList)) ||
                  types.OfType<OptionChoice>()
                       .Any(choice => choice.Alternatives.Any(alternative => alternative.Fields.Any(field => field.Kind == OptionKind.EnumerationList)));

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
              {{(lists ? "using System.Linq;\n" : "")}}{{(writes ? "using System.Text.Json;\n" : "")}}using System.Text.Json.Serialization;

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
                                                    types[0]));
            break;

          case OptionChoice choice:
            AppendChoice(source,
                         choice);
            break;

          case OptionEnumeration enumeration:
            AppendEnumeration(source,
                              enumeration);
            break;
        }
      }

      return text.ToString();
    }

    private static string Converter(string choice)
      => choice + "JsonConverter";

    // The locals a rendered `WriteValue` declares for itself.
    private static readonly HashSet<string> Plumbing = new(StringComparer.Ordinal)
                                                       {
                                                         "item",
                                                         "writer",
                                                         "written",
                                                       };

    // The engine reads the environment's keys and a command line's without case, so two names that
    // differ only in case would be one key there.
    private static void RefuseTwins(IEnumerable<string> names,
                                    string what)
    {
      var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

      foreach (var name in names)
      {
        if (!seen.Add(name))
        {
          throw new NotSupportedException($"`{name}` is one of two {what} that differ at most in case, which the engine reads as one key.");
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

    private static string PropertyType(Option option)
      => option.Kind == OptionKind.EnumerationList
           ? ListOf(option.Type)
           : option.Type;

    // What an alternative's record holds of a list: read-only, since a record is a value.
    private static string ParameterType(Option option)
      => option.Kind == OptionKind.EnumerationList
           ? $"global::System.Collections.Generic.IReadOnlyList<{option.Type}>"
           : option.Type;

    private static string ListOf(string type)
      => $"global::System.Collections.Generic.List<{type}>";

    private static void AppendGroup(IndentedTextWriter source,
                                    OptionGroup group,
                                    bool encodes)
    {
      Document(source,
               group.Description);

      // Public, because this is what a caller fills in. Not partial: the whole surface is
      // rendered here - the properties, the bounds, the copy and the encoding - so a
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
        // contract with the engine, which reads past an option it does not know, and a policy
        // set elsewhere would be enough to turn every option into one.
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

      if (encodes)
      {
        AppendEncode(source,
                     group);
      }

      source.Indent--;
      source.WriteLine("}");
    }

    // A choice is a closed hierarchy: an abstract record whose constructor is private, so the
    // sealed records nested in it are every alternative there is. The compiler does not know
    // that, and a switch over them without a default case is reported as incomplete. Records,
    // because an alternative is a value: two that say the same are equal, and none changes once
    // made, so a copy of the group holding one shares it safely.
    private static void AppendChoice(IndentedTextWriter source,
                                     OptionChoice choice)
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

      source.Indent--;
      source.WriteLine("}");

      Blank(source);
      AppendConverter(source,
                      choice);
    }

    // The shape of a record says what its alternative needs: a mandatory field is a parameter of
    // the constructor, with no default, so none can be left out, and an optional one is a
    // nullable property a caller sets by name, so that what is set is read at the call. An
    // alternative with no mandatory field has a constructor with no parameter.
    private static void AppendAlternative(IndentedTextWriter source,
                                          OptionChoice choice,
                                          Alternative alternative)
    {
      Document(source,
               alternative.Description);

      var parameters = alternative.Fields.Where(field => field.Required)
                                  .ToList();
      var optional = alternative.Fields.Where(field => !field.Required)
                                .ToList();

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
          var parameter = $"{ParameterType(field)} {field.Name}";

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
      foreach (var field in parameters.Where(field => field.Required && (field.Kind is OptionKind.Choice or OptionKind.EnumerationList || field.Type == "string")))
      {
        Document(source,
                 field.Description);

        // A list is copied, so the record keeps what it was given and not a list its caller still
        // holds.
        source.WriteLine(field.Kind == OptionKind.EnumerationList
                           ? $"public {ParameterType(field)} {field.Name} {{ get; init; }} = new {ListOf(field.Type)}({field.Name} ?? throw new ArgumentNullException(nameof({field.Name}))).AsReadOnly();"
                           : $"public {field.Type} {field.Name} {{ get; init; }} = {field.Name} ?? throw new ArgumentNullException(nameof({field.Name}));");
        Blank(source);
      }

      foreach (var field in optional)
      {
        Document(source,
                 field.Description);

        if (field.Kind == OptionKind.EnumerationList)
        {
          // A list is copied as it is given, so that the record keeps what it was given and not a
          // list its caller still holds, and stays null when it was not given.
          var held = $"{char.ToLowerInvariant(field.Name[0])}{field.Name.Substring(1)}_";
          source.WriteLine($"public {ParameterType(field)}? {field.Name}");
          source.WriteLine("{");
          source.Indent++;
          source.WriteLine($"get => {held};");
          source.WriteLine($"init => {held} = value is null ? null : new {ListOf(field.Type)}(value).AsReadOnly();");
          source.Indent--;
          source.WriteLine("}");
          Blank(source);
          source.WriteLine($"private {ParameterType(field)}? {held};");
        }
        else
        {
          source.WriteLine($"public {ParameterType(field)}? {field.Name} {{ get; init; }}");
        }
        Blank(source);
      }

      if (alternative.Fields.Any(field => field.Kind == OptionKind.EnumerationList))
      {
        AppendListEquality(source,
                           alternative);
      }

      AppendValidate(source,
                     alternative.Fields,
                     """
                     /// <inheritdoc />
                     public override void Validate()
                     """);

      if (alternative.Fields.Any(field => field.Secret || field.Kind == OptionKind.EnumerationList))
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
                        : field.Kind == OptionKind.EnumerationList
                          ? field.Required
                              ? $"\"[\" + string.Join(\", \", {field.Name}) + \"]\""
                              : $"{field.Name} is null ? \"null\" : \"[\" + string.Join(\", \", {field.Name}) + \"]\""
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

    // A record compares its fields, and a list compares by reference: two alternatives that name the
    // same statuses are equal all the same, as every other alternative is.
    private static void AppendListEquality(IndentedTextWriter source,
                                           Alternative alternative)
    {
      var equal = string.Join(" && ",
                              alternative.Fields.Select(field => field.Kind == OptionKind.EnumerationList
                                                                   ? field.Required
                                                                       ? $"global::System.Linq.Enumerable.SequenceEqual({field.Name}, other.{field.Name})"
                                                                       : $"({field.Name} is null ? other.{field.Name} is null : other.{field.Name} is not null && global::System.Linq.Enumerable.SequenceEqual({field.Name}, other.{field.Name}))"
                                                                   : $"global::System.Collections.Generic.EqualityComparer<{field.Type}{(field.Required ? "" : "?")}>.Default.Equals({field.Name}, other.{field.Name})"));

      Lines(source,
            $$"""
              /// <inheritdoc />
              public bool Equals({{alternative.Name}}? other)
                => other is not null && {{equal}};

              /// <inheritdoc />
              public override int GetHashCode()
              {
                var hash = 17;
              """);

      source.Indent++;

      foreach (var field in alternative.Fields)
      {
        if (field.Kind == OptionKind.EnumerationList)
        {
          // An optional list that was not given has no items and hashes as an empty one would not:
          // the null is its own value.
          source.WriteLine(field.Required
                             ? $"foreach (var item in {field.Name})"
                             : $"foreach (var item in {field.Name} ?? global::System.Linq.Enumerable.Empty<{field.Type}>())");
          source.WriteLine("{");
          source.Indent++;
          source.WriteLine(field.Type == "string"
                             ? "hash = hash * 31 + (item?.GetHashCode() ?? 0);"
                             : "hash = hash * 31 + item.GetHashCode();");
          source.Indent--;
          source.WriteLine("}");

          if (!field.Required)
          {
            source.WriteLine($"hash = hash * 31 + ({field.Name} is null ? 0 : 1);");
          }
        }
        else
        {
          source.WriteLine($"hash = hash * 31 + ({field.Name}?.GetHashCode() ?? 0);");
        }
      }

      Blank(source);
      source.WriteLine("return hash;");
      source.Indent--;
      source.WriteLine("}");
      Blank(source);
    }

    // The engine reads an alternative that carries nothing as its name, and any other as an object
    // of one key naming it, which no serializer policy writes from a record: written here, by the
    // schema's names. Only written, because options go one way, to the engine.
    private static void AppendConverter(IndentedTextWriter source,
                                        OptionChoice choice)
    {
      var converter = Converter(choice.Name);

      Lines(source,
            $$"""
              /// <summary>Writes a <see cref="{{choice.Name}}" /> as the engine reads one: the name of an alternative that carries nothing, else an object whose one key names the alternative.</summary>
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

        if (alternative.Shape != AlternativeShape.Unit)
        {
          source.WriteLine("writer.WriteStartObject();");
        }

        switch (alternative.Shape)
        {
          case AlternativeShape.Unit:
            source.WriteLine($"writer.WriteStringValue(\"{alternative.Name}\");");
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
              source.WriteLine($"if ({local}.{field.Name} is {(field.Kind == OptionKind.EnumerationList ? ParameterType(field) : field.Type)} {value})");
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

        if (alternative.Shape != AlternativeShape.Unit)
        {
          source.WriteLine("writer.WriteEndObject();");
        }

        source.WriteLine("break;");
        source.Indent--;
        source.WriteLine("}");
      }

      source.Indent -= 3;

      Lines(source,
            """
                  }
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

        case OptionKind.EnumerationList:
          source.WriteLine($"writer.WriteStartArray({key});");
          source.WriteLine($"foreach (var item in {value})");
          source.WriteLine("{");
          source.Indent++;
          source.WriteLine(option.Type == "string"
                             ? "writer.WriteStringValue(item);"
                             : "writer.WriteStringValue(item.ToString());");
          source.Indent--;
          source.WriteLine("}");
          source.WriteLine("writer.WriteEndArray();");
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
      var checks = options.Where(option => (option.Kind == OptionKind.EnumerationList && option.Type != "string") ||
                                           option.Kind == OptionKind.Enumeration ||
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

          var block = new List<string>
                      {
                        $"if ({option.Name} is {{ }} {value})",
                        "{",
                      };

          // The count of the names a list holds, which is the one bound a list here states.
          if (option.Bounds.FirstOrDefault(bound => bound.Keyword == "minItems") is { Keyword: not null } count)
          {
            block.Add($"  if ({value}.Count < {count.Literal})");
            block.Add("  {");
            block.Add($"    throw new ArgumentOutOfRangeException(nameof({option.Name}),");
            block.Add($"    {under}{value}.Count,");
            block.Add($"    {under}\"{option.Name} has to name at least {count.Literal} {(count.Literal == "1" ? "item" : "items")}.\");");
            block.Add("  }");
            block.Add(string.Empty);
          }

          block.Add($"  var {undeclared} = {value}.Where({item} => !Enum.IsDefined(typeof({option.Type}), {item}))");
          block.Add($"  {chain}.ToList();");
          block.Add(string.Empty);
          block.Add($"  if ({undeclared}.Count > 0)");
          block.Add("  {");
          block.Add($"    throw new ArgumentOutOfRangeException(nameof({option.Name}),");
          block.Add($"    {under}string.Join(\", \",");
          block.Add($"    {under}            {undeclared}),");
          block.Add($"    {under}\"{option.Name} has to be names {option.Type} declares.\");");
          block.Add("  }");
          block.Add("}");

          Lines(source,
                string.Join("\n",
                            block));
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
    ///   option is a Rust field renamed. The name of an enumeration's member may also be capitals
    ///   and digits joined by underscores, which is how a status is spelled and which all four take
    ///   as they take letters. Anything else is a compile error in a file nobody wrote, which is a
    ///   poor way to learn that a schema said something this cannot write.
    /// </remarks>
    private static void RefuseAName(string name,
                                    string what,
                                    bool capitals = false)
    {
      if (!PascalCase.IsMatch(name) && !(capitals && Capitals.IsMatch(name)))
      {
        throw new NotSupportedException($"`{name}` names {what}, and this generator writes a name of PascalCase letters and digits{(capitals ? ", or of capitals and digits joined by underscores" : "")}.");
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

    // A name as gRFC A6 spells a status: capitals and digits, joined by single underscores.
    private static readonly Regex Capitals = new(@"\A[A-Z][A-Z0-9]*(_[A-Z0-9]+)*\z",
                                                 RegexOptions.CultureInvariant);

    // What a generated type already declares, or inherits and would shadow without saying so.
    // `Encode` is the root's, and a group has none - listed for every class all the same, because
    // which group is the root is no reason for an option to be spelled one way here.
    // `Deconstruct`, `EqualityContract` and `PrintMembers` are what C# declares on every record.
    private static readonly HashSet<string> Members = new(StringComparer.Ordinal)
                                                      {
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
