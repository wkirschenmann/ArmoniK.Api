# Observability: what a host can see of the engine

Status: decided on 2026-10-07, the metrics on 2026-10-08 and 2026-10-09. T10.1 (logs and the
effective configuration) is built; T10.2 (metrics) is decided and T10.3 (traces) is to build.

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
  the creation returns.** The configuration's load runs inside the creation, and its events - the
  unknown keys - cannot be selected by a filter that is among what it loads. They are kept on the
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
  one `Microsoft.Extensions.Logging` reads from `appsettings.json` - meets in a file read with no
  prefix: its `LogLevel` is logged as the unknown key `Logging.LogLevel`, and the filter is not
  touched. Under the default prefix the two do not meet.
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
- **Unknown configuration keys are logged at info**, with their source and their path, never their
  value, since a misspelled key may hold a secret.
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
- **Metrics (T10.2) are read on demand, with no callback**, as "Decided for the metrics" below
  says. Pushing each measurement would cost the engine a crossing per measurement, where a read
  costs one per collection.
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

## Decided for the metrics (2026-10-08, 2026-10-09)

- **Counters and gauges, read on demand.** One structure holds them: read per channel by
  `GrpcChannel::stats()` in Rust, and over the channels of a runtime by `ak_runtime_stats` across
  the ABI. The memory the runtime holds stays `ak_runtime_memory_usage`'s. The .NET binding exposes
  the structure through observable instruments of `Meter`s, which a collector reads at its own
  pace; `System.Diagnostics.DiagnosticSource` brings the `Meter` to .NET Framework.
- **What the engine counts, as monotonic counters:**

  | Group | Counter | Counted where |
  |-------|---------|---------------|
  | calls | calls started | a call's creation |
  | calls | calls ended, per gRPC status, 17 slots | a call's end, by the status it ends with |
  | calls | messages sent, messages received | a message taken from the call's request stream once, whatever the attempts that send it, a message read off its response |
  | calls | retries, per failure origin, 41 slots | the policy sending a failed call again |
  | calls | calls not replayable | a call whose messages outgrow `Grpc.OutboundTraffic.Replay.MaxPerCallKiB` or the channel's `MaxPerChannelKiB`, so that it is never tried again |
  | calls | transparent resends | a request the peer's application never saw, sent again at once |
  | throttle | retries refused by the throttle | a retry the policy would send and the adaptive estimate stops |
  | connections | dials tried, succeeded and failed | a dial's start, and its outcome |
  | connections | connections closed, per reason, 8 slots | the end of an open HTTP/2 session |
  | connections | streams reset by the server, per HTTP/2 error code, 15 slots | an RST_STREAM frame read off a connection, the call it ends or not |
  | bytes | wire bytes sent and received | the HTTP/2 bytes a connection writes and reads, below the engine's own framing and above TLS's |
  | bytes | message bytes before compression, message bytes sent | the length of a message as the caller wrote it, and as the engine sent it, the gRPC prefix of five bytes in neither |
  | host | waits on a full `Grpc.Host.Receive.Window` | a delivery that finds every credit spent, reported by the embedding crate |
  | host | waits and refusals at the memory ceiling | a read held back by the first threshold, a lend refused with `AK_STATUS_BUDGET_BUSY`, a received message refused at the second; reported by the embedding crate |

  The slots are fixed, so that the structure has one size: a status is its own number, 0 to 16.
  A retry's origin is the failure the policy named, as `Cause` names them - a status (16 slots,
  one per code but OK), an HTTP status that no gRPC status states (408, 429, 500, 502, 503, 504,
  and one slot for any other), a reset by its HTTP/2 error code (14 slots and one for a code
  RFC 9113 does not list), a pushback, a dial, a connection (which a GOAWAY that left the call
  unprocessed and a request never sent are counted as, the policy reading them so). A connection
  closes for one of: the peer's GOAWAY, a keepalive that timed out, the engine's idle timeout, an
  I/O error, the engine's own close of the channel, the peer closing the stream of bytes with no
  GOAWAY, an HTTP/2 protocol error, or something else. Dials succeeded is counted beside dials
  tried and failed, because a dial in flight is neither succeeded nor failed.
  What distinguishes them is read where hyper does not say it: the engine's connection wrapper,
  the one that holds writes back, also follows the HTTP/2 frames of what the peer sends - each
  frame's 9-byte header across the chunks a read arrives in, skipping its payload, and the
  error code of a GOAWAY or an RST_STREAM, the first four bytes of a reset's payload and the second four of a GOAWAY's - at a
  comparison per frame. It needs the bytes a read has filled, which hyper's read cursor does not
  lend, so the wrapper reads them back from the buffer it handed down, in one `unsafe` block. A
  GOAWAY frame read marks the connection; an RST_STREAM frame counts the reset and its code. A
  session's end is given the first reason that holds, in this order:
  1. the engine ended it - an idle timeout, a channel closed, the runtime dropping it - which
     marks the session before the engine lets go of it;
  2. a GOAWAY frame was read: the peer's GOAWAY;
  3. the error is hyper's `is_timeout()`, which a keepalive that timed out returns in the version
     the workspace pins: a keepalive timeout;
  4. the end of the stream was read: the peer closing it with no GOAWAY, whether h2 reports the
     end as an error or as a clean end;
  5. an `io::Error` is among the error's causes: an I/O error;
  6. any other error: a protocol error;
  7. a clean end with none of these: something else.
  A test pins each reason against a server that makes it, so that a change of hyper's reports shows.
  A session dropped with its runtime is counted as the engine's own close.
  Every dial that succeeds yields exactly one close, so that open connections, dials succeeded
  less closed, does not drift: a dial that lands on a channel already closed, whose connection
  is dropped unused, counts as failed.
    A request never sent is sent again once without an attempt, and so is one that a GOAWAY left
  unprocessed or a reset with REFUSED_STREAM refused, those two sharing one allowance, as the driver
  has it. Such a resend is counted as a resend and never as a retry. A further one is the
  policy's, and counts as a retry of its own origin: the connection's for a request never sent
  or a GOAWAY, the reset's for a refusal. A message's bytes are counted once
  whatever the attempts that send it, both before compression and as sent, so that the gain is
  not skewed by a retry.
