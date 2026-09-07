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
using System.Linq;
using System.Reflection;
using System.Text;
using System.Threading.Tasks;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator.Tests
{
  [TestFixture]
  public class GeneratorTest
  {
    private static async Task<string> Render(string schema)
      => CSharpSource.Render(await OptionVocabulary.ReadAsync(schema)
                                                   .ConfigureAwait(false),
                             "Test",
                             "test.schema.json");

    /// <summary>A root schema around <paramref name="properties" />.</summary>
    /// <remarks>
    ///   It states a `description` because every option and group of this vocabulary documents
    ///   itself, and a fixture that left one out would be testing the refusal rather than what it
    ///   set out to. `Undescribed` is the fixture for that.
    /// </remarks>
    private static string Wrap(string properties,
                               string defs = "")
      => $@"{{
  ""$schema"": ""https://json-schema.org/draft/2020-12/schema"",
  ""title"": ""Options"",
  ""description"": ""What a caller may set."",
  ""type"": ""object"",
  ""properties"": {{{properties}}},
  ""additionalProperties"": false{defs}
}}";

    private const string Documented = @"""description"": ""What this option does."", ";

    [Test]
    public async Task AnOptionBecomesANullableSettableProperty()
    {
      var rendered = await Render(Wrap($@"""Credits"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"" }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("public int? Credits { get; set; }"));
      Assert.That(rendered,
                  Does.Contain(@"[JsonPropertyName(""Credits"")]"));
      Assert.That(rendered,
                  Does.Contain("[JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]"));
    }

    /// <summary>
    ///   Two bounds on one option are one check, because two are two locals of the same name.
    /// </summary>
    /// <remarks>
    ///   A check per keyword reads more simply and does not compile: `is int credits` twice in a
    ///   block is CS0128. The message names the whole admissible range for the same reason a
    ///   caller who set zero wants to be told what to set instead.
    /// </remarks>
    [Test]
    public async Task TwoBoundsOnOneOptionAreOneCheck()
    {
      var rendered = await Render(Wrap($@"""Credits"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"", ""minimum"": 1, ""maximum"": 9 }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("if (Credits is int credits && (credits < 1 || credits > 9))"));
      Assert.That(rendered,
                  Does.Contain(@"""Credits has to be at least 1 and at most 9."""));
      Assert.That(rendered.Split("is int credits")
                          .Length - 1,
                  Is.EqualTo(1),
                  "one local, or the generated file does not compile");
    }

    /// <summary>A property states its own keywords beside a `$ref`, which 2020-12 allows.</summary>
    /// <remarks>
    ///   The type comes from the reference and the bound from the property, so reading only one of
    ///   the two loses either the type or the constraint. This is the shape ConnectTimeoutSeconds
    ///   has.
    /// </remarks>
    [Test]
    public async Task AReferenceKeepsTheKeywordsStatedBesideIt()
    {
      var rendered = await Render(Wrap($@"""Timeout"": {{ {Documented}""$ref"": ""#/$defs/Seconds"", ""exclusiveMinimum"": 0.0 }}",
                                       @",
  ""$defs"": { ""Seconds"": { ""type"": ""number"", ""format"": ""double"" } }"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("public double? Timeout { get; set; }"),
                  "the type comes through the reference");
      Assert.That(rendered,
                  Does.Contain("timeout <= 0"),
                  "the bound stated beside the reference is kept");
    }

    [Test]
    public async Task AGroupBecomesItsOwnClassAndIsValidatedThroughItsOwner()
    {
      var rendered = await Render(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                       @",
  ""$defs"": {
    ""TransportOptions"": {
      ""type"": ""object"",
      ""description"": ""What the transport does."",
      ""properties"": { ""Timeout"": { ""description"": ""How long."", ""type"": ""number"", ""format"": ""double"", ""minimum"": 1 } },
      ""additionalProperties"": false
    }
  }"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("public TransportOptions? Transport { get; set; }"));
      Assert.That(rendered,
                  Does.Contain("internal sealed partial class TransportOptions"));
      Assert.That(rendered,
                  Does.Contain("Transport?.Validate();"));
    }

    /// <summary>A description is documentation, and its paragraphs are the XML's two elements.</summary>
    [Test]
    public async Task ADescriptionBecomesTheDocumentation()
    {
      var rendered = await Render(Wrap(@"""Credits"": {
    ""description"": ""What it is.\n\nWhat a caller has to know, mentioning `code`, a < and an &."",
    ""type"": ""integer"",
    ""format"": ""int32""
  }"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("/// <summary>What it is.</summary>"));
      Assert.That(rendered,
                  Does.Contain("<c>code</c>"),
                  "a backtick is code");
      Assert.That(rendered,
                  Does.Contain("a &lt; and an &amp;"),
                  "and XML's own characters are escaped");
      Assert.That(rendered,
                  Does.Contain("/// <remarks>"));
    }

    /// <summary>A bound nobody turns into a check stops the generator.</summary>
    /// <remarks>
    ///   Emitting the property and dropping the keyword would be a value a caller can set, the
    ///   engine refuses, and nothing on this side saw - a failure at the far end of the ABI. The
    ///   schema grows with every feature, so this is the case that arrives on its own.
    /// </remarks>
    [Test]
    public void ABoundThisGeneratorCannotCheckIsRefused()
    {
      var unbounding = OptionVocabulary.Unhandled(Wrap(@"""Name"": { ""type"": ""string"", ""pattern"": ""^a"" }"));

      Assert.That(unbounding,
                  Is.EqualTo(new[]
                             {
                               "pattern",
                             }));
    }

    [Test]
    public void EveryBoundTheGeneratorChecksIsOneItAlsoAccepts()
    {
      foreach (var keyword in OptionVocabulary.Keywords)
      {
        Assert.That(OptionVocabulary.Unhandled($@"{{ ""type"": ""object"", ""additionalProperties"": false, ""properties"": {{ ""X"": {{ ""{keyword}"": 1 }} }} }}"),
                    Is.Empty,
                    $"`{keyword}` is checked and should not be reported unhandled");
      }
    }

    [Test]
    public void ATypeThisGeneratorHasNoCSharpForIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Names"": {{ {Documented}""type"": ""array"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("Names"));

    /// <summary>A bound stated twice applies twice, so the check is the stricter of the two.</summary>
    /// <remarks>
    ///   Draft 2020-12 applies a `$ref` and the keywords beside it together. Taking the property's
    ///   own would generate a class admitting what the reference forbids, and the engine - which
    ///   reads the same schema - would refuse it across the ABI, naming neither.
    /// </remarks>
    [Test]
    public async Task OfTwoBoundsThatBothApplyTheStricterIsChecked()
    {
      var rendered = await Render(Wrap($@"""Credits"": {{ {Documented}""$ref"": ""#/$defs/Window"", ""minimum"": 1, ""maximum"": 900 }}",
                                       @",
  ""$defs"": {
    ""Window"": { ""description"": ""A window."", ""type"": ""integer"", ""format"": ""int32"", ""minimum"": 5, ""maximum"": 99 }
  }"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("(credits < 5 || credits > 99)"),
                  "the larger lower bound and the smaller upper one");
      Assert.That(rendered,
                  Does.Not.Contain("credits < 1"));
      Assert.That(rendered,
                  Does.Not.Contain("credits > 900"));
    }

    /// <summary>A cycle of references resolves to no type, and says so rather than ending.</summary>
    /// <remarks>
    ///   Followed blindly this is a StackOverflowException, which .NET cannot catch: the tool
    ///   would take the build process down with no message naming the schema.
    /// </remarks>
    [Test]
    public void ACycleOfReferencesIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""X"": {{ {Documented}""$ref"": ""#/$defs/A"" }}",
                                                   @",
  ""$defs"": {
    ""A"": { ""$ref"": ""#/$defs/B"" },
    ""B"": { ""$ref"": ""#/$defs/A"" }
  }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("cycle"));

    /// <summary>Two schemas of one name would be one class declared twice.</summary>
    [Test]
    public void TwoSchemasOfOneNameAreRefused()
      => Assert.That(async () => await Render(Wrap($@"""First"": {{ {Documented}""$ref"": ""#/$defs/Group"" }},
  ""Second"": {{ {Documented}""$ref"": ""#/$defs/Other"" }}",
                                                   @",
  ""$defs"": {
    ""Group"": { ""title"": ""Same"", ""description"": ""One."", ""type"": ""object"", ""additionalProperties"": false, ""properties"": { ""A"": { ""description"": ""A."", ""type"": ""string"" } } },
    ""Other"": { ""title"": ""Same"", ""description"": ""Two."", ""type"": ""object"", ""additionalProperties"": false, ""properties"": { ""B"": { ""description"": ""B."", ""type"": ""string"" } } }
  }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("Same"));

    [Test]
    public void AnObjectStatingNoPropertiesIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Bag"": {{ {Documented}""type"": ""object"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("no properties"));

    /// <summary>A name reaches four places in the emitted C#, and none of them quotes it.</summary>
    /// <remarks>
    ///   An identifier, a `nameof`, and two string literals. A name that is not PascalCase letters
    ///   and digits is a compile error in a file nobody wrote, which is a poor way to learn that
    ///   a schema said something the generator cannot write. The trailing-newline case is why the
    ///   pattern is anchored with `\z` and not `$`: outside multiline mode `$` matches before a
    ///   single trailing newline, so `^...$` admits a name that does not close a string literal.
    /// </remarks>
    [TestCase("class", TestName = "AName_AKeyword")]
    [TestCase("1Bad", TestName = "AName_LeadingDigit")]
    [TestCase("My-Name", TestName = "AName_Punctuation")]
    [TestCase("Max Sends", TestName = "AName_Space")]
    [TestCase("credits", TestName = "AName_LowerCase")]
    [TestCase("Trailing\\n", TestName = "AName_TrailingNewline")]
    [TestCase("Validate", TestName = "AName_AMemberEveryClassHas")]
    public void ANameThisGeneratorCannotWriteIsRefused(string name)
      => Assert.That(async () => await Render(Wrap($@"""{name}"": {{ {Documented}""type"": ""string"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>());

    /// <summary>A name spelled with a JSON escape is the name it decodes to, on both sides.</summary>
    /// <remarks>
    ///   The identifier and the wire name are built from the same decoded string, so no escape
    ///   can make them disagree - which is worth a test rather than a reader's confidence, since
    ///   a property that serialized under a name the engine never declared would be refused
    ///   across the ABI and named by neither side.
    /// </remarks>
    [Test]
    public async Task ANameSpelledWithAnEscapeIsUsedDecoded()
    {
      var rendered = await Render(Wrap($@"""A\u0062"": {{ {Documented}""type"": ""string"" }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("public string? Ab { get; set; }"));
      Assert.That(rendered,
                  Does.Contain(@"[JsonPropertyName(""Ab"")]"));
    }

    /// <summary>A description carrying a Unicode line terminator still yields a readable file.</summary>
    /// <remarks>
    ///   C# ends a line on U+0085, U+2028 and U+2029 as it does on a carriage return, so one left
    ///   inside a description would put the rest of it outside the `///` and into the code.
    /// </remarks>
    [TestCase("\u0085", TestName = "ATerminator_NextLine")]
    [TestCase("\u2028", TestName = "ATerminator_LineSeparator")]
    [TestCase("\u2029", TestName = "ATerminator_ParagraphSeparator")]
    public async Task ADescriptionKeepsEveryLineBehindItsSlashes(string terminator)
    {
      var rendered = await Render(Wrap($@"""X"": {{ ""description"": ""first{terminator}second"", ""type"": ""string"" }}"))
                       .ConfigureAwait(false);

      foreach (var line in rendered.Split('\n')
                                   .Where(line => line.Contains("second")))
      {
        Assert.That(line.TrimStart(),
                    Does.StartWith("///"),
                    "every line of a description stays a comment");
      }
    }

    /// <summary>An option whose local would be a keyword still compiles.</summary>
    /// <remarks>
    ///   `Default` is a name the vocabulary admits and `default` is not an identifier, so the
    ///   verbatim form is what makes the pattern's local one.
    /// </remarks>
    [Test]
    public async Task AnOptionWhoseLocalIsAKeywordIsWrittenVerbatim()
    {
      var rendered = await Render(Wrap($@"""Default"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"", ""minimum"": 1 }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("if (Default is int @default && @default < 1)"));
    }

    /// <summary>A keyword that asserts nothing about its type has no check to emit.</summary>
    /// <remarks>
    ///   Draft 2020-12 lets a schema state `minLength` beside a number, where it asserts nothing.
    ///   Emitted regardless it would be `.Length` on an `int`, which does not compile - so the
    ///   generator stops instead, naming the keyword and the type.
    /// </remarks>
    [TestCase(@"""type"": ""integer"", ""format"": ""int32"", ""minLength"": 3", TestName = "AMismatch_LengthOnANumber")]
    [TestCase(@"""type"": ""string"", ""minimum"": 1", TestName = "AMismatch_MinimumOnText")]
    [TestCase(@"""type"": ""boolean"", ""minimum"": 1", TestName = "AMismatch_MinimumOnABoolean")]
    public void ABoundThatDoesNotFitItsTypeIsRefused(string schema)
      => Assert.That(async () => await Render(Wrap($@"""X"": {{ {Documented}{schema} }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("bounds no"));

    /// <summary>A `double` is wider than the `number` the schema describes.</summary>
    /// <remarks>
    ///   NaN passes every bound, because every comparison against it is false, and
    ///   System.Text.Json refuses to write NaN or either infinity - so a caller who set one would
    ///   get its exception instead of one naming the option.
    /// </remarks>
    [Test]
    public async Task ANumberIsRefusedWhenItIsNotFinite()
    {
      var rendered = await Render(Wrap($@"""Ratio"": {{ {Documented}""type"": ""number"", ""format"": ""double"" }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("double.IsNaN(ratio) || double.IsInfinity(ratio)"));
      Assert.That(rendered,
                  Does.Contain(@"""Ratio has to be finite."""));
    }

    /// <summary>A pointer spells `/` as `~1`, so the two forms have to be compared as one.</summary>
    /// <remarks>
    ///   Corvus unescapes correctly and would follow this reference for ever; comparing the raw
    ///   member name against the escaped pointer misses the edge, and the miss is the stack
    ///   overflow the guard exists to prevent.
    /// </remarks>
    [Test]
    public void ACycleThroughAnEscapedPointerIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""X"": {{ {Documented}""$ref"": ""#/$defs/A~1B"" }}",
                                                   @",
  ""$defs"": { ""A/B"": { ""$ref"": ""#/$defs/A~1B"" } }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>());

    /// <summary>A reference to a schema that contains it is a circle too.</summary>
    /// <remarks>
    ///   No chain of `$ref`s shows this one - the edge leaves the ancestor by a different member
    ///   each time round - and the class it would generate holds a property of its own type,
    ///   whose generated `Validate()` recurses with nothing to stop it.
    /// </remarks>
    [Test]
    public void AReferenceToAnEnclosingSchemaIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Self"": {{ {Documented}""$ref"": ""#"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("contains it"));

    /// <summary>`required` reaches no check, so it has to be reported rather than dropped.</summary>
    /// <remarks>
    ///   Nothing generated verifies that an option was set, and every option of this vocabulary
    ///   has a default - but a future schema stating `required` would silently lose it.
    /// </remarks>
    [Test]
    public void ARequiredOptionIsReportedBecauseNothingChecksPresence()
      => Assert.That(OptionVocabulary.Unhandled(@"{
  ""type"": ""object"",
  ""properties"": { ""X"": { ""type"": ""string"" } },
  ""required"": [""X""],
  ""additionalProperties"": false
}"),
                     Is.EqualTo(new[]
                                {
                                  "required",
                                }));

    /// <summary>A subschema container is walked, so what is unhandled inside it is named.</summary>
    /// <remarks>
    ///   `patternProperties` and `dependentSchemas` hold subschemas this generator reads as
    ///   nothing, and reporting the outer keyword alone would say that something was dropped
    ///   without saying what - so both the container and its contents are reported.
    /// </remarks>
    [TestCase("patternProperties", TestName = "AContainer_PatternProperties")]
    [TestCase("dependentSchemas", TestName = "AContainer_DependentSchemas")]
    public void AContainerThisGeneratorReadsAsNothingIsWalkedAndReported(string container)
      => Assert.That(OptionVocabulary.Unhandled($@"{{
  ""type"": ""object"",
  ""additionalProperties"": false,
  ""properties"": {{ ""X"": {{ ""type"": ""string"" }} }},
  ""{container}"": {{ ""Y"": {{ ""type"": ""string"", ""pattern"": ""^a"" }} }}
}}"),
                     Is.EqualTo(new[]
                                {
                                  "pattern",
                                  container,
                                }),
                     "the keyword inside the container is named, and so is the container");

    /// <summary>Only `additionalProperties: false` closes an object; the rest is an open map.</summary>
    [Test]
    public void AnObjectLeftOpenIsReported()
    {
      Assert.That(OptionVocabulary.Unhandled(@"{ ""type"": ""object"", ""properties"": {}, ""additionalProperties"": { ""type"": ""string"" } }"),
                  Is.EqualTo(new[]
                             {
                               "additionalProperties",
                             }));

      Assert.That(OptionVocabulary.Unhandled(@"{ ""type"": ""object"", ""properties"": {}, ""additionalProperties"": false }"),
                  Is.Empty,
                  "the closed form is what a generated class already is");
    }

    /// <summary>A bound written beside a reference to a group is refused, not read.</summary>
    /// <remarks>
    ///   Stated there it binds one embedding and not the next, which is the opposite of what a
    ///   `$defs` entry is for - so the schema has to say it in the group, where it means the same
    ///   thing everywhere the group is used. Read instead, it would be dropped in silence: the
    ///   group path built its option without consulting a single keyword.
    /// </remarks>
    [Test]
    public void ABoundBesideAReferenceToAGroupIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"", ""minimum"": 1 }}",
                                                   @",
  ""$defs"": {
    ""TransportOptions"": {
      ""description"": ""What the transport does."",
      ""additionalProperties"": false,
      ""type"": ""object"",
      ""properties"": { ""Timeout"": { ""description"": ""How long."", ""type"": ""number"", ""format"": ""double"" } }
    }
  }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("bounds no group"));

    /// <summary>Two bounds are compared as integers, so one past 2^53 does not move.</summary>
    /// <remarks>
    ///   A `double` holds 53 bits of mantissa, so 9007199254740993 and 9007199254740992 are one
    ///   number to it: `>` is false either way, and comparing that way always picks the
    ///   reference's. The larger is on the property here, so that choice is the wrong one - put
    ///   the other way round, a double comparison lands on the right answer by luck and the test
    ///   says nothing.
    /// </remarks>
    [Test]
    public async Task TwoLargeBoundsAreComparedWithoutLosingPrecision()
    {
      var rendered = await Render(Wrap($@"""Count"": {{ {Documented}""$ref"": ""#/$defs/Big"", ""minimum"": 9007199254740993 }}",
                                       @",
  ""$defs"": {
    ""Big"": { ""description"": ""A count."", ""type"": ""integer"", ""format"": ""int64"", ""minimum"": 9007199254740992 }
  }"))
                       .ConfigureAwait(false);

      Assert.That(rendered,
                  Does.Contain("count < 9007199254740993"),
                  "the larger of the two lower bounds, which a double cannot tell apart");
      Assert.That(rendered,
                  Does.Not.Contain("count < 9007199254740992"));
    }

    /// <summary>Every option and every group documents itself, or the schema is refused.</summary>
    /// <remarks>
    ///   The documentation is written once as a Rust doc comment and lands in a .NET caller's
    ///   tooltip. Absent, a caller has to read the engine's source to learn what an option does;
    ///   empty, the doc comment exists and says nothing.
    /// </remarks>
    [TestCase(@"""type"": ""string""", TestName = "ADescription_Absent")]
    [TestCase(@"""description"": """", ""type"": ""string""", TestName = "ADescription_Empty")]
    [TestCase(@"""description"": ""   "", ""type"": ""string""", TestName = "ADescription_Blank")]
    public void AnOptionThatDocumentsNothingIsRefused(string schema)
      => Assert.That(async () => await Render(Wrap($@"""X"": {{ {schema} }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("documents itself"));

    [Test]
    public void AGroupThatDocumentsNothingIsRefused()
      => Assert.That(async () => await Render($@"{{
  ""$schema"": ""https://json-schema.org/draft/2020-12/schema"",
  ""title"": ""Options"",
  ""type"": ""object"",
  ""additionalProperties"": false,
  ""properties"": {{ ""X"": {{ {Documented}""type"": ""string"" }} }}
}}")
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("documents itself"));

    /// <summary>The committed class is what the committed schema renders.</summary>
    /// <remarks>
    ///   The build checks this too, so that a stale file is a compile failure rather than a
    ///   surprise. Here as well because a test says which two files disagree and how, and because
    ///   a build that is never run checks nothing.
    /// </remarks>
    [Test]
    public async Task TheCommittedClassIsWhatTheCommittedSchemaRenders()
    {
      var schemaPath = Metadata("OptionsSchema");
      var classPath  = Metadata("GeneratedOptions");

      var rendered = CSharpSource.Render(await OptionVocabulary.ReadAsync(File.ReadAllText(schemaPath))
                                                               .ConfigureAwait(false),
                                         "ArmoniK.Api.Client.RustGrpcChannel",
                                         Path.GetFileName(schemaPath));

      // Bytes, not text: `File.ReadAllText` would strip a byte order mark the generator never
      // writes, so a file that picked one up would compare equal to output that has none.
      Assert.That(new UTF8Encoding(false).GetString(File.ReadAllBytes(classPath))
                                         .Replace("\r\n",
                                                  "\n"),
                  Is.EqualTo(rendered),
                  "the schema changed and the class did not; write it again with\n"
                  + "  dotnet run --project packages/csharp/tools/ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator -- "
                  + "--schema packages/rust/armonik-transport/options.schema.json "
                  + "--output packages/csharp/ArmoniK.Api.Client.RustGrpcChannel/ChannelOptions.g.cs");
    }

    private static string Metadata(string key)
      => Assembly.GetExecutingAssembly()
                 .GetCustomAttributes<AssemblyMetadataAttribute>()
                 .Single(attribute => attribute.Key == key)
                 .Value
         ?? throw new InvalidOperationException($"`{key}` names no path.");
  }
}
