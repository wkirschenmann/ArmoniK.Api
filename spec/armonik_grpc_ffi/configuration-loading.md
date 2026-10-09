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
  them. A source is a document, judged whole against the schema: a key it does not declare, the
  root's included, a value of the wrong type or out of its bounds and a missing mandatory field
  are refused by their path; a value is never quoted back.
- Two documents merge by `ChannelOptions::over`: a struct field by field, but for a struct with
  a mandatory field, which is stated whole. An alternative (an enum: TLS verification, client
  identity, proxy, receive windows) is whole when the two state different variants, and merges as
  the variant's payload does when they state the same one. A variant that carries nothing is
  written as its name, `"None"`, and a variant that carries something as an object of one key,
  `{"Url": {"Address": "..."}}`.

## Decided (2026-10-06, the shape confirmed 2026-10-07)

- **Sources**: any number of files, the environment, and what the host states itself - its command
  line, or values written in its code. The host lists them in the order it wants, a later one over
  an earlier one: typically the files from the least to the most important, then the environment,
  then its own values.
- **File formats**: JSON, YAML and TOML.
- **Environment**: `__` as the separator, as .NET's; the prefix is the host's to choose and is
  always given (decided 2026-10-09: no loader, no ABI entry and no binding takes a default that
  silently means one prefix or none). `ArmoniK__Client__Grpc` (decided 2026-10-07) is the value a
  host that keeps the engine's options beside its own passes: `ArmoniK__Client__` is the family of
  the ArmoniK client's configuration, and `Grpc` its gRPC part, which this engine reads. A prefix is
  a path: its parts, joined by `__` (a `:`, as a .NET section's path is written, is read as `__`),
  are the nested sections of a file or of a document, each key compared as written
  (`{"ArmoniK": {"Client": {"Grpc": {...}}}}`), and the start of a variable's name. An empty prefix
  is a prefix like another, and takes everything: the whole file is the engine's. Read once, when
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
- **A key left out and a key set to a variant differ** (2026-10-08): a source that leaves an option
  out leaves what an earlier source set, or the default, unless the option is an optional field of
  a group stated whole (2026-10-09), and a source that wants none says so with
  the variant `None`, `"None"`, which `Transport.TcpKeepalive`, `Http2.KeepAlive`, `Http2.IdleTimeout`,
  `Grpc.Deadline` and the units of `Grpc.OutboundTraffic` have, as `Http2.SimultaneousCallsPerConnection`
  has `"FromServer"` (decisions.md, "How an option is turned off by a variant").
- **A source is a document, judged whole against the schema** (2026-10-09). A file or a
  document is judged on the section its prefix names, and nothing outside that section is looked at: a host's `Logging` and
  `Serilog` are its own. The environment, pairs and a command line are made one document from the
  names that start with the prefix, and a name that does not is never looked at. Refused, by the
  full key path and never quoting a value: a key the schema does not declare, at the root of the
  document as below it (`Endpiont`, `Http2.Send.FramesPerWrit`, `Http2.KeepAlive.Ping.IntervalSecond`,
  the variable `ArmoniK__Client__Grpc__Endpiont`), a key of an alternative that names none of its
  variants (`{"Pingg": {...}}`, refused naming the variants it accepts, `None, Ping`), a name that
  is none of them (`KeepAlive=Pingg`, refused at the option's own path), a value of the wrong type,
  a value out of the bounds the schema states and a mandatory field that is missing. A misspelling
  would leave an option at what an earlier source or the default gave it, and a line in the log is
  too little to say so. A configuration written for a later engine is not a reason to load it: it
  is written for that engine, and an older one that drops what it does not know runs something else
  than the file states. With an empty prefix the whole file is the engine's and is judged whole,
  so a host's own `Logging` section is refused at `Logging.LogLevel`, where the runtime's `Logging`
  group meets it; a host reads its file with `ArmoniK:Client:Grpc` and the two never meet.
  decisions.md has the reasons.
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
5. **Said**: a key the schema does not declare, at the root of a document or anywhere else, from
   any source, is refused, by its path; so is a value that does not fit its key's type or its
   bounds; a secret (a password, a key) never appears in a message.
6. **Conformance**: one set of fixtures - sources in, resulting document or refusal out - run
   against the Rust loader directly and against each binding through the ABI, and each run through
   a JSON Schema validator as well, which has to accept and refuse what the loader does
   (`tests/schema.rs`).

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
        "MemoryCeiling": { "SoftMiB": 2048 },
        "ChannelDefaults": {
          "Http2": { "SimultaneousCallsPerConnection": { "Limit": 4 } },
          "Transport": { "Tls": { "ClientCertificate": { "P12": { "Path": "client.p12" } } } }
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
  refused), and it is judged whole against the schema. The host's own `appsettings.json` can carry
  it beside sections the host reads itself, `Logging` and `Serilog` among them: they lie outside the
  section, and are never looked at. A file with no such section contributes nothing. With an empty
  prefix, the document is the whole file, and the file belongs to the engine alone: every key of it
  has to be one the schema declares, so that the sections of a host are refused, `Logging.LogLevel`
  being the first a host's file usually holds (observability.md). A missing file is refused, unless
  the host marks it optional, as .NET's `AddJsonFile(path, optional: true)` does.
- **The environment**: the variables whose name starts with the prefix and `__`, read once, when
  the runtime is created, and built into one document: the rest of a name is the key's path, its
  parts joined by `__`, compared without case:
  `ArmoniK__Client__Grpc__ChannelDefaults__Http2__SimultaneousCallsPerConnection__Limit=4`. A
  value is text, parsed by the schema's type for that key. A variable whose name does not start
  with the prefix is never looked at; one under it that names no key,
  `ArmoniK__Client__Grpc__Endpiont`, is refused, as a key of a file would be. The environment needs
  a prefix: with an empty one, every variable of the process would be a key, so an environment
  source with an empty prefix is refused. A key that holds a list, such as
  `Grpc.Receive.Compression`, is read from its one variable, whose value is a JSON array:
  `ArmoniK__Client__Grpc__ChannelDefaults__Grpc__Receive__Compression=["Zstd","Gzip"]`. It is the
  one form: a bare value (`Zstd`, or `Zstd,Gzip`, which is not split at the commas) and keys under
  the list's (`...__Compression__0`, as .NET's providers render an array, which the engine's
  environment does not) are refused, as is JSON that is not an array. An empty array states an
  empty list, and `[]` is the only way to: a variable set to nothing is refused. Each element is
  read as the element's type, a name matched without case as the other variables' values are.
- **Pairs**: a JSON object whose names are the prefix, `__` and a key's path, its parts joined by
  `__`, and whose values are text, read as the environment's values are, by the schema's type for
  that key: `{"ArmoniK__Client__Grpc__ChannelDefaults__Http2__SimultaneousCallsPerConnection__Limit": "4"}`.
  A name that does not start with the prefix is never looked at, and the rest are built into one
  document as the environment's are, so that with an empty prefix every name is the engine's. It
  is what a binding sends for a command line it has parsed, every argument of it. Pairs state no
  list, and neither does a command line: a key that holds a list is refused, by its path.
- **A document**: JSON, of which the section the prefix names is the engine's, in the schema's
  vocabulary, as a file's is; with an empty prefix the whole document is. It is what a binding sends
  for an object set in code, which it nests under the prefix.
- **A variant that carries nothing**: in the environment and in pairs, the key's value is its name,
  matched without case - `ArmoniK__Client__Grpc__ChannelDefaults__Transport__Proxy=None`. A variant
  that carries something is its keys under the alternative's - `...__Proxy__Url__Address=...`, and
  `...__Proxy__Url__Credentials__Username=...` for the group within it - and a value beside keys under
  it is refused, by its path.

A later source overrides an earlier one option by option: a structure field by field, an
alternative whole when two sources state different ones, as `ChannelOptions::over` merges two
documents. A structure with a mandatory field - a `Probe` with its `IdleSeconds`, a `Ping` with its
`IntervalSeconds`, a `Url` with its `Address`, a proxy's `Credentials` with its `Username` and its
`Password` - is stated whole, so that no source leaves it half
stated (2026-10-09): a later source that states it replaces the earlier one's, its optional fields
taking what it gives or their default, and one that omits a mandatory field is refused, by the
path of the structure, whatever an earlier source states. The rule comes from the shape of the type
and names no option. An empty string is a value, never an absent one: a `Credentials` stated with
an empty `Password` states it, and the proxy is sent that password, not one filled from the URL of
a proxy the environment names, which applies only when no `Credentials` is stated (2026-10-09).
A list is a value, which `over` takes whole as it takes a text or a number: the later source's list
replaces the earlier one's, an empty list included, and a source that does not state it leaves the
earlier one's. Two lists are never joined.

A list is stated by a file, a document, or an environment variable; the command line and pairs
cannot state one, since a command line is parsed by .NET's configuration into keys and text, and
the engine keeps no convention of its own for a list of them.

### Refusals

The first refusal ends the load and names its source - the file's path, `the environment`,
`pairs` or `a document` - then, when a key is at fault, the key's path within the prefix. Refused:

- a file that does not parse, with its line, as .NET reports one, and a file whose extension is
  none of the four;
- a missing file not marked optional;
- an environment source with an empty prefix;
- a document that is not JSON, a prefix's section that is not an object, and pairs that are not
  a JSON object of text values;
- a value that is not of its key's type, an environment or pair value being parsed as that type,
  and a value out of the bounds the schema states for its key (`minimum`, `maximum`, `minLength`),
  but for `minLength` of `Transport.Proxy.Url.Address` and of `Transport.Proxy.UrlWithCredentials`,
  which the loader does not check.
  A number a file or a document writes with a fraction of zero, `2.0` or `1e1`, is the integer it
  equals, as JSON Schema reads one, and `2.5` is refused; a text of the environment or of pairs is
  parsed by its key's type, so `2.0` is no integer there;
- a mandatory field that is missing;
- a list in pairs, whatever its form, with its path and the sources that do state one, and in the
  environment a list that is not a JSON array, with its path and the form that is, or an element
  that is not of its type, with the element's index in the path (`Compression.1`).

A key that a struct does not declare is refused, by its path, at the root of the document, which
is the section the prefix names, as below it; so is a key of an alternative - how the server is
verified, who the client is, which proxy - that names none of its variants, the refusal naming the
variants it accepts. A name that carries nothing and is none of its variants is refused, as is a
variant that carries nothing given a value.

A value is never quoted, a password being one. Through the C structure, what is malformed in it - a
kind it does not name, a nonzero `reserved`, a flag, none being defined, a value on an environment
source, a byte view that is null with a length - is `AK_STATUS_INVALID_ARG`, before any source is
read.

### Where the loader and the schema each decide

The loader's reader decides, and no validator runs beside it (decided 2026-10-09). The reader
refuses a key by the type that declares it, and a value by its type; the bounds the schema
states are checked where the value is read, by a small function on the
field's `deserialize_with` (`within::between`, `within::at_least`, `within::non_empty` and the few
named for a bound of their own), so that the types stay the one source of the schema, which
`schemars` renders from them, and of what the loader accepts. A runtime validator would be a
second reader: it needs a typed document, which the environment and pairs, being text until a type
reads them, do not give without a reader of their own, and its messages would quote what the
loader's never do, and name an alternative's failure at the alternative and not at the key at
fault. The two statements of a bound, the schema's and the field's, are kept in step by
`tests/schema.rs`, which runs every fixture through the `jsonschema` crate and through the loader
and fails when one accepts what the other refuses.

