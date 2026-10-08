# Configuration loading: one loader, every host

Status: decided on 2026-10-06, its shape confirmed and its open points settled on 2026-10-07, and
built by T6.14 on 2026-10-07.

## Why

A host reaches the engine through the C ABI. Were each host to build the engine's options itself -
.NET binding an `IConfiguration` section, the Rust `armonik` crate reading `GrpcClient__*`
environment variables in a vocabulary T3.5 found to share no name with the engine's - a third
host, Python, Java or C++, would write a third loader, and three loaders are three readings of
what an environment variable or a file means.

The aim: the configuration crosses the ABI as a C structure that says where it comes from - which
may be "this JSON document" - and the engine's Rust loads it, from the environment or from a file as
well as from JSON, so every host language gets the same result from the same sources.

## What exists

- `ak_runtime_config` (`armonik_transport_ffi.h`) is a versioned C structure - `struct_size`,
  `version`, `flags`, `reserved` - with the memory ceilings and `channel_defaults_json`, a channel
  document every channel of the runtime takes where its own states nothing.
- `ak_runtime_create_from` takes an `ak_config`, the sources the engine loads the runtime's document
  from.
- `ak_channel_create(runtime, endpoint, config_json, ...)` takes the channel's own document, and an
  empty endpoint as the runtime's `Endpoint`.
- The vocabulary is `options.schema.json` (channel) and `runtime.schema.json` (runtime), rendered
  from the Rust types; .NET's `ChannelOptions.g.cs` and `RuntimeOptions.g.cs` are generated from
  them. An unknown key is ignored and logged with its path; a value is never quoted back.
- Two documents merge by `ChannelOptions::over`: a struct field by field, an alternative (an
  enum: TLS verification, client identity, proxy, receive windows) whole when the two state
  different ones.

## Decided (2026-10-06, the shape confirmed 2026-10-07)

- **Sources**: any number of files, the environment, and what the host states itself - its command
  line, or values written in its code. The host lists them in the order it wants, a later one over
  an earlier one: typically the files from the least to the most important, then the environment,
  then its own values.
