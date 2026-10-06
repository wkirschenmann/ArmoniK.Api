# Configuration loading: one loader, every host

Status: decided on 2026-10-06 and its shape confirmed on 2026-10-07, to be built by T6.14, but
for the two points under Open.

## Why

A host reaches the engine through the C ABI, and today each host builds the engine's options
itself: .NET binds an `IConfiguration` section through the generated `ChannelOptions.Bind`, then
hands the engine a JSON document; the Rust `armonik` crate reads its own `GrpcClient__*`
environment variables through `ClientConfig::from_env` (`armonik-transport/src/config.rs`), a
vocabulary T3.5 found to share no name with the engine's. A third host - Python, Java, C++ -
would write a third loader, and three loaders are three readings of what an environment variable
or a file means.

The aim: the configuration crosses the ABI as a C structure that says where it comes from - which
may be "this JSON document" - and the engine's Rust loads it, from the environment or from a file as
well as from JSON, so every host language gets the same result from the same sources.

## What exists

- `ak_runtime_config` (`armonik_transport_ffi.h`) is a versioned C structure - `struct_size`,
  `version`, `flags`, `reserved` - with the memory ceilings and `channel_defaults_json`, a channel
  document every channel of the runtime takes where its own states nothing.
- `ak_channel_create(runtime, endpoint, config_json, ...)` takes the channel's own document.
- The vocabulary is `options.schema.json` (channel) and `runtime.schema.json` (runtime), rendered
  from the Rust types; .NET's `ChannelOptions.g.cs` and `RuntimeOptions.g.cs` are generated from
  them. An unknown key is refused with its path, never ignored; a value is never quoted back.
- Two documents merge by `ChannelOptions::over`: a struct field by field, an alternative (an
  enum: TLS verification, client identity, proxy, receive windows) whole when the two state
  different ones.

## Decided (2026-10-06, the shape confirmed 2026-10-07)

- **Sources**: any number of files, the environment, and what the host states itself - its command
  line, or values written in its code. The host lists them in the order it wants, a later one over
  an earlier one: typically the files from the least to the most important, then the environment,
  then its own values.
- **File formats**: JSON, YAML and TOML.
- **Environment**: `__` as the separator, as .NET's; the prefix is the host's to choose, `GrpcClient`
  by default - the section name `ArmoniK.Api.Client`'s `GrpcClient.SettingSection` gives today.
  Read once, when the runtime is created, and only if the host lists it among the sources.
- **No aliases**: the keys under the prefix are the schema's. The `GrpcClient` names the Rust and
  .NET clients read today migrate, a channel's option under `ChannelDefaults`, the endpoint as
  Open 2 decides; none is mapped.
- **.NET stops binding these options from `IConfiguration`**: the engine's loader reads the files
  and the environment; the host passes only what it states itself. `NativeRuntime.Create(IConfiguration,
  key)` and the `RustGrpcRuntime` section it reads by default go: the runtime's options move under
  the prefix, the channel defaults within them, with no alias.
- **The transport's choice, native or managed, is not a key of this configuration.** The options a
  managed transport needs may be added to the schema.
- **A file that does not parse is reported as .NET reports it**: the file and the line.
- **What the host states itself is JSON in a string**: a document for an object set in code, and
  for a command line its pairs of a key and a text value. Every language has the tools to write
  JSON, and a C mirror of the options would be a second rendering of the vocabulary for no gain a
  host would see.
- **One reader of the environment, by the binding's API shape.** A consumer of `ArmoniK.Api`
  builds its `IConfiguration` as it likes, and the binding cannot constrain that, so it takes none.
  It exposes a set of loads instead, applied in the order the host calls them, a later one over an
  earlier one: `LoadConfigFromFiles(files)`, `LoadConfigFromEnvironment()`,
  `LoadConfigFromCommandLine(args)` and `LoadConfigFromObject(obj)`, where `obj` is an instance of
  the generated options types, set in code and never bound. The host feeds what it wants, and
  nothing reaches the engine through an `IConfiguration` by mistake. Each load is a source the
  engine reads; the files and the environment are read by the engine alone.