- **Derived at read, never counted**, and computed by the binding: current calls (started less
  ended), failed (ended with a status but OK), deadline-exceeded and unimplemented (slots of the
  status array), open connections (dials succeeded less closed), dials pending (tried less
  succeeded and failed), the totals of the arrays, and the compression gain (one less the ratio of
  the message bytes sent to the message bytes before compression).
- **Gauges, read from state at collection time**: the throttle's current cap of first attempts,
  whether its retries are open or closed, the calls waiting for a turn at the cap, and the calls
  waiting for a stream. The first three are the adaptive estimate's own state, read then. A call
  waits for a stream when no open session has room under both the server's limit and
  `Http2.SimultaneousCallsPerConnection`, and it waits for a session to open or to have room:
  the call's own counters hold that it does while it does, and a read counts the live calls that
  hold it. A call that h2 holds inside a session, past the limit the server announced after the
  call was placed, is not counted. A runtime has one estimate per channel, so a gauge is read
  over its channels: the caps add up as the rates they allow, with the number of channels
  capped, and the channels whose retries are closed are counted. No channel capped has no cap, which
  the instrument leaves unreported; it is not a cap of zero, which would forbid every call.
- **A counting point of a message or a byte is a relaxed load and a relaxed store, shared with no
  other writer.** The engine supports
  one thread per channel - the FFI's runtime - and tokio's multi-thread runtime, which a Rust user
  has, and counts correctly under both with no atomic that two writers update:
  - What a message or a byte touches is counted by the one task that writes it, in a block of
    counters that task owns: the request's body for what leaves, the call's driver for what
    arrives, the connection's task for what the connection reads and writes. A counter is
    never updated by a read-modify-write, and the blocks of different tasks
    of one call or connection sit on cache lines of their own.
  - What happens once or rarely per call - a retry, a resend, a refusal - is added to the
    totals of a shard of the registry, under that shard's lock. The shard is the calling
    thread's, chosen once from a number each thread draws on its first count, so that threads do
    not meet on a lock unless there are more of them than shards. Dials and the closes of
    connections are added under the one lock that the connections' registry has.
  - Each call is in the registry from its start to its end, in the shard of the thread that
    started it, and leaves its counters in that shard's totals, under the same lock, when it ends:
    a call that is never driven ends CANCELLED when it is dropped. A connection is in the
    connections' registry from its dial's success to its end in the same way. A read sums the
    totals of every shard and the blocks of every live call and connection, a shard at a time
    under its lock, so that a call is counted once - live or finished, never both and never
    neither - and a shard never holds more ends than starts: a derived current count is not
    negative. A long stream's
    numbers are therefore current at each collection: totals flushed at a call's end alone would
    leave a long call invisible until it closed, which a collection must not do. The read runs once
    per collection, and not on any path a call takes.
  - What a call writes after its driver has ended it, such as the tail of a request body a closing
    stream still polls, is not counted.
