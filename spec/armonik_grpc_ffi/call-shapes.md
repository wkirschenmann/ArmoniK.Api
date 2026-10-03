# Call shapes: what a call carries one of

**Status**: proposal, for decision (2026-10-03). Nothing here is built yet.

A gRPC method has one of four cardinalities, and each direction of a call carries either exactly
one message or a stream of them. The ABI treats every call as a bidirectional stream: the host
opens it, lends a buffer, sends, ends the sending, and receives each event in a callback of its
own. That is correct for every cardinality, and costs a unary call what a stream needs. This
document proposes that a call declare at its start which of its directions carries one message,
that the engine and the binding use it, and that the ABI carry more per crossing. The level-0 and
level-1 models keep their actions and guards; what changes in them is listed below, with the
argument that the new crossings refine them.

## What a unary call costs

Measured on the .NET 8 unary benchmark (loopback, TLS, one channel, 10 000 calls in a row), with
timestamps on both sides of the ABI on one clock, and allocations counted by stack over 20 calls
of the engine at `956b8811`:

| | per unary call |
|---|---|
| downcalls | seven: `ak_call_start`, `ak_get_call_buffer`, `ak_call_send_message`, `ak_call_end_send`, three `ak_event_consumed` |
| wake-ups of the channel's thread from the host | up to five: three spawns in `ak_call_start`, the send, the end of the sending |
| `ak_call_start` alone | a median of 17 to 41.5 us depending on the pass, broken down below |
| callbacks | four: WRITE_DONE, INITIAL_METADATA, MESSAGE, STATUS |
| thread-pool wake-ups on the .NET side | about three: from a callback's return to the code it wakes, a median of 11 to 27 us after the head, 3 to 6 us after the message and 12 to 17 us after the status, depending on the pass |
| tasks on the channel's thread | five, counted in the code: the driver, the FFI writer and reader, hyper's request task, hyper's body pipe |
| engine allocations | 80, 30 KB |
| copies of the sent message | one, into tonic's encoding buffer |

In one pass, where `ak_call_start` took 38.5 us at the median, the median of each of its steps:

| step | us |
|---|---:|
| crossing into the library | 0.3 |
| checks and decoding | 2.9 |
| joining the channel | 3.1 |
| the transport's start | 18.8 |
| - of which spawning the driver, which wakes the channel's thread | 9.9 |
| - of which creating the call's queues | 5.0 |
| - of which the path and metadata | 2.4 |
| - of which the rest: the closed check, the deadline, the return | 1.5 |
| creating the call's state | 4.4 |
| spawning the writer and the reader together | 6.7 |
| returning to the host | 0.7 |

A unary call has one request message, known before it is sent, and one response message, after
which only the trailers can follow. Every cost above that is not the network's comes from
treating those two singles as streams.

## Shapes

A call declares its shape in `ak_call_start_options.flags`:

| flag | meaning | cardinalities |
|---|---|---|
| `AK_CALL_ONE_REQUEST` | the request is exactly one message | unary, server streaming |
| `AK_CALL_ONE_RESPONSE` | the response is at most one message | unary, client streaming |

A call with neither flag is the bidirectional stream the ABI has, unchanged. The .NET binding sets
the flags from `Method.Type`. Proposed: a server that breaks a declared shape ends the call
`INTERNAL`, as the binding reports a unary call answered with more than one message.

## The request side: the commit sends the request

**ABI.**

- `ak_call_start` and `ak_get_call_buffer` are what they are today: the call starts at its
  creation, and its buffer is lent when the host knows the length.
- On a one-request call, `ak_call_send_message` also ends the sending. Proposed:
  `ak_call_end_send` is refused with `AK_STATUS_INVALID_STATE`, as on a call whose sending has
  ended, even before the commit - a one-request call with no request is not a call gRPC has, and
  a host that wants none cancels.
- Committing the empty buffer is the empty request, as it is today an empty message. The hazard
  the header states - the zeroed `*out` of a refused lend, committed, is that empty message -
  changes its outcome here: today a host that then sends its real message sends two requests on
  a unary call, which the server refuses; on a one-request call the empty message is the request,
  and the server runs it, so the mistake succeeds silently. The .NET binding never meets it: it
  throws on a refused lend before any commit, and commits the empty buffer only for a message of
  no bytes. A C host can. Decision 7.

**Engine.**