- **The command line is parsed by each binding, in its language's idiom**: .NET's syntaxes through
  an `IConfiguration` with the command-line provider alone, used inside the binding and never
  exposed. What it parses reaches the engine as pairs of a key's path and a text value, read as
  the environment's are, since a command line, like the environment, has only text.
- **An unknown key is refused**, a variable under the prefix as much as a key in a file, where
  .NET's own binder ignores one unless `ErrorOnUnknownConfiguration` is set: a misspelled key
  would otherwise give the defaults with nothing to say so. On every host once Open 1 is settled.

## Requirements

1. **One loader**, in Rust, behind the ABI and in the `armonik` crate alike: the FFI entry points
   call the same function a Rust caller does.
2. **Sources**: files, the process environment, and what the host states itself. A later source
   overrides an earlier one, option by option, by the merge rule above.
3. **The C structure drives**: it lists the sources, it is versioned as `ak_runtime_config` is,
   and a field it does not know is refused rather than ignored. Additive: the current entry points
   keep working, a JSON document being the one-source case.
4. **The same vocabulary everywhere**: the keys are the schema's; the environment and file forms
   are mechanical renderings of them, not a second vocabulary.
5. **Strict**: an unknown key, from any source, is refused with the source and the path, on every
   host once Open 1 is settled; a secret (a password, a key) never appears in a message.
6. **Conformance**: one set of fixtures - sources in, resulting document or refusal out - run
   against the Rust loader directly and against each binding through the ABI.

## Shape

### What a configuration holds

One document per runtime, under one prefix: the runtime's options as `runtime.schema.json` has
them, the channel options every channel takes by default among them, under `ChannelDefaults`.

```json
{
  "GrpcClient": {
    "MemoryCeiling": 2147483648,
    "ChannelDefaults": {
      "Http2": { "SimultaneousCallsPerConnection": 4 },
      "Transport": { "Tls": { "Client": { "P12": { "Path": "client.p12" } } } }
    }
  }
}
```

A channel loads no source. The runtime loads them when it is created, and only then: a channel
created later states its own options as a document, merged over these defaults as today. A host
with no runtime, the `armonik` crate's client, loads the same sources when it is created and takes
their `ChannelDefaults` as its channel's options, so that one file or one environment configures
every host alike; how it reads the rest of the document is Open 1.

### Sources

- **A file**, JSON, YAML or TOML by its extension (`.json`, `.yaml` or `.yml`, `.toml`). The
  document is the file's section named by the prefix, so the host's own `appsettings.json` can
  carry it beside sections the host reads itself; the other sections are not the engine's, and are
  left alone. A file with no such section contributes nothing. With no prefix, the document is the
  whole file, so a file that also holds sections of its own host is refused. A missing file is
  refused, unless the host marks it optional, as .NET's `AddJsonFile(path, optional: true)` does.
- **The environment**: the variables whose name starts with the prefix and `__`, read once, when
  the runtime is created. The rest of a name is the key's path, its parts joined by `__`, compared
  without case: `GrpcClient__ChannelDefaults__Http2__SimultaneousCallsPerConnection=4`. A value is
  text, parsed by the schema's type for that key. The environment needs a prefix: with none, every
  variable of the process would be a key, and none could be refused, so an environment source with
  no prefix is refused. The schema holds no list today; one that comes takes an element by its
  index, `__0`, as .NET's providers render one.
- **Pairs**: a JSON object whose names are keys' paths, their parts joined by `__`, under no
  prefix, and whose values are text, read as the environment's values are, by the schema's type
  for that key: `{"ChannelDefaults__Http2__SimultaneousCallsPerConnection": "4"}`. It is what a
  binding sends for a command line it has parsed.
- **A document**: JSON in the schema's vocabulary, with no prefix around it. It is what a binding
  sends for an object set in code.

