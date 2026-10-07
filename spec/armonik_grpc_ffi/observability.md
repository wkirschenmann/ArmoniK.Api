# Observability: what a host can see of the engine

Status: decided on 2026-10-07, to be built by T10.1 (logs and the effective configuration), T10.2
(metrics) and T10.3 (traces).

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

- `armonik-transport` emits almost nothing: one `debug!` and one span; `armonik-transport-ffi`
  nothing. h2, hyper and tonic emit `tracing` events, at debug and trace levels mostly, which reach
  no host.
- The runtime's callback runs on the library's threads, which it must not stall: it publishes and
  returns, and must not parse, allocate what it could have allocated earlier, take a lock the
  host's own code holds, or run application code.
- The .NET binding has no `ILogger`, no `EventSource` and no trace. It reports what it catches to
  whoever awaits the operation, but for the trampoline's three catches - a context handle that no
  longer names a target, a `Publish` that threw, an `Arrived` that threw - which are the boundary
  an exception must not cross back into Rust, and which say nothing.
- `ak_call_start_options` is versioned by `struct_size`: a field added at its end is absent, and
  takes its default, for a host built against a structure without it.

## Decided (2026-10-07)

- **Logs cross through a callback of their own**, which a host that wants them registers:
  `ak_runtime_set_log_callback(runtime, callback, ctx, filter)`. Until one is registered nothing
  is logged across the ABI, so a host built against an earlier header sees no event it does not
  know. A queue the host drains would be preferable, batching what crosses, but is hard to make
  generic across the languages a C ABI serves; the callback is what is built for now.
- **The log callback keeps the runtime callback's contract**: it records the event where the
  host's own thread will find it, and returns. A host whose logger runs application code - .NET's
  providers do - copies the record into a queue of its own and writes it from a thread of its own:
  the queue is the host's, and costs the ABI nothing. It is called on the library's threads, and
  on the host's own inside an `ak_*` call that logs, which is why a host takes no lock its logging
  needs around such a call.
- **Each runtime logs to its own callback.** `tracing`'s default subscriber is the process's, and a
  host may hold several runtimes: the FFI sets each runtime's own dispatcher on the runtime's
  threads and around the FFI calls made on it, so that its events reach it alone. Nothing else
  sets one, so a Rust host's own subscriber receives the engine's events.
- **What is logged before a callback exists is kept for it.** The configuration's load runs inside
  `ak_runtime_create`, before a host can register: its events - the unknown keys, the effective
  configuration - are kept, and delivered when the log callback is registered, selected by the
  runtime's filter. Only the load's events are kept, as many as the load produced, and a host that
  never registers keeps them for the runtime's life: a few lines, against the cost of losing the
  one warning a misspelled key gives.
- **The engine filters.** The filter is in `tracing`'s directive syntax, `info,h2=debug`: a key of
  the runtime's options, loaded with them (configuration-loading.md), which
  `ak_runtime_set_log_callback` takes when its `filter` is empty and replaces when it is not, and
  which `ak_runtime_set_log_filter` changes while the runtime runs, refusing a directive it cannot
  parse with `AK_STATUS_INVALID_ARG` and keeping the filter in force. A disabled event never
  crosses. By default the engine logs at `info`, and h2, hyper and tonic at `warn`; a directive
  brings them back, as a diagnosis of a proxy's GOAWAY needs.
- **The effective configuration is logged** at `info`: once when the runtime is created, and for a
  channel only when its creation states options of its own, which it does not in general. Every
  option's value is logged, the endpoint as `safe_endpoint` renders it, and not where a value came
  from. A secret never is: the options type a password as `Password`, which holds a
  `SecretString`, and a proxy URL carrying `user:password@` as `CredentialedUrl`, each rendering
  redacted, and the logger renders every value through the type's own rendering, never a generic
  serialization of the document. A key's or a certificate's path is not a secret.
- **An unknown configuration key is logged at `warn` with its source and its path, never its
  value**, since a misspelled key may hold a secret (T6.14).