- **The `metrics` feature.** A Cargo feature of `armonik-transport`, and one of
  `armonik-transport-ffi` that enables it. Both are off by default, the FFI included. Without it,
  a counting point is an empty function, the registry has no state, and a call or a connection
  carries no counter. `GrpcChannel::stats()` exists in both builds and returns an empty `Stats`
  when the feature is off, which says so with `counting` false. `Metrics`, the registry, is
  shareable: a channel built with the handle another holds counts into the same registry, and its
  `stats()` is that registry's. The FFI makes one for its runtime and gives it to every channel,
  which is how a runtime reads all its channels as one and counts a runtime-wide event once. The
  host group is the embedding crate's to report - the transport names an event and counts it, as
  it counts what its `ReadGate` makes it wait for - and the FFI reports its three through the
  registry its runtime shares: a call's delivery window through the call's own counters, the
  memory ceiling's waits and refusals through the registry.
- **One ABI for both builds.** `ak_runtime_stats` is always in the header, and answers `AK_STATUS_OK`
  with an empty structure from a library built without the feature, never a not-supported status:
  a host built once runs against either library. The structure is the host's to size, and the
  library's to fill: a third kind of record beside the options the host fills and the records the
  library fills whole. It starts with `struct_size`, `version`, `flags`
  and `reserved`; the host sets `struct_size` to the size of the record it was built with, which
  is at least those four fields, and the other three to zero, which are refused otherwise. The
  library writes the eight-byte words that lie within it and sets `struct_size` to what it wrote, and
  `flags` to what it is. A host that passes the four fields alone learns only whether the library
  counts. `flags` carries `AK_STATS_COUNTING` when it does. The record's fields past the
  four are `uint64_t` and `double` alone, in 8-byte steps after a head of 16 bytes, so that its
  layout is one on x86, x64 and arm whatever each aligns an eight-byte integer to. "Empty" is
  therefore stated twice: the flag is clear, and every counter is zero.
  An array is as long as the constant the header gives, and a slot added to one is a new array or
  a new ABI version; a field is appended. The structure is observational and outside the formal
  model.
- **The .NET binding registers the engine's instruments only when the library counts.** It reads
  the structure once when the runtime is created, and registers nothing of the engine's when the
  flag is clear. A `Meter` per
  group, under the prefix `ArmoniK.Api.Client.RustGrpcChannel`: `.Calls`, `.Throttle`,
  `.Connections`, `.Bytes` and `.Host`. Every instrument of the engine's is observable, and reads
    `ak_runtime_stats` when a collector collects: no listener, no ABI call. The dropped logs, which
  are the binding's own, are the one instrument registered whatever the structure says, and the
  one that does not read it. A host filters with `AddMeter`, with views
  or with a `MeterListener`, and the engine has no option for it. A tag is the status code, or the
  reason or origin of what is counted, and never a method name: a method name is unbounded, and
  a series is kept per tag value.
  A runtime is read over all its channels, so no instrument carries an endpoint: a breakdown per
  endpoint is a read per channel, which the ABI does not offer.