A later source overrides an earlier one option by option: a structure field by field, an
alternative whole when two sources state different ones, as `ChannelOptions::over` merges two
documents today.

### Refusals

The first refusal ends the load and names its source - the file's path, `the environment`,
`pairs` or `a document` - then, when a key is at fault, the key's path within the prefix. Refused:

- a file that does not parse, with its line, as .NET reports one, and a file whose extension is
  none of the four;
- a missing file not marked optional;
- an environment source with no prefix;
- a document that is not JSON, a prefix's section that is not an object, and pairs that are not
  a JSON object of text values;
- a key under the prefix that the schema does not declare, in every source, and a value that is
  not of its key's type, an environment or pair value being parsed as that type.

A value is never quoted, a password being one. Through the C structure, what is malformed in it - a
kind it does not name, a nonzero `reserved`, a flag it does not know, a value on an environment
source, a prefix beside `AK_CONFIG_NO_PREFIX` - is `AK_STATUS_INVALID_ARG`, before any source is
read.

### Rust

The loader lives in `armonik-transport` and is generic over the document it reads: each source is
read into that document's type, which refuses a key it does not declare, and the documents merge
by the type's own `over`. `armonik-transport-ffi` reads its `RuntimeOptions`, the channel
defaults within it, the runtime's keys staying the FFI's own. As written, the `armonik` crate reads
the `ChannelDefaults` section of the same document as `ChannelOptions`, leaving the runtime's keys
beside it alone, as a file's other sections are; Open 1 would have it read the whole document.

```rust
/// A document a configuration can be read into: strict on its keys, merged one over another.
pub trait Document: serde::de::DeserializeOwned {
    fn over(self, earlier: Self) -> Self;
}

/// Where a configuration comes from, in the order the sources are added.
pub struct Configuration { /* prefix, sources */ }

impl Configuration {
    /// Under `GrpcClient`, with no source.
    pub fn new() -> Self;
    /// Under `prefix`, or under none when it is empty.
    pub fn with_prefix(prefix: &str) -> Self;
    pub fn file(self, path: impl Into<PathBuf>) -> Self;
    pub fn optional_file(self, path: impl Into<PathBuf>) -> Self;
    pub fn environment(self) -> Self;
    pub fn pairs(self, pairs: impl IntoIterator<Item = (String, String)>) -> Self;
    pub fn document(self, json: impl Into<String>) -> Self;

    /// Reads the sources, in order, into one document.
    pub fn load<D: Document>(&self) -> Result<D, ConfigRefusal>;
    /// The same, from the section `section` of each source's document.
    pub fn load_section<D: Document>(&self, section: &str) -> Result<D, ConfigRefusal>;
}
```

The FFI calls `load::<RuntimeOptions>()`, the `armonik` crate, as written,
`load_section::<ChannelOptions>("ChannelDefaults")` (Open 1).

### C

```c
typedef enum {
    AK_SOURCE_FILE = 1,           /* value: the path, UTF-8 */
    AK_SOURCE_OPTIONAL_FILE = 2,  /* value: the path; a missing file contributes nothing */
    AK_SOURCE_ENVIRONMENT = 3,    /* value: empty; another is AK_STATUS_INVALID_ARG */
    AK_SOURCE_DOCUMENT = 4,       /* value: a JSON document, no prefix around it */
    AK_SOURCE_PAIRS = 5,          /* value: a JSON object, key__path to text, no prefix */
} ak_source_kind;

typedef struct {
    uint32_t kind;                /* ak_source_kind; another is AK_STATUS_INVALID_ARG */
    uint32_t reserved;            /* zero; another is AK_STATUS_INVALID_ARG */
    ak_bytes_in value;
} ak_config_source;

enum {
    AK_CONFIG_NO_PREFIX = 1,      /* no prefix: a file's document is the whole file */
};

typedef struct {
    uint32_t struct_size;
    uint32_t version;             /* zero */
    uint32_t flags;               /* AK_CONFIG_*; another bit is AK_STATUS_INVALID_ARG */
    uint32_t source_count;
    const ak_config_source *sources;  /* in order, a later one over an earlier one */
    ak_bytes_in prefix;           /* empty: GrpcClient; with AK_CONFIG_NO_PREFIX it must be
                                     empty, else AK_STATUS_INVALID_ARG */
} ak_config;

ak_status ak_runtime_create_from(const ak_config *config,
                                 ak_callback callback,
                                 void *runtime_ctx,
                                 ak_handle *out,
                                 ak_error *out_error);
```

