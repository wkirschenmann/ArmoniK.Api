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
                  Does.Contain("public sealed class TransportOptions"));
      Assert.That(rendered,
                  Does.Contain("Transport?.Validate();"));
      Assert.That(rendered,
                  Does.Contain("<summary>What this option does.</summary>"),
                  "the option is documented by what it states, not by what its group states");
    }

    /// <summary>A root holding one group renders its classes to exactly this.</summary>
    /// <remarks>
    ///   The classes as whole text, because the other tests read fragments and the committed
    ///   schema has only its own shapes. This one pins two that schema lacks: a `Validate` holding
    ///   only groups, and one that bounds nothing.
    /// </remarks>
    [Test]
    public async Task ARootHoldingOneGroupRendersItsClassesToExactlyThisText()
    {
      var rendered = await Render(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                       @",
  ""$defs"": {
    ""TransportOptions"": {
      ""description"": ""What the transport does."",
      ""type"": ""object"",
      ""properties"": { ""Name"": { ""description"": ""A name."", ""type"": ""string"" } },
      ""additionalProperties"": false
    }
  }"))
                       .ConfigureAwait(false);

      const string classes = @"/// <summary>What a caller may set.</summary>
public sealed class Options
{
  /// <summary>Options nobody has set.</summary>
  public Options()
  {
  }

  /// <summary>A copy of <paramref name=""other"" />, sharing nothing with it.</summary>
  /// <param name=""other"">The options to copy.</param>
  /// <exception cref=""ArgumentNullException""><paramref name=""other"" /> is null.</exception>
  public Options(Options other)
  {
    if (other is null)
    {
      throw new ArgumentNullException(nameof(other));
    }

    Transport = other.Transport is null
                  ? null
                  : new TransportOptions(other.Transport);
  }

  /// <summary>What this option does.</summary>
  [JsonPropertyName(""Transport"")]
  [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
  public TransportOptions? Transport { get; set; }

  /// <summary>Refuses an option outside the range the engine accepts.</summary>
  /// <exception cref=""ArgumentOutOfRangeException"">An option is outside its stated bounds.</exception>
  public void Validate()
  {
    Transport?.Validate();
  }

  /// <summary>The document the engine reads, as UTF-8.</summary>
  /// <returns>The options as JSON, without the ones left unset.</returns>
  /// <exception cref=""ArgumentOutOfRangeException"">An option is outside its bounds.</exception>
  /// <remarks>
  ///   Checked before it is written, not after it is refused: the engine answers a bad
  ///   document with a status naming neither the option nor the bound.
  /// </remarks>
  internal byte[] Encode()
  {
    Validate();

    return JsonSerializer.SerializeToUtf8Bytes(this,
                                               OptionsJsonContext.Default.Options);
  }
}

/// <summary>What the transport does.</summary>
public sealed class TransportOptions
{
  /// <summary>Options nobody has set.</summary>
  public TransportOptions()
  {
  }

  /// <summary>A copy of <paramref name=""other"" />, sharing nothing with it.</summary>
  /// <param name=""other"">The options to copy.</param>
  /// <exception cref=""ArgumentNullException""><paramref name=""other"" /> is null.</exception>
  public TransportOptions(TransportOptions other)
  {
    if (other is null)
    {
      throw new ArgumentNullException(nameof(other));
    }

    Name = other.Name;
  }

  /// <summary>A name.</summary>
  [JsonPropertyName(""Name"")]
  [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
  public string? Name { get; set; }

  /// <summary>Refuses an option outside the range the engine accepts.</summary>
  /// <exception cref=""ArgumentOutOfRangeException"">An option is outside its stated bounds.</exception>
  public void Validate()
  {
    // The schema bounds nothing here.
  }
}
";

      Assert.That(rendered,
                  Does.Contain(classes.Replace("\r\n",
                                               "\n")));
    }

    /// <summary>A choice, its alternatives of each shape, and an enumeration, rendered to exactly this.</summary>
    /// <remarks>
    ///   A Rust enum renders as a `oneOf`: of objects of one key for one whose variants carry
    ///   something, of constants for one whose variants carry nothing. The first is a closed
    ///   hierarchy of records, with its writer; the second a C# enum.
    /// </remarks>
    [Test]
    public async Task AChoiceRendersItsRecordsAndWriterToExactlyThisText()
    {
      var rendered = await Render(Choice)
                       .ConfigureAwait(false);

      const string choice = @"/// <summary>How a peer is verified.</summary>
[JsonConverter(typeof(VerificationJsonConverter))]
public abstract record Verification
{
  private Verification()
  {
  }

  /// <summary>Against a pinned key.</summary>
  /// <param name=""Value"">Against a pinned key.</param>
  public sealed record Pinned(string Value) : Verification
  {
    /// <summary>Against a pinned key.</summary>
    public string Value { get; init; } = Value ?? throw new ArgumentNullException(nameof(Value));

    /// <inheritdoc />
    public override void Validate()
    {
      if (Value is string value && value.Length < 1)
      {
        throw new ArgumentOutOfRangeException(nameof(Value),
                                              value,
                                              ""Value has to be at least 1 character long."");
      }
    }
  }

  /// <summary>Against a store.</summary>
  /// <param name=""Path"">Its path.</param>
  /// <param name=""Where"">Where it is.</param>
  public sealed record Store(string Path,
                             Place? Where = null) : Verification
  {
    /// <summary>Its path.</summary>
    public string Path { get; init; } = Path ?? throw new ArgumentNullException(nameof(Path));

    /// <inheritdoc />
    public override void Validate()
    {
      if (Where is Place where && !Enum.IsDefined(typeof(Place), where))
      {
        throw new ArgumentOutOfRangeException(nameof(Where),
                                              where,
                                              ""Where has to be a name Place declares."");
      }
    }
  }

  /// <summary>Not at all.</summary>
  public sealed record Unchecked : Verification
  {
    /// <inheritdoc />
    public override void Validate()
    {
      // The schema bounds nothing here.
    }
  }

  /// <summary>Refuses a field outside the range the engine accepts.</summary>
  /// <exception cref=""ArgumentOutOfRangeException"">A field is outside its stated bounds.</exception>
  public abstract void Validate();
}

/// <summary>Writes a <see cref=""Verification"" /> as the engine reads one: an object whose one key names the alternative.</summary>
internal sealed class VerificationJsonConverter : JsonConverter<Verification>
{
  /// <inheritdoc />
  /// <remarks>Options go to the engine and nothing reads them back, so this reads nothing.</remarks>
  public override Verification? Read(ref Utf8JsonReader reader,
                                     Type typeToConvert,
                                     JsonSerializerOptions options)
    => throw new NotSupportedException(""Verification is written to the engine, and never read back."");

  /// <inheritdoc />
  public override void Write(Utf8JsonWriter writer,
                             Verification value,
                             JsonSerializerOptions options)
    => WriteValue(writer,
                  value);

  /// <summary>Writes <paramref name=""written"" />, as the converter of a choice holding one does too.</summary>
  /// <param name=""writer"">Where it is written.</param>
  /// <param name=""written"">The alternative.</param>
  internal static void WriteValue(Utf8JsonWriter writer,
                                  Verification written)
  {
    writer.WriteStartObject();

    switch (written)
    {
      case Verification.Pinned pinned:
      {
        writer.WriteString(""Pinned"",
                           pinned.Value);
        break;
      }

      case Verification.Store store:
      {
        writer.WriteStartObject(""Store"");

        if (store.Where is Place where)
        {
          writer.WriteString(""Where"",
                             where.ToString());
        }

        writer.WriteString(""Path"",
                           store.Path);
        writer.WriteEndObject();
        break;
      }

      case Verification.Unchecked:
      {
        writer.WriteBoolean(""Unchecked"",
                            true);
        break;
      }
    }

    writer.WriteEndObject();
  }
}

/// <summary>Where a store is.</summary>
[JsonConverter(typeof(JsonStringEnumConverter<Place>))]
public enum Place
{
  /// <summary>Here.</summary>
  Here,

  /// <summary>There.</summary>
  There,
}
";

      Assert.That(rendered,
                  Does.Contain(choice.Replace("\r\n",
                                              "\n")));
    }

    private const string Choice = @"{
  ""$schema"": ""https://json-schema.org/draft/2020-12/schema"",
  ""title"": ""Options"",
  ""description"": ""What a caller may set."",
  ""type"": ""object"",
  ""properties"": {""Verify"": { ""description"": ""How the peer is verified."", ""$ref"": ""#/$defs/Verification"" }},
  ""additionalProperties"": false,
  ""$defs"": {
    ""Verification"": {
      ""description"": ""How a peer is verified."",
      ""oneOf"": [
        { ""description"": ""Against a pinned key."", ""type"": ""object"", ""properties"": { ""Pinned"": { ""type"": ""string"", ""minLength"": 1 } }, ""additionalProperties"": false, ""required"": [""Pinned""] },
        { ""description"": ""Against a store."", ""type"": ""object"", ""properties"": { ""Store"": { ""$ref"": ""#/$defs/Store"" } }, ""additionalProperties"": false, ""required"": [""Store""] },
        { ""description"": ""Not at all."", ""type"": ""object"", ""properties"": { ""Unchecked"": { ""const"": true } }, ""additionalProperties"": false, ""required"": [""Unchecked""] }
      ]
    },
    ""Store"": {
      ""description"": ""A store."",
      ""type"": ""object"",
      ""properties"": {
        ""Where"": { ""description"": ""Where it is."", ""$ref"": ""#/$defs/Place"" },
        ""Path"": { ""description"": ""Its path."", ""type"": ""string"" }
      },
      ""required"": [""Path""],
      ""additionalProperties"": false
    },
    ""Place"": {
      ""description"": ""Where a store is."",
      ""oneOf"": [
        { ""description"": ""Here."", ""type"": ""string"", ""const"": ""Here"" },
        { ""description"": ""There."", ""type"": ""string"", ""const"": ""There"" }
      ]
    }
  }
}";

    /// <summary>The fixture with one alternative replaced by <paramref name="alternative" />.</summary>
    private static string ChoiceWith(string alternative)
      => Choice.Replace(@"{ ""description"": ""Not at all."", ""type"": ""object"", ""properties"": { ""Unchecked"": { ""const"": true } }, ""additionalProperties"": false, ""required"": [""Unchecked""] }",
                        alternative);

    /// <summary>An alternative is an object of one required key; anything else names no alternative.</summary>
    [TestCase(@"{ ""description"": ""Two."", ""type"": ""object"", ""properties"": { ""A"": { ""const"": true }, ""B"": { ""const"": true } }, ""additionalProperties"": false, ""required"": [""A"", ""B""] }",
              "one required key",
              TestName = "AnAlternative_TwoKeys")]
    [TestCase(@"{ ""description"": ""Optional."", ""type"": ""object"", ""properties"": { ""A"": { ""const"": true } }, ""additionalProperties"": false }",
              "one required key",
              TestName = "AnAlternative_Optional")]
    [TestCase(@"{ ""description"": ""False."", ""type"": ""object"", ""properties"": { ""A"": { ""const"": false } }, ""additionalProperties"": false, ""required"": [""A""] }",
              "other than `true`",
              TestName = "AnAlternative_ConstantFalse")]
    [TestCase(@"{ ""description"": ""Bare."", ""type"": ""object"", ""properties"": { ""A"": { ""$ref"": ""#/$defs/Place"" } }, ""additionalProperties"": false, ""required"": [""A""] }",
              "carries a choice bare",
              TestName = "AnAlternative_BareChoice")]
    [TestCase(@"{ ""description"": ""Group."", ""type"": ""object"", ""properties"": { ""A"": { ""type"": ""object"", ""properties"": { ""Inner"": { ""description"": ""Inner."", ""$ref"": ""#/$defs/Held"" } }, ""additionalProperties"": false } }, ""additionalProperties"": false, ""required"": [""A""] }",
              "an alternative holds none",
              TestName = "AnAlternative_HoldingAGroup")]
    [TestCase(@"{ ""description"": ""Plumbing."", ""type"": ""object"", ""properties"": { ""A"": { ""type"": ""object"", ""properties"": { ""Writer"": { ""description"": ""Writer."", ""type"": ""string"" } }, ""additionalProperties"": false } }, ""additionalProperties"": false, ""required"": [""A""] }",
              "declares already",
              TestName = "AnAlternative_FieldNamedLikeALocal")]
    public void AnAlternativeThisGeneratorHasNoRecordForIsRefused(string alternative,
                                                                  string said)
      => Assert.That(async () => await Render(ChoiceWith(alternative)
                                                .Replace(@"""Place"": {",
                                                         @"""Held"": { ""description"": ""A group."", ""type"": ""object"", ""properties"": { ""X"": { ""description"": ""X."", ""type"": ""string"" } }, ""additionalProperties"": false },
    ""Place"": {"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains(said));

    /// <summary>The document is a group, so a root that is a choice is refused.</summary>
    [Test]
    public void ARootThatIsAChoiceIsRefused()
      => Assert.That(async () => await Render(@"{
  ""title"": ""Options"",
  ""description"": ""Either."",
  ""oneOf"": [
    { ""description"": ""A."", ""type"": ""object"", ""properties"": { ""A"": { ""const"": true } }, ""additionalProperties"": false, ""required"": [""A""] }
  ]
}")
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("root is a choice"));

    /// <summary>A type named after a converter this renders would be two types of one name.</summary>
    [Test]
    public void ATypeNamedAfterAConverterIsRefused()
      => Assert.That(async () => await Render(Choice.Replace("Place",
                                                             "VerificationJsonConverter"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("beside the vocabulary"));

    /// <summary>A class's options are optional, so one the schema requires is refused.</summary>
    /// <remarks>Nothing generated checks that an option of a class was set; a field of an alternative is a parameter.</remarks>
    [Test]
    public void ARequiredOptionOfAGroupIsRefused()
      => Assert.That(async () => await Render(@"{
  ""title"": ""Options"",
  ""description"": ""What a caller may set."",
  ""type"": ""object"",
  ""properties"": { ""X"": { ""description"": ""X."", ""type"": ""string"" } },
  ""required"": [""X""],
  ""additionalProperties"": false
}")
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("may leave unset"));

    /// <summary>A field the schema marks `writeOnly` is a secret: printed elided, and never quoted.</summary>
    /// <remarks>
    ///   A record prints every property in its ToString, which reaches logs and debuggers; and an
    ///   ArgumentOutOfRangeException carries the value it is given in its message.
    /// </remarks>
    [Test]
    public async Task ASecretFieldIsPrintedElidedAndNeverQuoted()
    {
      var rendered = await Render(ChoiceWith(@"{ ""description"": ""A bundle."", ""type"": ""object"", ""properties"": { ""Bundle"": { ""type"": ""object"", ""properties"": { ""Path"": { ""description"": ""Its path."", ""type"": ""string"" }, ""Password"": { ""description"": ""Its password."", ""type"": ""string"", ""minLength"": 1, ""writeOnly"": true } }, ""required"": [""Path""], ""additionalProperties"": false } }, ""additionalProperties"": false, ""required"": [""Bundle""] }"))
                       .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(rendered,
                                    Does.Contain("protected override bool PrintMembers(global::System.Text.StringBuilder builder)"));
                        Assert.That(rendered,
                                    Does.Contain(@"builder.Append(Password is null ? ""null"" : ""***"");"));
                        Assert.That(rendered,
                                    Does.Contain("builder.Append((object?)Path);"));
                        Assert.That(rendered,
                                    Does.Match(@"nameof\(Password\),\n\s+""Password has to be at least 1 character long\.""\);"),
                                    "the secret is not passed as the actual value");
                        Assert.That(rendered.Split("PrintMembers(")
                                            .Length - 1,
                                    Is.EqualTo(1),
                                    "only the alternative holding a secret prints its own way");
                      });
    }

    /// <summary>An alternative whose fields are none is made with none.</summary>
    [Test]
    public async Task AnAlternativeWithNoFieldsIsMadeWithNone()
    {
      var rendered = await Render(ChoiceWith(@"{ ""description"": ""Empty."", ""type"": ""object"", ""properties"": { ""Empty"": { ""type"": ""object"", ""properties"": {}, ""additionalProperties"": false } }, ""additionalProperties"": false, ""required"": [""Empty""] }"))
                       .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(rendered,
                                    Does.Contain("public sealed record Empty : Verification"));
                        Assert.That(rendered,
                                    Does.Contain(@"writer.WriteStartObject(""Empty"");
        writer.WriteEndObject();".Replace("\r\n",
                                          "\n")));
                      });
    }

    /// <summary>What the engine reads as one key, or a C# scope would confuse, is refused.</summary>
    [TestCase(@"""Unchecked""",
              @"""PINNED""",
              "differ at most in case",
              TestName = "AName_AlternativesTwinInCase")]
    [TestCase(@"""Pinned""",
              @"""Place""",
              "which it would hide",
              TestName = "AName_AnAlternativeHidingAType")]
    [TestCase(@"""const"": ""There""",
              @"""const"": ""HERE""",
              "differ at most in case",
              TestName = "AName_EnumerationTwinsInCase")]
    public void ANameTheEngineOrTheScopeCannotTellApartIsRefused(string from,
                                                                 string to,
                                                                 string said)
      => Assert.That(async () => await Render(Choice.Replace(from,
                                                             to))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains(said));

    /// <summary>Two options of a class that differ only in case are one key to the engine.</summary>
    [Test]
    public void TwoOptionsThatDifferOnlyInCaseAreRefused()
      => Assert.That(async () => await Render(Wrap($@"""Name"": {{ {Documented}""type"": ""string"" }}, ""NAME"": {{ {Documented}""type"": ""string"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("differ at most in case"));

    /// <summary>A constant outside an alternative is a constraint nothing here would check.</summary>
    [Test]
    public void AConstantOutsideAnAlternativeIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Fixed"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"", ""const"": 5 }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("states a constant"));

    /// <summary>A `oneOf` of nothing admits no value, and is refused.</summary>
    [Test]
    public void AChoiceOfNothingIsRefused()
      => Assert.That(async () => await Render(Choice.Replace("\r\n",
                                                             "\n")
                                                    .Replace(@"""oneOf"": [
        { ""description"": ""Here."", ""type"": ""string"", ""const"": ""Here"" },
        { ""description"": ""There."", ""type"": ""string"", ""const"": ""There"" }
      ]".Replace("\r\n",
                 "\n"),
                                                             @"""oneOf"": []"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("states no alternative"));

    /// <summary>The options keep the order the schema states, which is the order the type declares.</summary>
    /// <remarks>Corvus sorts them by name, which would put a password before its username.</remarks>
    [Test]
    public async Task TheOptionsKeepTheOrderTheSchemaStates()
    {
      var rendered = await Render(Wrap($@"""Zed"": {{ {Documented}""type"": ""string"" }}, ""Alpha"": {{ {Documented}""type"": ""string"" }}"))
                       .ConfigureAwait(false);

      Assert.That(rendered.IndexOf("public string? Zed",
                                   StringComparison.Ordinal),
                  Is.LessThan(rendered.IndexOf("public string? Alpha",
                                               StringComparison.Ordinal)));
    }

    /// <summary>A choice the reused schema renders is not rendered again.</summary>
    [Test]
    public async Task AChoiceTheReusedSchemaHoldsIsNotRenderedAgain()
    {
      var reused = await OptionVocabulary.ReadAsync(Choice)
                                         .ConfigureAwait(false);
      var types = await OptionVocabulary.ReadAsync(Choice.Replace(@"""title"": ""Options""",
                                                                  @"""title"": ""Other"""))
                                        .ConfigureAwait(false);

      Assert.That(OptionVocabulary.Without(types,
                                           reused)
                                  .Select(type => type.Name),
                  Is.EqualTo(new[]
                             {
                               "Other",
                             }));
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

    /// <summary>A default stated as a keyword stops the generator.</summary>
    /// <remarks>
    ///   A default is stated in its option's description and applied by the engine. As a keyword
    ///   it is one a generator could act on, and the class would then hold a second copy.
    /// </remarks>
    [Test]
    public void ADefaultStatedAsAKeywordIsRefused()
      => Assert.That(OptionVocabulary.Unhandled(Wrap(@"""Name"": { ""type"": ""string"", ""default"": ""a"" }")),
                     Is.EqualTo(new[]
                                {
                                  "default",
                                }));

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

    /// <summary>A group is copied and not shared, or a caller still holds what it handed over.</summary>
    /// <remarks>
    ///   A holder of these options reads them more than once - once to size what it allocates,
    ///   once to serialize - and a settable property read twice is two values if anything sets it
    ///   in between. The copy is what makes it one, and a shared group would be the same hole one
    ///   level down.
    /// </remarks>
    [Test]
    public async Task AGroupIsCopiedRatherThanShared()
    {
      var rendered = await Render(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                       @",
  ""$defs"": {
    ""TransportOptions"": {
      ""description"": ""What the transport does."",
      ""type"": ""object"",
      ""additionalProperties"": false,
      ""properties"": { ""Timeout"": { ""description"": ""How long."", ""type"": ""number"", ""format"": ""double"" } }
    }
  }"))
                       .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(rendered,
                                    Does.Contain("public Options(Options other)"));
                        Assert.That(rendered,
                                    Does.Contain(": new TransportOptions(other.Transport);"),
                                    "the group is copied");
                        Assert.That(rendered,
                                    Does.Contain("throw new ArgumentNullException(nameof(other));"));
                      });
    }

    /// <summary>A vocabulary the engine takes as fields has no document to encode.</summary>
    [Test]
    public async Task AVocabularyTakenAsFieldsRendersNoEncoding()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Credits"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"" }}"))
                                         .ConfigureAwait(false);

      var fields = CSharpSource.Render(groups,
                                       "Test",
                                       "test.schema.json",
                                       document: false);
      var document = CSharpSource.Render(groups,
                                         "Test",
                                         "test.schema.json");

      Assert.Multiple(() =>
                      {
                        Assert.That(fields,
                                    Does.Not.Contain("Encode()"));
                        Assert.That(fields,
                                    Does.Not.Contain("JsonSerializerContext"));
                        Assert.That(fields,
                                    Does.Contain("public void Validate()"),
                                    "the bounds are checked all the same");
                        Assert.That(document,
                                    Does.Contain("internal byte[] Encode()"));
                      });
    }

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

    /// <summary>And a cycle that goes through a type's properties, which is what a recursive one
    /// has.</summary>
    /// <remarks>
    ///   An edge is recorded at the node holding the `$ref` - `#/$defs/A/properties/B` - so a walk
    ///   that looked its target up as a key would find one only where the target is itself a bare
    ///   `$ref`. That shape is the test above; this one is the shape a schema actually gets, and
    ///   the guard has to reach it through what the target contains.
    /// </remarks>
    [Test]
    public void ACycleThroughPropertiesIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""X"": {{ {Documented}""$ref"": ""#/$defs/A"" }}",
                                                   @",
  ""$defs"": {
    ""A"": {
      ""description"": ""A."", ""type"": ""object"", ""additionalProperties"": false,
      ""properties"": { ""B"": { ""description"": ""To B."", ""$ref"": ""#/$defs/B"" } }
    },
    ""B"": {
      ""description"": ""B."", ""type"": ""object"", ""additionalProperties"": false,
      ""properties"": { ""A"": { ""description"": ""Back."", ""$ref"": ""#/$defs/A"" } }
    }
  }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("cycle"));

    /// <summary>A bound stated on a node the chain passes through is checked like the ends'.</summary>
    /// <remarks>
    ///   Draft 2020-12 applies the keywords of every node a `$ref` chain reaches, so a value has
    ///   to satisfy all of them. This generator reads the property's node and the chain's far end
    ///   and no hop between - which holds only because the far end it is handed is Corvus's
    ///   reduction of the whole chain, carrying every hop's keywords already. That is the fact
    ///   this test pins: `maximum` is stated on the middle node alone and `minimum` on the last,
    ///   and both have to survive.
    /// </remarks>
    [Test]
    public async Task ABoundOnAHopOfTheChainIsCheckedToo()
    {
      var rendered = await Render(Wrap($@"""Credits"": {{ {Documented}""$ref"": ""#/$defs/Middle"" }}",
                                       @",
  ""$defs"": {
    ""Middle"": { ""description"": ""Through here."", ""$ref"": ""#/$defs/End"", ""maximum"": 900 },
    ""End"": { ""description"": ""The end."", ""type"": ""integer"", ""format"": ""int32"", ""minimum"": 1 }
  }"))
                       .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(rendered,
                                    Does.Contain("(credits < 1 || credits > 900)"),
                                    "the end's minimum and the hop's maximum, both");
                      });
    }

    /// <summary>The document the engine reads is rendered here too, root only.</summary>
    /// <remarks>The document is one object, so a group of it gets neither.</remarks>
    [Test]
    public async Task TheRootRendersTheDocumentAndAGroupDoesNot()
    {
      var rendered = await Render(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                       @",
  ""$defs"": {
    ""TransportOptions"": {
      ""description"": ""What the transport does."",
      ""type"": ""object"",
      ""additionalProperties"": false,
      ""properties"": { ""Timeout"": { ""description"": ""How long."", ""type"": ""number"", ""format"": ""double"" } }
    }
  }"))
                       .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(rendered,
                                    Does.Contain("[JsonSerializable(typeof(Options))]"));
                        Assert.That(rendered,
                                    Does.Contain("internal partial class OptionsJsonContext : JsonSerializerContext"));
                        Assert.That(rendered,
                                    Does.Contain("OptionsJsonContext.Default.Options"));
                        Assert.That(rendered.Split("internal byte[] Encode()")
                                            .Length - 1,
                                    Is.EqualTo(1),
                                    "the root encodes and the group does not");
                      });
    }

    /// <summary>A group named after the serializer context is refused.</summary>
    /// <remarks>The context is declared beside the classes, so the two would be one name.</remarks>
    [Test]
    public void AGroupNamedAfterTheSerializerContextIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Held"": {{ {Documented}""$ref"": ""#/$defs/OptionsJsonContext"" }}",
                                                   @",
  ""$defs"": {
    ""OptionsJsonContext"": {
      ""description"": ""A group whose name is taken."",
      ""type"": ""object"",
      ""additionalProperties"": false,
      ""properties"": { ""A"": { ""description"": ""A."", ""type"": ""string"" } }
    }
  }"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("serializer context"));

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

    /// <summary>An option named after the group that declares it is refused.</summary>
    /// <remarks>C# admits no member named after its own class, and says so about a generated file.</remarks>
    [Test]
    public void AnOptionNamedAfterItsOwnGroupIsRefused()
      => Assert.That(async () => await Render(Wrap($@"""Options"": {{ {Documented}""type"": ""string"" }}"))
                       .ConfigureAwait(false),
                     Throws.TypeOf<NotSupportedException>()
                           .With.Message.Contains("named after its own class"));

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
    [TestCase("Encode", TestName = "AName_TheRootsOwnMember")]
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

    /// <summary>`oneOf`, `const` and `required` are read, and what a `oneOf` holds is still walked.</summary>
    /// <remarks>
    ///   The reading refuses a constant outside an alternative and a required option of a class,
    ///   so the walk need not; a keyword inside an alternative is still one the walk reports.
    /// </remarks>
    [Test]
    public void AChoiceIsReadAndWhatItHoldsIsWalked()
      => Assert.That(OptionVocabulary.Unhandled(@"{
  ""oneOf"": [
    { ""type"": ""object"", ""properties"": { ""A"": { ""const"": true } }, ""required"": [""A""], ""additionalProperties"": false },
    { ""type"": ""object"", ""properties"": { ""B"": { ""type"": ""string"", ""pattern"": ""^b"" } }, ""required"": [""B""], ""additionalProperties"": false }
  ]
}"),
                     Is.EqualTo(new[]
                                {
                                  "pattern",
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
                           .With.Message.Contains("bounds no type"));

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
                  + "  dotnet run --project packages/csharp/ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator -- "
                  + "--schema packages/rust/armonik-transport/options.schema.json "
                  + "--output packages/csharp/ArmoniK.Api.Client.RustGrpcChannel/ChannelOptions.g.cs");
    }

    private const string TransportDefs = @",
  ""$defs"": {
    ""TransportOptions"": {
      ""type"": ""object"",
      ""description"": ""What the transport does."",
      ""properties"": { ""Timeout"": { ""description"": ""How long."", ""type"": ""number"", ""format"": ""double"" } },
      ""additionalProperties"": false
    }
  }";

    /// <summary>A group another schema renders is taken as declared there, and only the rest is
    /// rendered.</summary>
    [Test]
    public async Task AGroupAnotherSchemaRendersIsNotRenderedAgain()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs))
                                         .ConfigureAwait(false);
      var reused = await OptionVocabulary.ReadAsync(Wrap($@"""Elsewhere"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs)
                                                      .Replace(@"""title"": ""Options""",
                                                               @"""title"": ""Other"""))
                                         .ConfigureAwait(false);

      var kept = OptionVocabulary.Without(groups,
                                          reused);

      Assert.That(kept.Select(group => group.Name),
                  Is.EqualTo(new[]
                             {
                               "Options",
                             }));
      Assert.That(CSharpSource.Render(kept,
                                      "Test",
                                      "test.schema.json"),
                  Does.Contain("public TransportOptions? Transport { get; set; }")
                      .And.Not.Contain("public sealed class TransportOptions"));
    }

    /// <summary>Two groups of one name that differ would be one class that is not what one of the
    /// schemas states.</summary>
    [Test]
    public async Task AReusedGroupThatDiffersIsRefused()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs))
                                         .ConfigureAwait(false);
      var reused = await OptionVocabulary.ReadAsync(Wrap($@"""Elsewhere"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs.Replace("How long.",
                                                                               "How long, in seconds."))
                                                      .Replace(@"""title"": ""Options""",
                                                               @"""title"": ""Other"""))
                                         .ConfigureAwait(false);

      Assert.That(() => OptionVocabulary.Without(groups,
                                                 reused),
                  Throws.InstanceOf<NotSupportedException>()
                        .With.Message.Contains("TransportOptions"));
    }

    /// <summary>A reused group whose option is of another type is a drift too.</summary>
    [Test]
    public async Task AReusedGroupWhoseOptionDiffersIsRefused()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs))
                                         .ConfigureAwait(false);
      var reused = await OptionVocabulary.ReadAsync(Wrap($@"""Elsewhere"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs.Replace(@"""type"": ""number"", ""format"": ""double""",
                                                                               @"""type"": ""integer"", ""format"": ""int32"""))
                                                      .Replace(@"""title"": ""Options""",
                                                               @"""title"": ""Other"""))
                                         .ConfigureAwait(false);

      Assert.That(() => OptionVocabulary.Without(groups,
                                                 reused),
                  Throws.InstanceOf<NotSupportedException>()
                        .With.Message.Contains("TransportOptions"));
    }

    /// <summary>A reused schema holding none of the groups reuses nothing, and the classes it was
    /// meant to stand for would be declared twice.</summary>
    [Test]
    public async Task AReusedSchemaHoldingNoneOfTheGroupsIsRefused()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs))
                                         .ConfigureAwait(false);
      var unrelated = await OptionVocabulary.ReadAsync(Wrap($@"""Credits"": {{ {Documented}""type"": ""integer"", ""format"": ""int32"" }}")
                                                         .Replace(@"""title"": ""Options""",
                                                                  @"""title"": ""Other"""))
                                            .ConfigureAwait(false);

      Assert.That(() => OptionVocabulary.Without(groups,
                                                 unrelated),
                  Throws.InstanceOf<NotSupportedException>()
                        .With.Message.Contains("nothing is reused"));
    }

    /// <summary>The root reused would leave nothing to render the file for.</summary>
    [Test]
    public async Task AReusedRootIsRefused()
    {
      var groups = await OptionVocabulary.ReadAsync(Wrap($@"""Transport"": {{ {Documented}""$ref"": ""#/$defs/TransportOptions"" }}",
                                                         TransportDefs))
                                         .ConfigureAwait(false);

      Assert.That(() => OptionVocabulary.Without(groups,
                                                 groups),
                  Throws.InstanceOf<NotSupportedException>()
                        .With.Message.Contains("root"));
    }

    private static string Metadata(string key)
      => Assembly.GetExecutingAssembly()
                 .GetCustomAttributes<AssemblyMetadataAttribute>()
                 .Single(attribute => attribute.Key == key)
                 .Value
         ?? throw new InvalidOperationException($"`{key}` names no path.");
  }
}