- **A Rust host needs no crossing.** The engine emits ordinary `tracing` events, which a Rust
  application's own subscriber receives and its own filter selects; the filter key is read there
  and does nothing, as the memory ceilings are. The events and their fields are the same that cross
  the ABI, so a diagnosis reads alike in both languages; their targets and fields are documented
  with the engine.
- **The .NET binding takes an optional `ILoggerFactory`** on `NativeRuntime.Create`, the target as
  the category, which is what the ArmoniK ecosystem logs through; the binding registers the log
  callback when it is given one. The trampoline's two catches that have a target - a
  `Publish` or an `Arrived` that threw - log through that target's runtime's logger, by the same
  queue, since they too run on the library's thread; the one whose handle names nothing has no
  logger to reach. The binding takes
  `Microsoft.Extensions.Logging.Abstractions` as a dependency.
- **Metrics (T10.2) are one structure of counters and gauges** - calls in flight, dials, retries,
  resets, GOAWAYs - read per channel by `GrpcChannel::stats()` in Rust and per runtime, over its
  channels, by `ak_runtime_stats` across the ABI; the memory the runtime holds stays
  `ak_runtime_memory_usage`'s. The .NET binding exposes them through a `Meter`, which
  `System.Diagnostics.DiagnosticSource` brings to .NET Framework.
- **Traces (T10.3) take one ABI.** `ak_call_start_options` gains a W3C trace context at its end -
  `traceparent` and `tracestate` as bytes, empty for an untraced call - so that a host built
  against the structure without them starts untraced calls with no other change, and no second
  entry point exists. It is a field rather than a header the host writes in the call's metadata
  because the engine reads it: its events for the call carry the trace's identifiers, and it does
  not parse the metadata it is given. The engine sends the context in the call's metadata, as
  grpc-dotnet's stack does - its `HttpClient` diagnostics handler creates the `Activity` and
  writes `traceparent` - and carries the trace's identifiers in the fields of the events it logs
  for the call. The .NET binding creates the call's `Activity` as that stack does, so that a
  host's OpenTelemetry sees the same tree whichever transport it runs.
- **The engine's own spans come with T10.3**: a dial, an attempt, a retry, crossing through the
  log callback and becoming children of the call's `Activity` in .NET.

Out of V1's scope (requirements.md): exporting telemetry from the native side, to an
OpenTelemetry collector of its own. What is decided here goes to the host's pipeline instead.

## What crosses

A log record is valid for the callback alone: the host copies what it keeps, so nothing is lent
on its behalf and nothing comes back. A sketch, which T10.1 settles:

```c
typedef struct {
    ak_bytes_in key;
    ak_bytes_in value;            /* rendered as text */
} ak_log_field;

typedef struct {
    uint32_t level;               /* AK_LOG_ERROR, WARN, INFO, DEBUG, TRACE */
    uint32_t field_count;
    ak_bytes_in target;           /* armonik_transport::grpc, h2, ... */
    ak_bytes_in message;
    const ak_log_field *fields;
} ak_log_record;

typedef void (*ak_log_callback)(void *ctx, const ak_log_record *record);
```

T10.1 settles, in building it:

- the record's shape, and how it grows - a `struct_size`, as the ABI's other structures have, and
  an `ak_log_field` that cannot grow in an array without breaking its stride;
- the log callback's lifetime: whether it can be replaced or removed, and its last invocation
  against `AK_EVENT_RESOURCES_RELEASED` and `ak_runtime_destroy`, so that a host frees what `ctx`
  names knowing nothing will reach it;
- whether the kept events are delivered inside `ak_runtime_set_log_callback`, on the host's
  thread, or from the library's;
- the catalogue of the engine's events - their targets, levels, messages and fields - and the
  filter's key;
- the secret-bearing options, and a test that sets each and finds none of their values in the
  effective configuration's log, so that an option added later as plain text is caught.

T10.3 settles how a span's start and end cross.