- `ak_call_start` registers the call and spawns nothing (decision 3, first branch). The commit
  spawns the call's one task with the request as its argument: the host wakes the channel's
  thread once for the whole request.
- A call that needs its task before the commit has it spawned then, without its request. That is
  a cancellation, a channel release or a shutdown, which then end the call through the paths they
  have today; and a lend refused for the budget, since the AK_EVENT_BUDGET_WAKE it waits for is
  raised by the task. Such a task takes its request later from a one-shot slot in the call's
  state - one hand-off, not a queue of commands - and sends nothing, not even the request's
  head, until the slot is filled.
- The buffer lent to a one-request call reserves five bytes ahead of what the host sees. The
  commit writes the gRPC prefix there, and the buffer becomes the request's whole body: one
  frame. hyper still spawns its body pipe for it unless a first, eager poll of the body
  completes, which needs send capacity the stream may not have yet; whether the pipe task goes,
  and whether HEADERS and DATA share one socket write as they do today, is measured at step 4.
- tonic's client still builds the request - the path, `content-type`, `te`, `grpc-timeout`, its
  sanitized headers, the origin's scheme and authority - and still reads the response head,
  Trailers-Only included. The call gives it an empty stream of messages, and the engine's own
  HTTP/2 service, which every request already passes through, replaces the body tonic encoded
  with the framed buffer. The message is not copied, no encoding buffer is allocated, and the
  replay keeps the same framed buffer.
- An empty request has no buffer to frame in place: its body is the bare five-byte prefix.
- No writer task, no command queue, no request channel: the request is the task's argument, or
  its slot's.
- `armonik-transport` gains the type of a message framed in place - an allocation with the prefix
  ahead of the payload - which the FFI lends from and `CallStartOptions` carries. A Rust caller
  with a plain `Bytes` has it framed by a copy.

**Deadline.** The deadline is an instant fixed at `ak_call_start`, so `grpc-timeout` carries what
is left of it when the request leaves, as it does today. A call spawned before its commit has its
task, which ends the call at its deadline as today, and a later commit is refused. A call not
spawned has nothing to end it at its deadline. One answer, decision 3's first branch: its commit
checks the deadline and, past it, is accepted, spawns the task, and ends `DEADLINE_EXCEEDED`
without sending: WRITE_DONE for the abandoned message, then the status. A call never committed ends
at its cancellation, release or shutdown.

What that weakens, for a host that holds an uncommitted call past its deadline: the status comes
at the commit, the lent buffer stays charged until then, requirement 2.5 holds only from the
commit, and a `timeout_ns` of zero still never reaches the server but ends the call only at the
commit or the cancellation. The two cases also answer a late commit differently - refused when
the call was spawned early, as after a budget refusal, accepted then ended when it was not.
The binding commits right after its serializer returns, so for it the window is a serialization's.
Deadlines are not modelled at levels 0 and 1. The alternative is decision 3.

**The .NET binding** keeps its sequence: start at the call's creation, lend at the serializer's
announcement, commit. It sets the flags and no longer ends the sending.

**Level 1.** The same transitions, the last two grouped in one downcall:

| downcall | today | proposed |
|---|---|---|
| `ak_call_start` | `CallStart` | `CallStart`: the deferred spawn is not a step |
| `ak_get_call_buffer` | `LendSendBuffer` or a refusal | unchanged |
| `ak_call_send_message` | `SendMessage` | `SendMessage`, then `EndSend` |
| `ak_call_end_send` | `EndSend` | refused on a one-request call |

`SendMessage` and `EndSend` both need the sending open and no status pending (`~status_pending`).
Before its commit a call sends nothing - no task, or a task holding back even the request's head
- so no answer can arrive; a status the engine raises itself ends the call, and the commit is
then refused, with the one exception the deadline paragraph gives (decision 3, first branch).
Otherwise
`SendMessage` and `EndSend` linearize together at the commit, on the call's state, which no
other step of the call holds between them. The empty request is outside the model, as it is
today: an empty send takes no memory, and level 1 does not represent it.

## The response side: what arrives together is delivered together

**ABI.**

- The callback takes the events of one call as an array, in delivery order:
  `void (*ak_callback)(void *runtime_ctx, ak_call_ctx call_ctx, const ak_event *events, size_t count)`,
  `count` at least 1. A runtime event is an array of one. The array and its `ak_event`s are valid
  for the callback's duration only; the payloads they own are the host's until given back, as
  today.