- **File formats**: JSON, YAML and TOML.
- **Environment**: `__` as the separator, as .NET's; the prefix is the host's to choose,
  `ArmoniK__Client__Grpc` by default (decided 2026-10-07): `ArmoniK__Client__` is the family of the
  ArmoniK client's configuration, and `Grpc` its gRPC part, which this engine reads. A prefix is a
  path: its parts, joined by `__` (a `:`, as a .NET section's path is written, is read as `__`), are
  the nested sections of a file, each key compared as written
  (`{"ArmoniK": {"Client": {"Grpc": {...}}}}`), and the start of a variable's name. Read once, when
  the runtime is created, and only if the host lists it among the sources.
- **No aliases**: the keys under the prefix are the schema's. The engine maps none of the names
  `ArmoniK.Api.Client`'s managed transport reads under its `GrpcClient` section: a channel's option
  is under `ChannelDefaults`, the endpoint is `Endpoint`.
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
- **A key left out and a key set to 0 differ** (2026-10-07): a source that leaves an option out
  leaves what an earlier source set, or the default, and a source that wants none says so with 0,
  which `Transport.TcpKeepalive.IdleSeconds`, `Http2.KeepAliveIntervalSeconds`,
  `Http2.IdleTimeoutSeconds` and `Grpc.DefaultDeadlineSeconds` read as none
  (decisions.md, "How an option is turned off", lists the options left as they are).
- **An unknown key is ignored, and logged** (2026-10-07), in every source and on every host, a
  channel's own document included: the load goes on, and the log names the source and the key's
  path, so that a misspelled key does not give the defaults with nothing to say so. The engine logs
  through `tracing`, at info; it reaches a host through the log callback it gives when the runtime
  is created (observability.md), the load's events delivered on the creating thread, selected by
  the `Logging.Filter` the load found.
- **The endpoint is a key of the runtime's document**, `Endpoint` (2026-10-07): the one the
  `armonik` client reaches, and the one a channel reaches when `ak_channel_create` is given none.
- **Both hosts read the runtime's document**, which follows from the endpoint's being one of its
  keys: the `armonik` client, which has no runtime, has to read beyond `ChannelDefaults` to reach
  it. `RuntimeOptions` moves from `armonik-transport-ffi` to `armonik-transport`, with
  `runtime.schema.json` and the `RuntimeOptions.g.cs` generated from it. What it costs: the
  `armonik` client reads the memory ceilings, which bound the FFI's lent buffers, and does nothing
  with them.

## Requirements

1. **One loader**, in Rust, behind the ABI and in the `armonik` crate alike: the FFI entry points
   call the same function a Rust caller does.
2. **Sources**: files, the process environment, and what the host states itself. A later source
   overrides an earlier one, option by option, by the merge rule above.
3. **The C structure drives**: it lists the sources, it is versioned as `ak_runtime_config` is,
   and a field of `ak_config` it does not know is refused rather than ignored. Additive: the
   current entry points keep working, a JSON document being the one-source case.
4. **The same vocabulary everywhere**: the keys are the schema's; the environment and file forms
   are mechanical renderings of them, not a second vocabulary.
5. **Said**: an unknown key, from any source, is logged with the source and the path and
   otherwise ignored; a value that does not fit its key is refused; a secret (a password, a key)
   never appears in a message.
6. **Conformance**: one set of fixtures - sources in, resulting document or refusal out - run
   against the Rust loader directly and against each binding through the ABI; the keys logged as
   unknown are checked on the Rust loader, and through the ABI by the log callback (T10.1).

## Shape

### What a configuration holds

One document per runtime, under one prefix: the runtime's options as `runtime.schema.json` has
them, the channel options every channel takes by default among them, under `ChannelDefaults`.

```json
{
  "ArmoniK": {
    "Client": {
      "Grpc": {
        "Endpoint": "https://armonik.example.com:5001",
        "MemoryCeiling": 2147483648,
        "ChannelDefaults": {
          "Http2": { "SimultaneousCallsPerConnection": 4 },
          "Transport": { "Tls": { "Client": { "P12": { "Path": "client.p12" } } } }
        }
      }
    }
  }
}
```

A channel loads no source. The runtime loads them when it is created, and only then: a channel
created later states its own options as a document, merged over these defaults. A host
with no runtime, the `armonik` crate's client, loads the same document from the same sources when
it is created, and takes its `Endpoint` and its `ChannelDefaults` as its channel's, so that one
file or one environment configures every host alike; a document with no `Endpoint` it refuses, as
`ak_channel_create` refuses an empty endpoint the runtime does not supply.

### Sources

- **A file**, JSON, YAML or TOML by its extension (`.json`, `.yaml` or `.yml`, `.toml`). The
  document is the file's section named by the prefix, found by walking its parts down the nested
  sections as written (a section that is missing contributes nothing, one that is not an object is
  refused), so the host's own `appsettings.json` can carry it beside sections the host reads
  itself; the other sections are not the engine's, and are left alone. A file with no such section
  contributes nothing. With no prefix, the document is the whole file, so the sections of its own
  host that a file also holds are unknown keys, logged. A missing file is refused, unless the host
  marks it optional, as .NET's `AddJsonFile(path, optional: true)` does.
- **The environment**: the variables whose name starts with the prefix and `__`, read once, when
  the runtime is created. The rest of a name is the key's path, its parts joined by `__`, compared
  without case:
  `ArmoniK__Client__Grpc__ChannelDefaults__Http2__SimultaneousCallsPerConnection=4`. A value is text, parsed by the schema's type for that key. The environment needs a prefix: with none, every
  variable of the process would be a key, and the log would name every one of them, so an
  environment source with no prefix is refused. A key that holds a list, such as
  `Grpc.Receive.Compression`, is read from its one variable, whose value is a JSON array:
  `ArmoniK__Client__Grpc__ChannelDefaults__Grpc__Receive__Compression=["Zstd","Gzip"]`. It is the
  one form: a bare value (`Zstd`, or `Zstd,Gzip`, which is not split at the commas) and keys under
  the list's (`...__Compression__0`, as .NET's providers render an array, which the engine's
  environment does not) are refused, as is JSON that is not an array. An empty array states an
  empty list, and `[]` is the only way to: a variable set to nothing is refused. Each element is
  read as the element's type, a name matched without case as the other variables' values are.
- **Pairs**: a JSON object whose names are keys' paths, their parts joined by `__`, under no
  prefix, and whose values are text, read as the environment's values are, by the schema's type
  for that key: `{"ChannelDefaults__Http2__SimultaneousCallsPerConnection": "4"}`. It is what a
  binding sends for a command line it has parsed. Pairs state no list, and neither does a
  command line: a key that holds a list is refused, by its path.
- **A document**: JSON in the schema's vocabulary, with no prefix around it. It is what a binding
  sends for an object set in code.

A later source overrides an earlier one option by option: a structure field by field, an
alternative whole when two sources state different ones, as `ChannelOptions::over` merges two
documents. A list is a value, which `over` takes whole as it takes a text or a number: the later
source's list replaces the earlier one's, an empty list included, and a source that does not state
it leaves the earlier one's. Two lists are never joined.

A list is stated by a file, a document, or an environment variable; the command line and pairs
cannot state one, since a command line is parsed by .NET's configuration into keys and text, and
the engine keeps no convention of its own for a list of them.

### Refusals

The first refusal ends the load and names its source - the file's path, `the environment`,
`pairs` or `a document` - then, when a key is at fault, the key's path within the prefix. Refused:

- a file that does not parse, with its line, as .NET reports one, and a file whose extension is
  none of the four;
- a missing file not marked optional;
- an environment source with no prefix;
- a document that is not JSON, a prefix's section that is not an object, and pairs that are not
  a JSON object of text values;