`ak_runtime_create_from` sits beside `ak_runtime_create`, whose `ak_runtime_config` is the case of
one document, and takes the same callback and context, the channel through which every event of the
runtime reaches the host; `ak_channel_create` is unchanged. A refusal comes back through `ak_error`,
as an option's does today, its detail naming the source and the path.

### .NET

```csharp
var configuration = new NativeConfiguration()                // or NativeConfiguration(prefix)
                      .LoadConfigFromFiles("appsettings.json", "appsettings.Production.yaml")
                      .LoadConfigFromEnvironment()
                      .LoadConfigFromCommandLine(args)
                      .LoadConfigFromObject(new RuntimeOptions { MemoryCeiling = 1L << 31 });
await using var runtime = NativeRuntime.Create(configuration);
```

The prefix is `GrpcClient` unless the constructor is given one; `""` is none, which the binding
passes as `AK_CONFIG_NO_PREFIX`. `LoadConfigFromFiles` and `LoadConfigFromEnvironment` add a source
the engine reads, and what the engine refuses in one surfaces at `NativeRuntime.Create`.
`LoadConfigFromCommandLine` parses the arguments with an `IConfiguration` holding the command-line
provider alone and adds the section under the prefix - the whole tree with none - as pairs, its
keys as they are and its values as text, so that the engine refuses an unknown key and types a
value as it does the environment's; `LoadConfigFromObject` serializes its object as a document,
writing only the options set, so that a default does not override an earlier source. No
`IConfiguration` is taken or returned.

## Open

1. **Strict on every host.** As written, the `armonik` crate's client reads only
   `ChannelDefaults`, so the keys beside it under the prefix are neither read nor refused: a
   misspelled `ChannelDefaults`, or a misspelled runtime key, gives the defaults with nothing to say
   so, where the FFI refuses the same file. An option misspelled inside `ChannelDefaults` is
   refused on both. Two ways to close it:
   - `RuntimeOptions` moves from `armonik-transport-ffi` to `armonik-transport`, with its schema
     and its generated .NET class, and both hosts read the whole document as it, strictly. The
     `armonik` client then accepts the memory ceilings, which bound the FFI's lent buffers, and does
     nothing with them: values read and ignored.
   - The `armonik` client keeps reading `ChannelDefaults` alone, and refuses a key beside it that is
     not one of the runtime's, which it has to know by name.
2. **The endpoint.** `GrpcClient__Endpoint` is read by the Rust and .NET clients today, and
   neither `RuntimeOptions` nor `ChannelOptions` holds it: `ak_channel_create` takes it as an
   argument, the one value a channel cannot be created without. As written, a section that keeps
   its `Endpoint` is refused by the FFI as an unknown key, and ignored by the `armonik` client as
   Open 1 describes. Two ways:
   - `Endpoint` becomes a key of the runtime's document, the endpoint the `armonik` client reaches
     and the one a channel reaches when `ak_channel_create` is given none - which changes that
     entry point, its endpoint becoming optional. The `armonik` client can read it only if it reads
     more than `ChannelDefaults`, so this depends on how Open 1 is settled.
   - It is no key of this configuration: each host reads it, and spells it, as it likes, outside
     the prefix's section or under a name of its own. The endpoint is then not configured alike on
     every host, which this document sets out to give every option.