- `ak_events_consumed(const ak_bytes *payloads, size_t count)` gives back several payloads in one
  downcall, in delivery order, each as `ak_event_consumed` gives back one: an unowned payload,
  owner NULL, is a no-op there too, and a zero-length payload with an owner is given back, its
  credit with it.

**Engine.**

- The call's one task reads the response and delivers it itself: no reader task, no queue between
  the driver and the actor, no hand-off of the read admission between them.
- When an event is ready, the task looks once more, without waiting, at what is already there -
  a message already in hand, trailers already received - and delivers everything it found in one
  callback. Every read of that look, the trailers' included, goes through `ReadGate` as today.
  The turn it waits for opens when the previous message's delivery step is taken, which is as
  soon as that message is in hand (see level 1 below), not when a callback returns as today:
  `ReadTurn::delivered()` moves from after the callback to the delivery step. The ledger's
  admission against the runtime's threshold applies to each message as it does to every read,
  and a batch is also bounded by the delivery window. A read the gate admitted is never dropped:
  if what it waits for has not arrived, the batch goes without it and the same read goes on after
  the callback, admitted - so a read is admitted, and the threshold checked, before the callback
  that today comes first. It never waits to make a batch:
  a head that arrives alone, from a server that sends its headers early and computes the answer
  after, is delivered alone and at once.
- On a one-response call, what follows the message is looked at without being decoded. tonic's
  `Streaming` decodes whatever DATA it reads, so the look is below it, in the engine's own
  response body, which already tracks the gRPC framing of what it passes up: once one whole
  message has passed, further message bytes become the body's error, `INTERNAL`, before tonic
  sees them, and they are neither decoded nor charged to the runtime's count. The check is on
  bytes, not frames: an empty DATA frame is still nothing, as the body already holds, and a
  frame that ends the message and begins another is split, its first part passed up. The body
  counts messages for this, which it does not today. Trailers pass up as the status. The read
  admission of the message itself, against the runtime's memory threshold, is unchanged.
- WRITE_DONE joins events ready when it is, at the head of their batch, and never waits for any:
  one that is ready alone goes alone, at once. Several ready together - a host with more than one
  send in flight - head the batch in send order, each its own `EmitWriteDone` and
  `WriteDoneReturns`, each acquitted by the binding in turn. Whatever batch carries it, the binding
  acquits it in the callback, outside the ring, as today, so it never waits behind the host's
  handling of a message - the promise the header and `architecture.md` make. BUDGET_WAKE always
  comes alone.
- Trailers found while a WRITE_DONE is still owed wait for it: the terminal needs every send
  acquitted (`HasNoSendInFlight`), so the WRITE_DONE is taken first, at the head of the same
  batch.
- A unary call's WRITE_DONE is ready when its request is handed to the connection, a round trip
  before its response, so it usually comes alone: a unary call takes two callbacks, WRITE_DONE
  and then INITIAL_METADATA, MESSAGE and STATUS together, and one `ak_events_consumed`. One
  callback would need the WRITE_DONE of a one-request call held for the response's batch, which
  frees nothing the host could use - the call sends nothing more - but holds back an acquittal
  the ABI promises as soon as the send settles; that is decision 5.

**Level 1.** A batch is the sequence of level-1 steps its events are today, mapped so that every
guard holds and the delivery flag is conservative - TRUE whenever a data callback is on the
host's stack, and at times when none is:

- The engine builds the batch event by event, under each event's guard, before it calls the host.
  An event's delivery step - `DeliverMessage`, `EmitWriteDone` - is taken as soon as the event is
  in hand, before the engine looks for the next one. Its callback's return -
  `DeliveryCallbackReturns`, `WriteDoneReturns` - is taken just before the next event's delivery
  step, if one follows: the host has not been called, so no callback is on its stack, which is
  what the returned flag says, and the next event's `HasFreeDeliverySlot` holds.
- The last event's return is taken when the real callback returns. A batch with a data event ends
  with one, the terminal included, and a WRITE_DONE alone is the write-done callback it is today.
  So while the host runs a data batch, the call's `delivery_callback_running` is TRUE, and every
  guarantee tied to a callback's duration - `ReleaseCallHandle` waiting for it, the terminal
  being the call's last callback, level 2's roots surviving their callbacks - covers the whole
  real callback. The flag is also TRUE while the engine looks for the next event, which level 1
  allows: neither `AdmitRead` nor `NetworkReceive` nor `ReceiveStatus` has a guard on it.
