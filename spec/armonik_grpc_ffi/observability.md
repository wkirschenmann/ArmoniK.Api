# Observability: what a host can see of the engine

Status: decided on 2026-10-07. T10.1 (logs and the effective configuration) is built; T10.2
(metrics) and T10.3 (traces) are to build.

The logs cross through a callback a host gives when it creates the runtime, the traces through one
of their own, and the metrics are read on demand. A host that wants none of them gives none and
reads nothing.

## Why

The ABI lets a host observe two things - `ak_call_debt_of` and `ak_runtime_memory_usage` - and
tells it nothing else. A deployment that misbehaves gives a .NET operator no more than a status
code, and the questions an operator actually asks - which endpoint, which call, how long, why it
retried, what the configuration came to - have no answer on that side of the boundary.

grpc-dotnet's stack - `Grpc.Net.Client` over its `HttpClient` handler - answers them with an
`ILogger`, counters, and an `Activity` per call whose `traceparent` reaches the server. A host
that changes transport should not lose what it could see. What that stack does exactly is checked
against it before each part is built, grpc-dotnet being the reference where these documents are
silent.

## What exists

- `armonik-transport` emits `tracing` events, catalogued below; h2, hyper and tonic emit theirs,
  at debug and trace levels mostly. `armonik-transport-ffi` emits the effective configuration, and
  the warning about a filter directive it ignores.
- The runtime's callback runs on the library's threads, which it must not stall: it publishes and
  returns, and must not parse, allocate what it could have allocated earlier, take a lock the
  host's own code holds, or run application code.
- The .NET binding's trampoline has three catches - a context handle that no longer names a
  target, a `Publish` that threw, an `Arrived` that threw - which are the boundary an exception
  must not cross back into Rust. The last two report to the log.

## Decided for the logs (2026-10-07)

- **One log callback, given when the runtime is created, and never replaced or removed.** It is
  two fields appended to `ak_runtime_config` and to `ak_config`, `log_callback` and `log_ctx`,
  beside the entry points' own callback, which stays a parameter: appending is what the ABI's
  records are for, and a host built before the fields passes none. There is no setter, so there
  is no question of a callback replaced under a delivery, and none of when its context may be
  freed other than the runtime's life. `log_ctx` and the function stay valid until
  `ak_runtime_destroy` returns, or until a creation that is refused returns: the library waits for
  the deliveries under way before either, and delivers nothing after.
- **Logs are delivered as they come.** There is no queue the host drains. The callback keeps the
  runtime callback's contract - record the event where the host's own thread will find it, and
  return - and a host whose logger runs application code copies the record into a queue of its own
  and writes it from a thread of its own: the queue is the host's, and costs the ABI nothing. It
  is called on the library's threads, and on the host's own inside an `ak_*` call that logs; it
  may be called by several threads at once. It must not call this library: an event logged from
  inside it is dropped, since delivering it would enter the callback again on its own stack.
- **Events logged while the runtime is created are delivered on the host's calling thread, before
  the creation returns.** The configuration's load runs inside the creation, and its events - a key
  that an environment or pairs give twice - cannot be selected by a filter that is among what it
  loads. They are kept on the
  loading thread, which is the one thread that logs then, and delivered once the filter is known,
  each as the filter selects it. A refused creation delivers them too, selected by the default
  filter, since they say why; the callback is detached before the call returns.
- **The filter is the runtime's, defined once, at its creation.** Its key is `Logging.Filter`
  among the runtime's options, loaded as the others are, in `tracing`'s directive syntax:
  `ak_runtime_config` has no field for it, so a runtime created from it logs by the default. Only
  one runtime exists at a time, so one filter and one callback, and a creation sets the filter
  again. Nothing changes it while the runtime runs - no entry point, no hot reload - and no channel
  has a filter of its own.
  `Logging` is therefore a key of the runtime's options, which a host's own `Logging` section - the
  one `Microsoft.Extensions.Logging` reads from `appsettings.json` - meets in a file read with an
  empty prefix, the whole file then being the engine's: its `LogLevel` is refused as the unknown
  key `Logging.LogLevel`. A host keeps the engine's options under `ArmoniK:Client:Grpc` and reads
  the file with that prefix, so that its own `Logging`, `Serilog` and the rest lie outside the
  section the engine judges and are never looked at:

  ```json
  {
    "Logging": { "LogLevel": { "Default": "Information" } },
    "ArmoniK": { "Client": { "Grpc": { "Endpoint": "https://armonik.example.com:5001",
                                         "Logging": { "Filter": "armonik_transport=debug" } } } }
  }
  ```