### Rust

The loader lives in `armonik-transport` and is generic over the document it reads: each source is
read into that document's type, a key any struct or alternative does not declare refused, a value
out of its bounds refused, and the documents merge by the type's own `over`. The deserializer
refuses a key that is not among the fields or the variants the type declares. A configuration with
no source, or none that contributes, loads the document's default. The document is
`RuntimeOptions`, in `armonik-transport` with its schema: the FFI and the `armonik` crate read the
same one.

```rust
/// A document a configuration can be read into, merged one over another.
pub trait Document: serde::de::DeserializeOwned + Default {
    fn over(self, earlier: Self) -> Self;
}

/// Where a configuration comes from, in the order the sources are added.
pub struct Configuration { /* prefix, sources */ }

/// The prefix a caller that keeps the engine's options beside its own passes.
pub const DEFAULT_PREFIX: &str = "ArmoniK__Client__Grpc";

impl Configuration {
    /// Under `prefix`, with no source: the section of a file or of a document that is the
    /// engine's, and the start of the name of a variable or of a pair that is. An empty prefix
    /// takes everything. There is no constructor without one.
    pub fn with_prefix(prefix: &str) -> Self;
    pub fn file(self, path: impl Into<PathBuf>) -> Self;
    pub fn optional_file(self, path: impl Into<PathBuf>) -> Self;
    pub fn environment(self) -> Self;
    pub fn pairs(self, pairs: impl IntoIterator<Item = (String, String)>) -> Self;
    /// Pairs as a JSON object of text values, which `AK_SOURCE_PAIRS` carries.
    pub fn pairs_json(self, json: impl Into<String>) -> Self;
    pub fn document(self, json: impl Into<String>) -> Self;

    /// Reads the sources, in order, into one document, each judged whole: a key a struct does
    /// not declare is refused, the root's among them.
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
    AK_SOURCE_DOCUMENT = 4,       /* value: a JSON document, its prefix's section the engine's */
    AK_SOURCE_PAIRS = 5,          /* value: a JSON object, prefix__key__path to text */
} ak_source_kind;

typedef struct {
    uint32_t kind;                /* ak_source_kind; another is AK_STATUS_INVALID_ARG */
    uint32_t reserved;            /* zero; another is AK_STATUS_INVALID_ARG */
    ak_bytes_in value;
} ak_config_source;

typedef struct {
    uint32_t struct_size;
    uint32_t version;             /* zero */
    uint32_t flags;               /* none is defined; any bit is AK_STATUS_INVALID_ARG */
    uint32_t source_count;
    const ak_config_source *sources;  /* in order, a later one over an earlier one */
    ak_bytes_in prefix;           /* always the one given; empty takes everything, and a null
                                     pointer with a length is AK_STATUS_INVALID_ARG */
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
var configuration = new NativeConfiguration(NativeConfiguration.DefaultPrefix)   // the prefix is always given
                      .LoadConfigFromFiles("appsettings.json", "appsettings.Production.yaml")
                      .LoadConfigFromEnvironment()
                      .LoadConfigFromCommandLine(args)
                      .LoadConfigFromObject(new RuntimeOptions { MemoryCeiling = new MemoryCeilingOptions { SoftMiB = 2048 } });
await using var runtime = NativeRuntime.Create(configuration);
```

The constructor takes the prefix and has no overload without one: `NativeConfiguration.DefaultPrefix`,
`ArmoniK__Client__Grpc`, for a host that keeps the engine's options in its `appsettings.json` under
`ArmoniK:Client:Grpc`, whose own sections are then never looked at, and `""` for a file that is the
engine's alone, which the binding passes as an empty prefix: the whole file is judged, and the
environment is refused. `LoadConfigFromFiles` and `LoadConfigFromEnvironment` add a
source the engine reads, and what the engine refuses in one surfaces at `NativeRuntime.Create`.
`LoadConfigFromOptionalFiles` adds files the host marks optional, which contribute nothing when they
do not exist. `LoadConfigFromCommandLine` parses the arguments with an `IConfiguration` holding the
command-line provider alone and adds every key as pairs, each key's path joined by `__` and its
value as text, so that the engine takes those under the prefix and types a value as it does the
environment's; `LoadConfigFromObject` serializes its object as a document nested under the prefix,
writing only the options set, so that a default does not override an earlier source. No
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