- A WRITE_DONE at the head of a data batch has its `WriteDoneReturns` taken before the host is
  called, so `write_done_callback_running` is FALSE while it is on the host's stack. What the
  write-done flag guards is then carried by the delivery flag, TRUE for the same callback:
  `IsRuntimeDrained` reads both, and `ReleaseCallHandle` reads only the delivery flag.
- What the host does inside the callback - giving back a payload, lending - is a step after the
  delivery of everything in the batch, which level 1 already allows.
- `ak_events_consumed` of `n` payloads is `n` `HostConsumesEvent` steps, in order.
- Every message of a batch is read after `AdmitRead`, whose guard - every received message
  delivered - holds because each message's delivery step is taken as soon as it is in hand,
  before the look for what follows. Trailers are `ReceiveStatus`, and
  on a one-response call, message bytes refused after the message are the engine's own
  `INTERNAL` status, that same `ReceiveStatus`. No message is received that `AdmitRead` did not
  admit.

So no action, guard or variable changes at levels 0 and 1. Their prose does: the meaning of the
two flags in `formal-model.md`'s list of variables, "a callback is on the host stack", which
becomes conservative for the delivery flag and carried by it for the write-done one; and wherever
it says one event per callback - `formal-model.md`'s paragraph on the `Deliver` and `Emit`
actions ("an `ak_callback` carries one `ak_event`"), and the comment on `DeliverCancelled` in
`FfiGrpc.tla`,
whose rule - the initial metadata out before a cancellation settles the call - stays, as a
property of the model rather than of the callback.

## The binding (level 2)

- `OnEvent` hands a WRITE_DONE heading a batch to the sender, which it acquits without the ring,
  as today, and copies the batch's data events into the call's ring - the array does not outlive
  the callback - signalling its reader once per batch.
- A one-response call's reader is woken once the terminal is in the ring, the delivery window is
  full, or the call is cancelled, and takes head, message and status in one pass with one
  `ak_events_consumed`. Without the window's condition, a call whose window is one credit would
  wait for a terminal the engine holds until the head is given back.
- Proposed: the task that answers `ResponseHeadersAsync` starts when a caller asks for the
  headers, not on every call. Once started, it waits on the ring's arrival as it does today, so
  every batch wakes it, and a head that arrives alone and early answers it at once. The head is
  then consumed by whichever of it and the reader comes first, the phase arbitrating as it does
  today. A head nobody asks for is consumed by the reader in its pass. No message takes a side
  path: the reader parses every one.
- A one-request send commits its buffer, which ends the sending, and no longer calls
  `ak_call_end_send`: one wake of the channel's thread for the request.

`DotNetBinding.tla` describes the binding as built, so it follows. Writing its new steps is step
5's work; what they must achieve is set here, with the traps that lie on the way, and this
document does not choose their guards.

What the steps must achieve:

- Every level-2 step still refines one level-1 step, as `RefinesNext` states it.
- A call has a shape, chosen at `StartCall`.
- A one-request commit refines `SendMessage` then `EndSend`, with nothing of that call between
  the two, as the engine linearizes it: `EndSend`'s guard - the sending open, no status pending,
  the call active - holds from the first to the second, and the second always comes, a dispose
  included. The writer gains a state between the two and one after, a write in flight on a closed
  sending.
- A batch refines the level-1 sequence of the level-1 section above: its returns before the last
  are the engine's, the last is the binding's - `OnEventReturns`, or `TerminalCallbackReturns`,
  which frees the call's root - and the model tells them apart, with the engine's new steps fair
  as the runtime's are (`RuntimeOwedFairness`).
- A WRITE_DONE at the head of a batch is acquitted by the binding inside that batch's callback,
  before its last return, refining a level-1 stutter, since the engine took its return; a lone
  WRITE_DONE keeps today's steps.
- `ak_events_consumed` is `n` binding steps, one per payload, each refining one
  `HostConsumesEvent`.