- **Directives match by module-path segment, and `*` by text.** A directive is a level (`info`),
  a target and its level (`h2=debug`), or a target alone, which is all its levels. A target covers
  itself and the modules below it: `h2` covers `h2` and `h2::proto::connection`, not `h2x`, and
  `armonik_transport` covers `armonik_transport::grpc::channel`, not `armonik_transport_ffi`. A
  target that ends in `*` covers every target that starts with the text: `armonik_transport*`
  covers both, `hyper*` covers `hyper` and `hyper_util`, and `*` alone covers every target:
  `*=debug` is a level for every target, as `debug` alone is. When several directives cover an
  event the most specific decides: the longest target, and at the same length a segment directive
  before a `*` one, whatever order they are written in. `tracing-subscriber`'s `Targets` and
  `EnvFilter` match by text prefix, so the engine has its own matcher.
- **The default filter is `*=warn,armonik_transport*=info`; a filter the user gives replaces it
  whole.** With no filter, every target logs warnings and the engine's own - the targets that start
  with `armonik_transport`, the transport and the FFI - log at info; h2, hyper, tonic, tower and
  the rest fall under the star with no directive of their own, so a dependency added later is quiet
  without listing it. A filter replaces it whole, and a target none of its directives covers is
  off: `armonik_transport=debug` alone shows `armonik_transport` and its modules and nothing else
  - `armonik_transport_ffi` is a different target - and `armonik_transport*=debug` shows both of
  the engine's targets. A level for every target, as `*=warn` states it, is the one directive that
  covers the rest: `*=off` alone shows nothing, `*=off,h2=debug` is h2 alone, `*=error` is errors
  only, and `*=warn,armonik_transport*=info` is the default. A stray word such as
  `Information` or a misspelt `inf` is a target nothing emits, so a filter of such words shows
  nothing. A host that wants no log at all gives no callback, which costs a comparison per event.
- **No strict validation.** A directive that names a target nothing emits is no error: the
  filter says what one wants. One whose level is not a level, or that names a span or a field
  (`h2[conn]=debug`, which an event filter has no use for), is ignored and logged at warn, whatever
  the filter selects; a filter with no directive that holds, an empty one included, is the
  default.
- **One process-wide dispatcher.** The host is not Rust, so `tracing` has no subscriber but the
  engine's: the library installs it as the process's default dispatcher the first time a runtime is
  created, and it sends each event to the callback of the runtime there is, or drops it when there
  is none. Nothing is scoped to a thread, so a thread the engine starts, tokio's or its own, needs
  no set-up, and the cache `tracing` keeps of each callsite's interest cannot be filled from a
  thread no runtime owns. That cache is rebuilt, by
  `tracing_core::callsite::rebuild_interest_cache`, whenever the filter changes - at a creation,
  and at the destroy that ends the runtime, where the filter selects nothing - and an event the
  filter does not select costs a load and a comparison. A process that has a default dispatcher
  already, a Rust host's,
  keeps it, and the FFI's host receives nothing. The `armonik` crate does not use the FFI, keeps
  its own subscriber, and `armonik-transport` installs none.
- **The record.** A structure with `struct_size` first, as the ABI's records have it, the level, the
  target, the message, and the event's other values as pairs of text, every view borrowed for the
  callback's duration:

  ```c
  typedef struct { ak_bytes_in key; ak_bytes_in value; } ak_log_field;

  typedef struct {
      uint32_t struct_size;         /* sizeof the record the library built */
      uint32_t level;               /* AK_LOG_ERROR 1, WARN 2, INFO 3, DEBUG 4, TRACE 5 */
      ak_bytes_in target;           /* armonik_transport::grpc::channel, h2, ... */
      ak_bytes_in message;
      size_t field_count;
      const ak_log_field *fields;   /* in the order the event names them; NULL for none */
  } ak_log_record;

  typedef void (*ak_log_callback)(void *log_ctx, const ak_log_record *record);
  ```

  No source location and no time: the host's logger stamps its own. A field appended to the record
  lies past a smaller `struct_size`; `ak_log_field` is a fixed pair, so a datum per field would come
  as a pointer appended to the record. A value is rendered once into a buffer the thread reuses,
  so that a steady state allocates nothing, and a key is the field's static name.