- **The instruments**, named by OpenTelemetry's guidance - lowercase, dot-separated, no unit or
  `_total` in the name, a plural for what is counted, the unit in UCUM. The seven of grpc-dotnet's
  EventCounters are among them, with their meaning:

  | Meter | Instrument | Kind, unit | grpc-dotnet's | Tags |
  |-------|------------|------------|---------------|------|
  | Calls | `armonik.client.calls.started` | counter, `{call}` | `total-calls` | |
  | Calls | `armonik.client.calls.active` | up-down counter, `{call}` | `current-calls` | |
  | Calls | `armonik.client.calls.failed` | counter, `{call}` | `calls-failed` | |
  | Calls | `armonik.client.calls.deadline_exceeded` | counter, `{call}` | `calls-deadline-exceeded` | |
  | Calls | `armonik.client.calls.unimplemented` | counter, `{call}` | `calls-unimplemented` | |
  | Calls | `armonik.client.messages.sent` | counter, `{message}` | `messages-sent` | |
  | Calls | `armonik.client.messages.received` | counter, `{message}` | `messages-received` | |
  | Calls | `armonik.client.calls.ended` | counter, `{call}` | | `rpc.response.status_code` |
  | Calls | `armonik.client.retries` | counter, `{retry}` | | `armonik.retry.origin`, `armonik.retry.reason` |
  | Calls | `armonik.client.calls.not_replayable` | counter, `{call}` | | |
  | Calls | `armonik.client.requests.resent` | counter, `{request}` | | |
  | Throttle | `armonik.client.throttle.retries_refused` | counter, `{retry}` | | |
  | Throttle | `armonik.client.throttle.cap` | gauge, `{call}/s` | | |
  | Throttle | `armonik.client.throttle.channels_capped` | gauge, `{channel}` | | |
  | Throttle | `armonik.client.throttle.channels_retries_closed` | gauge, `{channel}` | | |
  | Throttle | `armonik.client.throttle.calls_waiting` | gauge, `{call}` | | |
  | Connections | `armonik.client.dials` | counter, `{dial}` | | `armonik.dial.outcome` |
  | Connections | `armonik.client.dials.pending` | up-down counter, `{dial}` | | |
  | Connections | `armonik.client.connections.open` | up-down counter, `{connection}` | | |
  | Connections | `armonik.client.connections.closed` | counter, `{connection}` | | `armonik.connection.close_reason` |
  | Connections | `armonik.client.streams.reset` | counter, `{stream}` | | `http2.reset.reason` |
  | Connections | `armonik.client.streams.calls_waiting` | gauge, `{call}` | | |
  | Bytes | `armonik.client.wire.sent` | counter, `By` | | |
  | Bytes | `armonik.client.wire.received` | counter, `By` | | |
  | Bytes | `armonik.client.compression.input` | counter, `By` | | |
  | Bytes | `armonik.client.compression.output` | counter, `By` | | |
  | Bytes | `armonik.client.compression.gain` | gauge, `1` | | |
  | Host | `armonik.client.host.window.waits` | counter, `{wait}` | | |
  | Host | `armonik.client.host.memory.waits` | counter, `{wait}` | | |
  | Host | `armonik.client.host.memory.refusals` | counter, `{refusal}` | | |
  | Host | `armonik.client.logs.dropped` | counter, `{record}` | | |

  `armonik.dial.outcome` is `succeeded` or `failed`, and `rpc.response.status_code` a status's name, such as `UNAVAILABLE`. `armonik.retry.origin` is `status`, `http`,
  `reset`, `pushback`, `dial` or `connection`, and `armonik.retry.reason` the status name, the HTTP
  status or `other`, or the HTTP/2 error name, none for the others. The gauge `compression.gain` is
  absent while nothing was sent.
  The dropped logs are the binding's own, and not the engine's: the bounded queue of 16384 records
  it drops from and counts is the binding's, so the instrument exists when the runtime was given
  an `ILoggerFactory`, whichever build of the library is loaded.
- **Two builds of the library, selected before the first is loaded.** The package carries the
  library twice, one with the feature and one without, under the same file name in two folders of
  a runtime identifier: `runtimes/<rid>/native/` and `runtimes/<rid>/metrics/`. The second is
  outside the folders a package manager reads native assets from, which would make two files of
  one name collide, and the package's build file copies every `runtimes/*/metrics/` folder the
  package carries into the output and the publish directory of a project of any framework, as
  `runtimes/<rid>/metrics/`, and for .NET Framework into `<architecture>/metrics/`, beside the
  default library's. The selection names the folder from the operating system, the architecture and, on Linux, the C
  library alone - `win-x64`, `linux-musl-arm64` - as the package's folders are named, and not from
  `RuntimeInformation.RuntimeIdentifier`, which names the distribution a runtime was built for. The binding is
  told which to load before the first runtime is created, by `NativeLibrarySelection`, a public
  static class of its own, which is also what loads: the static constructor that loads the default
  library on .NET Framework asks the selection first, and never loads a second one beside it. A
  build the selection names and cannot find is an error that names the folders it looked in, and
  is not swallowed. On .NET 8 and later it installs a `DllImportResolver` for the library's
  name; on .NET Framework it loads the library by its full path before the first call, so that the
  `DllImport` that follows finds the module already loaded, by its base name, which the Windows
  loader does. The binding's `netstandard2.0` build, which a .NET Core host older than 8 runs,
  loads by path as .NET Framework does, and refuses the metrics build off Windows, where the
  loader does not match a loaded module by the name a `DllImport` gives. A library is loaded
  once for the life of the process, so asking for the other build after one is loaded is refused
  with an error that says that switching is not supported: the first library would have to be
  unloaded, and the binding does not unload.
- **`GrpcClient.NativeMetrics` selects it from ArmoniK.Api.Client.** A boolean, false by default,
  read when the process's native runtime is created, which the first client to need one does, and
  forwarded to `NativeLibrarySelection` when it is true. The clients that come after share that
  runtime, and their value is not read. False asks for nothing: it takes the library that is
  loaded, or the default one. A true that finds the other build already loaded - by an earlier
  explicit selection - fails the creation of the runtime with the refusal's error. It is the client's option and never an engine key: the engine is chosen
  before it is loaded, so it cannot read a key.

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
| `armonik_transport::configuration` | info | the configuration names a key the engine does not know, which is ignored | `source`, `key` |
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

## T10.3 settles

T10.3 settles the span record and how a span's start and end cross - one record at its end, or
one at each - how the spans built are configured and sampled; the trace callback's lifetime
follows the log callback's.