- `BeginParseEvent`, the step from waiting to parsing, gains the one-response wake condition.
- `CloseWriter` disabled on a one-request call, whose commit ends its sending.
- If decision 9 keeps it, the headers task on demand, which changes `ConsumeHeader`, the prologue phase, `PastPrologueHeadersAnswered` and the
  liveness of `HeadersEventuallyResolved` - a head answers a headers request whenever one is made,
  before or after the reader consumed it.
- Restated: `AwaitingWriteDoneHasOneComing`, with a disjunct for a batch-head WRITE_DONE emitted
  and returned that waits for its acquittal, over the three writer states that await a WRITE_DONE
  - `awaiting_write_done`, the state between the commit's two steps, and the write in flight on a
  closed sending; and `PendingWriteEventuallySettled`, whose writer states gain the last two.

Traps:

- `EndSend`'s guard falls to any step of the call that sets `status_pending` -
  `ReceiveStatus`, and `EndCallPastHardCeiling` too, which an admitted read enables at any time -
  and to any that ends the call; those are what the state between the commit's two steps must
  hold off. A write settles only by `EmitWriteDone` and `WriteDoneReturns`; `NetworkSend` touches
  neither the guard nor the settling.
- `CloseWriter` needs an idle writer, which `CommitWrite` does not leave: the closing step of a
  one-request call is a new one.
- `DeliveryCallbackReturns` is enabled whenever the delivery flag is TRUE, so without a batch
  state the engine's return could be taken as the last one, skipping the acquittal and the root's
  release.
- A WRITE_DONE marked as heading a batch must be one a delivery follows, and the mark cleared at
  its acquittal, or a later write of a streaming call is taken for one.
- `WriteDoneCompletes` then has two forms, today's refining `WriteDoneReturns` and the
  batch-head one refining a stutter.

The refinement is to prove again once those steps are written; level 1's own theorems are
untouched.

## What else changes

- `abi.rs`, and so the header and `NativeMethods.g.cs`: the callback's type and documentation,
  WRITE_DONE's place in a batch, `ak_events_consumed`, `AK_CALL_ONE_REQUEST` and
  `AK_CALL_ONE_RESPONSE`, and what `ak_call_send_message` and `ak_call_end_send` do on a
  one-request call.
- `abi.md` and `contract.md`, wherever a callback carries one event, and `architecture.md`, whose
  sketches are normative: the trampoline, the ring's publication and `ak_event_consumed`.
- The hand-kept layout checks - `tests/layout.rs`, `AbiLayoutTests.cs` and the test host in
  `tests/support/host.rs` - for the callback.
- `formal-model.md`'s mapping table: the rows of `SendMessage` and `EndSend` name their new
  linearization point, `ak_events_consumed` gets a row - `n` `HostConsumesEvent` steps - and
  `tla/ci/check_abi_coverage.py` checks that every ABI entry has one.
- Rules that a declared cardinality reverses: `contract.md`'s note that "the cardinality is not
  declared at start", and `decisions.md`'s row on the replay ceiling, whose ceiling "needs no
  cardinality declared at start" - it still needs none, but calls may declare one.
  `architecture.md`'s "Unary is not a special case" holds: the response's message still travels
  the ring and is parsed by the reader, woken once.
- The WRITE_DONE promise: the header's "may arrive in parallel with any of them" stays a
  permission the host is built for, and `architecture.md`'s "WRITE_DONE must never queue behind a
  slow message handler" stays true of the binding, which acquits it outside the ring; both gain
  the batch WRITE_DONE may head.
- If decision 3 keeps the deadline checked at the commit: requirement 2.5 and its status, and the
  promise on `timeout_ns` of zero in `abi.rs` (so the header and `NativeMethods.g.cs`) and in
  `abi.md`.
- The engine's structure that step 1 replaces - a driver, an actor with a writer and a reader -
  wherever `architecture.md` and `formal-model.md`'s mapping table describe it.
- `decisions.md`: the row "How much of tonic the engine reuses" (decided 2026-09-28) has the
  engine run every call through tonic's client and accept its encoder's copy. This proposal
  amends only the copy: a one-request call still goes through tonic's client, which writes its
  request head and reads its response, and its body is swapped below tonic for the framed
  buffer.

## What it should buy

Per unary call, by construction, with decision 3's first branch - a call with a deadline spawns
nothing at its start; with the second, the wake-ups from the host are two. The times are the
measured costs of what goes, and the whole is measured after each step rather than added up:

| | today | proposed |
|---|---|---|
| downcalls | 7 | 4: the start, the lend, the send, one consume |
| wake-ups of the channel's thread from the host | up to 5 | 1 |
| callbacks | 4 | 2: WRITE_DONE, then the response together; 1 with decision 5 |
| .NET pool wake-ups | about 3 | 1 with the headers task on demand (decision 9) and no caller asking for headers - 2 otherwise; the WRITE_DONE of a unary call wakes nothing on the .NET side |
| tasks on the channel's thread | 5 | 2 or 3: the call's, hyper's request task, and its body pipe unless step 4's measure shows it gone |
| copies of the sent message | 1 | 0 |
| engine allocations | 80 | counted after each step; the command queue, the request and response channels, the writer's and reader's task cells and tonic's encoding buffer go |

## Decisions to take

1. **The shapes themselves, and the version.** The callback changes type: the ABI changes rather
   than grows, and a host built against the old header would read a batch as one event. T4.0
   keeps `AK_ABI_VERSION` at 1 while the ABI is not published, and the crate is
   `publish = false`; whether this change still fits that rule, or bumps the version, is to
   decide.

2. **The window and one-response calls.** With one credit, head and message cannot share a batch,
   and the response takes two callbacks: correct, only slower. Exempting one-response calls from
   the window would bound their payloads by their shape instead, at the cost of a per-shape bound
   in level 1 and a larger ring. Proposed: keep the window, whose default of four makes the case
   rare.

3. **The deadline of a call not yet committed**: checked at the commit, as in its paragraph; or
   a task spawned at the start of every call with a deadline, which keeps today's promises.
   Nearly every ArmoniK call has a deadline, so the second branch spends on almost every call the
   wake-up the first saves. Recommended: the first, with requirement 2.5 restated for a
   one-request call as holding from its commit; the .NET binding commits right after its
   serializer returns, and a C host that holds its buffer is the one that sees the difference.

4. **Where the request body is swapped.** Proposed: below tonic, in the engine's HTTP/2 service,
   tonic's client keeping the request head and the response. The alternative - the engine
   building the request itself - takes over everything tonic's `prepare_request` and its private
   `create_response` do, Trailers-Only and the `grpc-encoding` check included. Whether the
   response leaves tonic too, to slice a message out of its DATA frame without a copy, is a
   later decision.

5. **Holding a one-request call's WRITE_DONE** for its response's batch: one callback instead of
   two, against an acquittal the ABI promises as soon as the send settles. Proposed: not held.

6. **A body of known length.** hyper may send `content-length` for it. HTTP/2 allows it and gRPC
   servers accept it; checked on the test server and on Kestrel before step 4 lands.

7. **The empty request of a one-request call**, which a C host's mistaken commit turns into a
   silent success, as above: accepted; or made explicit at the commit, where the host knows the
   length - an entry or a flag of `ak_call_send_message` that sends it, and the zeroed buffer
   refused on a one-request call.

8. **Lending the request's buffer with `ak_call_start`.** It saves one downcall - a crossing and a
   lookup in the handle table - and moves the native start into the serializer's announcement,
   where a call cancelled, asked for its headers or disposed before the announcement, and an
   argument refused at the start, would all need a new answer. Recommended: not done, which is
   what this document assumes.

9. **The smaller choices marked "proposed" above**: `INTERNAL` for a broken shape;
   `ak_call_end_send` refused on a one-request call before its commit; and the headers task
   started on demand, which changes the level-2 liveness of `HeadersEventuallyResolved`.

## Steps

Each step is measured, in alternating passes, before the next starts.

1. **One task per call**, with no ABI change: the driver, the writer and the reader become one
   task, joined rather than spawned.
2. **Batched delivery**: the array callback and `ak_events_consumed`; the binding copies,
   publishes and signals per batch.
3. **One response**: what follows the message looked at in the engine's response body; the
   binding's single-pass reader and on-demand headers.
4. **One request**: the commit that ends the sending and spawns the task (as decision 3
   settles), the one-shot slot for a
   call spawned early, the framed body swapped below tonic, the replay of the same buffer; and
   the measure of whether hyper's body pipe task goes and of the socket writes per request.
5. **The models' prose and level 2**: the one-event wording at level 1, `DotNetBinding.tla`
   following the binding with its refinement proved again, the mapping table, and the TLC
   instantiability check.