- **Nothing is allocated for a disabled event.** What is not selected is decided by the callsite's
  cached interest, before an event is built.
- **An unknown configuration key is refused, not logged**, at the root of a document as below it,
  and the refusal names its path and never its value, since a misspelled key may hold a secret.
  What the load logs is a key an environment or pairs give twice, at warn.
- **The effective configuration is logged at info.** Once when the runtime is created, and once for
  each channel whose creation states options that differ from what it would take from the runtime -
  the channel's own document merged over the runtime's defaults, with its endpoint as
  `safe_endpoint` renders it. Every option is logged, rendered by its own type's `Debug`, never by
  a serialization of the document: the options type a password as `Password`, which holds a
  `SecretString`, and a proxy URL carrying `user:password@` as `CredentialedUrl`, each rendering
  redacted. A key's or a certificate's path is not a secret. The test that keeps this true walks
  the channel schema, sets each string option to a value shaped like a credential in a channel's
  own document and in the runtime's defaults, and finds in the log nothing of a secret; a string
  option that the schema does not mark `writeOnly` and that the test's list does not classify as
  plain fails it, so that an option added later as plain text is caught.
- **A Rust host needs no crossing.** The engine emits ordinary `tracing` events, which a Rust
  application's own subscriber receives and its own filter selects; the filter key is read there
  and does nothing, as the memory ceilings are. The events and their fields are the same that cross
  the ABI.
- **The .NET binding takes an optional `ILoggerFactory`** on each `NativeRuntime.Create`, the
  target as the category, which is what the ArmoniK ecosystem logs through. Its trampoline copies
  the record into a bounded queue, 16384 records, and returns; a thread of the binding's writes the
  queue to the loggers. A record that finds the queue full is dropped and counted, and the writer
  says how many it lost, as a warning, before the next it writes, or when the queue closes: a
  logger slower than the engine loses records and reports it, rather than holding an engine thread
  or the process's memory. A provider that throws costs its record and not the writer. The queue is
  flushed when the runtime is disposed, after the destroy, which is when the context is released,
  and when a creation is refused or throws, so that what its load logged is written; a refused
  creation hands the process's current log back to the runtime that lives, if one does. The
  message's braces are doubled in the `{OriginalFormat}` a structured provider reads. The
  trampoline's two catches that have a target - an event that could not be handed to its call's
  reader, a reader that could not be woken - log through the same queue, at error with the
  exception, under the category `ArmoniK.Api.Client.RustGrpcChannel`; the one whose handle names
  nothing has no logger to reach. The `ak_runtime_create` overload reaches only the default filter,
  the others `Logging.Filter`. The binding takes
  `Microsoft.Extensions.Logging.Abstractions` as a dependency.
- **Metrics (T10.2) are read on demand, with no callback**: one structure of counters and gauges -
  calls in flight, dials, retries, resets, GOAWAYs - read per channel by `GrpcChannel::stats()` in
  Rust and per runtime, over its channels, by `ak_runtime_stats` across the ABI; the memory the
  runtime holds stays `ak_runtime_memory_usage`'s. Pushing each measurement would cost the engine
  a crossing per measurement, where a read costs one per collection. The .NET binding exposes them
  through a `Meter`'s observable instruments, which its collector reads at its own pace, the
  binding reading `ak_runtime_stats` then. `System.Diagnostics.DiagnosticSource` brings the
  `Meter` to .NET Framework.
- **Histograms are set aside**, all of them: which ones, how the engine would build them, and how
  one would reach a .NET host, whose `Meter` has no observable histogram.
- **A call's trace context is a field of `ak_call_start_options`** (T10.3), appended after its
  last: `traceparent` and `tracestate` as bytes, empty for an untraced call, so that one entry
  point starts both. It is a field rather than a header the host writes in the call's metadata
  because the engine reads it: its events for the call carry the trace's identifiers, and it does
  not parse the metadata it is given. The engine sends the context in the call's metadata, as
  grpc-dotnet's stack does - its `HttpClient` diagnostics handler creates the `Activity` and
  writes `traceparent` - and carries the trace's identifiers in the fields of the events it logs
  for the call. The .NET binding creates the call's `Activity` as that stack does, so that a
  host's OpenTelemetry sees the same tree whichever transport it runs.