- a value that is not of its key's type, an environment or pair value being parsed as that type;
- a list in pairs, whatever its form, with its path and the sources that do state one, and in the
  environment a list that is not a JSON array, with its path and the form that is, or an element
  that is not of its type, with the element's index in the path (`Compression.1`).

A key under the prefix that the schema does not declare is not refused: it is logged, with its
source and its path, and the load goes on. Within an alternative - how the server is verified, who
the client is, which proxy - a key that names none of its variants is such a key, and the option
keeps what an earlier source gave it.

A value is never quoted, a password being one. Through the C structure, what is malformed in it - a
kind it does not name, a nonzero `reserved`, a flag it does not know, a value on an environment
source, a prefix beside `AK_CONFIG_NO_PREFIX` - is `AK_STATUS_INVALID_ARG`, before any source is
read.

### Rust

The loader lives in `armonik-transport` and is generic over the document it reads: each source is
read into that document's type, the keys it does not declare logged and left out, and the documents
merge by the type's own `over`. The keys left out are those serde passes over, each with its path,
which the loader's deserializer records as it skips them; an alternative reads a variant it does not
know as none, which the types' own deserialization allows. A configuration with no source, or none
that contributes, loads the document's default. The document is `RuntimeOptions`, in
`armonik-transport` with its schema: the FFI and the `armonik` crate read the same one.

```rust
/// A document a configuration can be read into, merged one over another.
pub trait Document: serde::de::DeserializeOwned + Default {
    fn over(self, earlier: Self) -> Self;
}

/// Where a configuration comes from, in the order the sources are added.
pub struct Configuration { /* prefix, sources */ }

impl Configuration {
    /// Under `ArmoniK__Client__Grpc`, with no source.
    pub fn new() -> Self;
    /// Under `prefix`, or under none when it is empty.
    pub fn with_prefix(prefix: &str) -> Self;
    pub fn file(self, path: impl Into<PathBuf>) -> Self;
    pub fn optional_file(self, path: impl Into<PathBuf>) -> Self;
    pub fn environment(self) -> Self;
    pub fn pairs(self, pairs: impl IntoIterator<Item = (String, String)>) -> Self;
    /// Pairs as a JSON object of text values, which `AK_SOURCE_PAIRS` carries.
    pub fn pairs_json(self, json: impl Into<String>) -> Self;
    pub fn document(self, json: impl Into<String>) -> Self;

    /// Reads the sources, in order, into one document, each key it does not declare logged.
    pub fn load<D: Document>(&self) -> Result<D, ConfigRefusal>;
}
```

The FFI and the `armonik` crate both call `load::<RuntimeOptions>()`.

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
    ak_bytes_in prefix;           /* empty: ArmoniK__Client__Grpc; with AK_CONFIG_NO_PREFIX it must be
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
runtime reaches the host. `ak_channel_create` takes an empty endpoint as the runtime's `Endpoint`,
and refuses one when the runtime has none. A refusal comes back through `ak_error`, as an option's
does, its detail naming the source and the path.

### .NET

```csharp
var configuration = new NativeConfiguration()                // or NativeConfiguration(prefix)
                      .LoadConfigFromFiles("appsettings.json", "appsettings.Production.yaml")
                      .LoadConfigFromEnvironment()
                      .LoadConfigFromCommandLine(args)
                      .LoadConfigFromObject(new RuntimeOptions { MemoryCeiling = 1L << 31 });
await using var runtime = NativeRuntime.Create(configuration);
```

The prefix is `ArmoniK__Client__Grpc` unless the constructor is given one; `""` is none, which the
binding passes as `AK_CONFIG_NO_PREFIX`. `LoadConfigFromFiles` and `LoadConfigFromEnvironment` add a
source the engine reads, and what the engine refuses in one surfaces at `NativeRuntime.Create`.
`LoadConfigFromOptionalFiles` adds files the host marks optional, which contribute nothing when they
do not exist. `LoadConfigFromCommandLine` parses the arguments with an `IConfiguration` holding the
command-line provider alone and adds the section under the prefix - the whole tree with none - as
pairs, each key's path joined by `__` and its value as text, so that the engine logs an unknown key
and types a value as it does the environment's; `LoadConfigFromObject` serializes its object as a
document, writing only the options set, so that a default does not override an earlier source. No
`IConfiguration` is taken or returned. A command line states no list, so a list option, such as
`Receive.Compression`, is set through a file, `LoadConfigFromObject` or the environment, the
engine refusing one on a command line by its path.

The engine reads every source, an object's document included, so the binding does not know the
delivery window a channel ends up with. It reads it back: once a channel is created, the binding asks
the engine for the window the channel's own options and the runtime's `ChannelDefaults` settled
(`ak_channel_delivery_window`, decided 2026-10-07) and sizes the channel's rings from the answer. A
`ChannelDefaults` window stated in any source therefore reaches every channel of the .NET binding,
and a window past what a ring can hold (`NativeRuntime.MaxDeliveryCredits`) is refused when the
channel is created, whichever source stated it.
