# Configuration loading: one loader, every host

Status: decided in principle on 2026-10-06; the proposed shape is to be confirmed on the points
listed at the end.

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

## Decided (2026-10-06)

- **Sources**: any number of files, the environment, and what the host states itself - its command
  line, or values written in its code. The host lists them in the order it wants, a later one over
  an earlier one: typically the files from the least to the most important, then the environment,
  then its own values.
- **File formats**: JSON, YAML and TOML.
- **Environment**: `__` as the separator, as .NET's; the prefix is the host's to choose, `GrpcClient`
  by default - the section name `ArmoniK.Api.Client`'s `GrpcClient.SettingSection` gives today.
  Read once, when the runtime is created, and only if the host lists it among the sources.
- **No aliases**: the keys under the prefix are the schema's. The `GrpcClient` names the Rust and
  .NET clients read today migrate; none is mapped.
- **.NET stops binding these options from `IConfiguration`**: the engine's loader reads the files
  and the environment; the host passes only what it states itself. `NativeRuntime.Create(IConfiguration,
  key)` and the `RustGrpcRuntime` section it reads by default go: the runtime's options move under
  the same prefix as the channels', with no alias.
- **The transport's choice, native or managed, is not a key of this configuration.** The options a
  managed transport needs may be added to the schema.
- **A file that does not parse is reported as .NET reports it**: the file and the line.
- **What the host states itself is a JSON document in a string**, the last source: every language
  has the tools to write JSON, and a C mirror of the options would be a second rendering of the
  vocabulary for no gain a host would see.
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
  exposed. What it parses reaches the engine as a JSON document, as an object does.
- **An unknown key is refused**, a variable under the prefix as much as a key in a file, where
  .NET's own binder ignores one unless `ErrorOnUnknownConfiguration` is set: a misspelled key
  would otherwise give the defaults with nothing to say so.

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
5. **Strict**: an unknown key, from any source, is refused with the source and the path; a secret
   (a password, a key) never appears in a message.
6. **Conformance**: one set of fixtures - sources in, resulting document or refusal out - run
   against the Rust loader directly and against each binding through the ABI.

## Proposed shape

### What a configuration holds

One document per runtime, under one prefix: the runtime's own options and the channel options
every channel takes by default, side by side.

```json
{
  "GrpcClient": {
    "MemoryCeiling": 2147483648,
    "Http2": { "SimultaneousCallsPerConnection": 4 },
    "Transport": { "Tls": { "Client": { "P12": { "Path": "client.p12" } } } }
  }
}
```

Its schema is `runtime.schema.json` with `ChannelDefaults` lifted to the top: the runtime's keys
(`MemoryCeiling`, `MemoryHardCeiling`) and the channel's (`Transport`, `Http2`, `Grpc`) do not
collide, and a user writes `GrpcClient__Http2__...` rather than
`GrpcClient__ChannelDefaults__Http2__...`. The sources are loaded when the runtime is created, and
only then: a channel created later states its own options as a document, merged over these
defaults as today, and reads neither files nor the environment.

### Sources

- **A file**, JSON, YAML or TOML by its extension (`.json`, `.yaml` or `.yml`, `.toml`). The
  document is the file's section named by the prefix, so the host's own `appsettings.json` can
  carry it beside sections the host reads itself; the other sections are not the engine's, and are
  left alone. A file with no such section contributes nothing. A missing file is refused, unless
  the host marks it optional, as .NET's `AddJsonFile(path, optional: true)` does.
- **The environment**: the variables whose name starts with the prefix and `__`, read once, when
  the runtime is created. The rest of a name is the key's path, its parts joined by `__`, compared
  without case: `GrpcClient__Http2__SimultaneousCallsPerConnection=4`. A value is text, parsed by
  the schema's type for that key. The schema holds no list today; one that comes takes an element
  by its index, `__0`, as .NET's providers render one.
- **A document**: JSON in the schema's vocabulary, with no prefix around it. It is what a binding
  sends for an object set in code and for a command line it has parsed.

A later source overrides an earlier one option by option: a structure field by field, an
alternative whole when two sources state different ones, as `ChannelOptions::over` merges two
documents today.

### Refusals

The first refusal ends the load and names its source - the file's path, `the environment`, or `a
document` - then the key's path within the prefix. A file that does not parse is refused with its
line, as .NET reports one. A key under the prefix that the schema does not declare is refused, in
every source. A value is never quoted, a password being one.

### Rust

The loader lives in `armonik-transport` and is generic over the document it reads: each source is
read into that document's type, which refuses a key it does not declare, and the documents merge
by the type's own `over`. `armonik-transport` reads `ChannelOptions`, the `armonik` crate's case;
`armonik-transport-ffi` declares the runtime's document - its memory ceilings beside the channel's
options - and reads that, the runtime's keys staying the FFI's own. serde refuses unknown keys and
flattens a structure, but not both at once, so the runtime's document is read in two passes: its
own keys taken out of the source's tree, the rest read strictly as `ChannelOptions`.

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
    pub fn with_prefix(prefix: &str) -> Self;
    pub fn file(self, path: impl Into<PathBuf>) -> Self;
    pub fn optional_file(self, path: impl Into<PathBuf>) -> Self;
    pub fn environment(self) -> Self;
    pub fn document(self, json: impl Into<String>) -> Self;

    /// Reads the sources, in order, into one document.
    pub fn load<D: Document>(&self) -> Result<D, ConfigRefusal>;
}
```

The `armonik` crate calls `load::<ChannelOptions>()`, the FFI `load::<RuntimeDocument>()`.

### C

```c
typedef enum {
    AK_SOURCE_FILE = 1,           /* value: the path, UTF-8 */
    AK_SOURCE_OPTIONAL_FILE = 2,  /* value: the path; a missing file contributes nothing */
    AK_SOURCE_ENVIRONMENT = 3,    /* value: empty */
    AK_SOURCE_DOCUMENT = 4,       /* value: a JSON document, no prefix around it */
} ak_source_kind;

typedef struct {
    uint32_t kind;                /* ak_source_kind; another is AK_STATUS_INVALID_ARG */
    uint32_t reserved;            /* zero */
    ak_bytes_in value;
} ak_config_source;

typedef struct {
    uint32_t struct_size;
    uint32_t version;             /* zero */
    uint32_t flags;               /* zero */
    uint32_t source_count;
    const ak_config_source *sources;  /* in order, a later one over an earlier one */
    ak_bytes_in prefix;           /* empty: GrpcClient */
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
var configuration = new NativeConfiguration()                // prefix GrpcClient
                      .LoadConfigFromFiles("appsettings.json", "appsettings.Production.yaml")
                      .LoadConfigFromEnvironment()
                      .LoadConfigFromCommandLine(args)
                      .LoadConfigFromObject(new RuntimeOptions { MemoryCeiling = 1L << 31 });
await using var runtime = NativeRuntime.Create(configuration);
```

`LoadConfigFromFiles` and `LoadConfigFromEnvironment` add a source the engine reads.
`LoadConfigFromCommandLine` parses the arguments with an `IConfiguration` holding the command-line
provider alone, binds the section under the prefix into the generated types, and adds the result as
a document; `LoadConfigFromObject` serializes its object as one. No `IConfiguration` is taken or
returned.

## To confirm

1. **One document per runtime**, the channel defaults lifted beside the runtime's options, so that
   a key reads `GrpcClient__Http2__...`.
2. **Only the runtime loads sources**: a channel states its own options as a document over the
   defaults, and reads no file and no variable.
3. **A file's document is its section named by the prefix**, the host's other sections left alone.
4. **A missing file is refused unless marked optional.**