- **The engine's spans cross through a callback of their own** (T10.3), which a host that wants
  them gives as the log callback is given, when the runtime is created. A dial, an attempt, a
  retry: each span is a record of its own - its trace's and its own identifiers, its parent's, when
  it started and ended, its attributes - not a level and a message, and what selects them is
  sampling, not a log level, so a host wanting traces without logs, as an OpenTelemetry pipeline
  may, gives this one alone. A call's logs carry its trace's identifiers, which is what correlates
  the two. The callback keeps the log callback's contract and lifetime, and the process-wide
  dispatcher sends it the spans of the runtime there is; a span ended before one is registered is
  not kept. Which spans are built is configured, and none is built while no trace callback is
  given, so that an engine nobody traces spends nothing on its spans. The .NET binding makes each
  span a child of the call's `Activity`, from a thread of its own.

Out of V1's scope (requirements.md): exporting telemetry from the native side, to an
OpenTelemetry collector of its own. What is decided here goes to the host's pipeline instead.

## What the logs cost

Measured on a Windows laptop, in release, by
`cargo run --release -p armonik-transport-ffi --example log_cost -- 10000000 15`: each figure the
minimum and the median of fifteen repetitions of ten million events (a tenth for the delivered
ones), net of the empty loop. Another process on the machine moves the median more than the
minimum, which is the better figure.

| What | ns per event |
|------|--------------|
| An event above the filter's highest level (`trace!` under the default) | 0.2 min, 0.4 median |
| An event the filter does not select by target (`info!` at `h2::...` under the default) | 0.3 min, 0.5 median |
| An event delivered to a callback that counts, message only | 74 min, 91 median |
| The same with three fields (an integer, a string, a boolean) | 138 min, 191 median |
| `ak_runtime_create` with a callback, against with none (a process with few callsites) | 0.24 ms against 0.24 ms median |

A runtime created with no callback sets the filter to select nothing, so its events cost the first
row. The delivered rows are the engine's whole side: rendering the values into the thread's buffer
and the call, with a callback that does nothing. Each value is rendered as text and each event's
message is formatted, which a static message and borrowed values would not need; the engine uses
neither. The creation's cost is the cache's rebuild, which scales with the callsites a
process has registered.

## The engine's events

A target is the module that emits it. A host selects by target and level (`Logging.Filter`).

| Target | Level | Message | Fields |
|--------|-------|---------|--------|
| `armonik_transport::configuration` | warn | the configuration gives a key twice, which is taken from the later | `source`, `key` |
| `armonik_transport_ffi::config` | info | the runtime's effective configuration | `endpoint`, `memory_ceiling`, `memory_hard_ceiling`, `channel_defaults`, `log_filter` |
| `armonik_transport_ffi::config` | info | the channel's effective configuration | `endpoint`, `options` |
| `armonik_transport_ffi::log` | warn | the log filter holds a directive that is not understood, which is ignored | `directive` |
| `armonik_transport::grpc::channel` | debug | channel created, channel closed | `endpoint` |
| `armonik_transport::grpc::channel` | debug | dialling | `endpoint` |
| `armonik_transport::grpc::channel` | debug | the HTTP/2 session opened | `endpoint` |
| `armonik_transport::grpc::channel` | debug | the HTTP/2 session closed, or ended | `endpoint`, `error` |
| `armonik_transport::grpc::channel` | warn | the dial failed | `endpoint`, `error` |
| `armonik_transport::grpc::channel` | error | the engine panicked while dialling | `endpoint` |
| `armonik_transport::grpc::driver` | debug | the call failed and is retried after a backoff | `method`, `attempt`, `code`, `wait_ms` |
| `armonik_transport::grpc::driver` | debug | the request never reached the peer's application, and is sent again | `method`, `reason` |

An event is emitted outside the engine's own locks where the engine owns the lock, since a host's
callback runs inside the emitting call. No field carries a message's payload, a metadata value, or
a credential.

## T10.2 and T10.3 settle

T10.2 settles the structure's counters and gauges.

T10.3 settles the span record and how a span's start and end cross - one record at its end, or
one at each - how the spans built are configured and sampled; the trace callback's lifetime
follows the log callback's.
