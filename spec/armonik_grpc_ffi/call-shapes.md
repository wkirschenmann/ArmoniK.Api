# Call shapes: what a call carries one of

**Status**: decided 2026-10-03 and 2026-10-04; built, steps 1 to 5.

A gRPC method has one of four cardinalities, and each direction of a call carries either exactly
one message or a stream of them. The ABI treats every call as a bidirectional stream: the host
opens it, lends a buffer, sends, ends the sending, and receives each event in a callback of its
own. That is correct for every cardinality, and costs a unary call what a stream needs. Here a call
declares at its start which of its directions carries one message, the engine and the binding use
it, and the ABI carries more per crossing. The level-0 and level-1 models keep their actions and
guards; what changes in them is listed below, with the argument that the new crossings refine
them. The design optimizes the path a call takes when it goes well; a call that goes wrong may
take more callbacks than that path does.

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
the flags from `Method.Type`. A server that breaks a declared shape ends the call `INTERNAL`, as
the binding reports a unary call answered with more than one message.

## The request side: the commit sends the request

**ABI.**

- `ak_call_start` and `ak_get_call_buffer` are what they are today: the call starts at its
  creation, and its buffer is lent when the host knows the length.
- On a one-request call, `ak_call_send_message` also ends the sending, and `ak_call_end_send` is
  refused with `AK_STATUS_INVALID_STATE`, as on a call whose sending has ended, even before the
  commit: a one-request call with no request is not a call gRPC has, and a host that wants none
  cancels.
- A one-request call has no AK_EVENT_WRITE_DONE (decisions 5 and 10): the host commits its only
  buffer, gives it up at the commit, and sends nothing more, so an acquittal would tell it nothing.
  The engine still settles the send for itself - the window's slot, the bytes released from the
  runtime's count - when the send settles, written or abandoned, before the call's terminal.
- After the commit, `ak_get_call_buffer` on a one-request call answers `AK_STATUS_INVALID_STATE`,
  as after the end of the sending: `AK_STATUS_SLOT_BUSY`, whose wake-up is a WRITE_DONE, would
  promise one that never comes.
- Committing the empty buffer is the empty request, as it is today an empty message. The hazard the
  header states - the zeroed `*out` of a refused lend, committed, is that empty message - changes
  its outcome here: today a host that then sends its real message sends two requests on a unary
  call, which the server refuses; on a one-request call the empty message is the request, the
  server runs it, and the host learns of its mistake only when its real send is refused. That is
  accepted: the .NET binding never meets it - it throws on a refused lend before any commit, and
  commits the empty buffer only for a message of no bytes - and a C host is told so by the header.

**Engine.**

- On a one-request call, `ak_call_start` registers the call and spawns nothing. The commit spawns
  the call's one task with the request as its argument: the host wakes the channel's thread once
  for the whole request. Any other call has its one task spawned at `ak_call_start`, as today.
- A call that needs its task before the commit has it spawned then, without its request. That is
  a cancellation, a channel release or a shutdown, which then end the call through the paths they
  have today; and a lend refused for the budget, since the AK_EVENT_BUDGET_WAKE it waits for is
  raised by the task. Such a task takes its request later from a one-shot slot in the call's
  state - one hand-off, not a queue of commands - and sends nothing, not even the request's
  head, until the slot is filled. One transition of the call's state decides which of a commit
  and those paths spawns the task. If one of the paths wins, the commit fills the slot, or is
  refused once the call has ended; if the commit wins, the path finds the task spawned and
  reaches it as it does today.
- The buffer lent to a one-request call reserves five bytes ahead of what the host sees. The
  commit writes the gRPC prefix there, and the buffer becomes the request's whole body: one
  frame. An empty request has no buffer to frame in place: its body is the bare five-byte prefix.
- tonic's client still builds the request - the path, `content-type`, `te`, `grpc-timeout`, its
  sanitized headers, the origin's scheme and authority - and still reads the response head,
  Trailers-Only included. The call gives it an empty stream of messages, and the engine's own
  HTTP/2 service, which every request already passes through, replaces the body tonic encoded
  with the framed buffer. The message is not copied, no encoding buffer is allocated, and the
  replay keeps the same framed buffer.
- hyper still spawns its body pipe for that body unless a first, eager poll of it completes,
  which needs send capacity the stream may not have yet. Whether the pipe task goes, and whether
  HEADERS and DATA share one socket write as they do today, is measured at step 4, as is whether
  hyper sends a `content-length` for a body of known length and whether the test server and
  Kestrel accept it - HTTP/2 allows it.
- No writer task, no command queue, no request channel: the request is the task's argument, or
  its slot's.
- `armonik-transport` gains the type of a message framed in place - an allocation with the prefix
  ahead of the payload - which the FFI lends from and `CallStartOptions` carries. A Rust caller
  with a plain `Bytes` has it framed by a copy.

**Deadline.** The deadline is an instant fixed at `ak_call_start`, so `grpc-timeout` carries what
is left of it when the request leaves, as it does today. A call spawned before its commit has its
task, which ends the call at its deadline as today, and a later commit is refused. A call not
spawned has nothing watching its deadline: its commit checks it and, past it, is accepted, spawns
the task, and ends the call `DEADLINE_EXCEEDED` without sending. A call never committed ends at
its cancellation, release or shutdown. So a late commit is refused on a call spawned early, as
after a budget refusal, and accepted then ended on one that was not: a host handles both, as it
handles today a send racing its call's deadline.

That serves the call that goes well, which is almost every call and has a deadline that never
fires, at the cost of the one that does not, for a host that holds an uncommitted call past its
deadline: the status comes at the commit, the lent buffer stays charged until then, and a
`timeout_ns` of zero still never reaches the server but ends the call only at the commit or the
cancellation. Requirement 2.5 holds, for a one-request call, from its commit. The binding commits
right after its serializer returns, so for it the window is a serialization's. Deadlines are not
modelled at levels 0 and 1.

**The .NET binding** keeps its sequence: start at the call's creation, lend at the serializer's
announcement, commit. It sets the flags, no longer ends the sending, and no longer waits for an
acquittal on a one-request call.

**Level 1.** The same transitions; the request's two are grouped in one downcall, and its
acquittal is the engine's:

| downcall | today | one-request call |
|---|---|---|
| `ak_call_start` | `CallStart` | `CallStart`: the deferred spawn is not a step |
| `ak_get_call_buffer` | `LendSendBuffer` or a refusal | unchanged |
| `ak_call_send_message` | `SendMessage` | `SendMessage`, then `EndSend` |
| `ak_call_end_send` | `EndSend` | refused |
| the WRITE_DONE callback | `EmitWriteDone`, then `WriteDoneReturns` | the same two steps, taken by the engine when the send settles, with no callback |

`SendMessage` and `EndSend` both need the sending open and no status pending (`~status_pending`).
Before its commit a call sends nothing - no task, or a task holding back even the request's head
- so no answer can arrive; a status the engine raises itself ends the call, and the commit is
then refused, except a deadline past on a call not yet spawned, as the deadline paragraph says.
Otherwise `SendMessage` and `EndSend` linearize together at the commit, on the call's state, which
no other step of the call holds between them. The engine takes `EmitWriteDone` and
`WriteDoneReturns` before the call's terminal, which needs them (`HasNoSendInFlight`). The empty
request is outside the model, as it is today: an empty send takes no memory, and level 1 does not
represent it.

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
- A batch carries data events only - INITIAL_METADATA, MESSAGE, STATUS (decision 11).
  AK_EVENT_WRITE_DONE, on a call whose sends cross the ABI, and AK_EVENT_BUDGET_WAKE come alone, as
  today.

**Engine.**

- The call's one task reads the response and delivers it itself: no reader task, no queue between
  the driver and the actor, no hand-off of the read admission between them.
- When a data event is ready, the task looks once more at what is already there - a message in
  hand, trailers received - and delivers all of it in one callback. What is not there yet is
  waited for one round of the channel's thread while the batch holds less than
  `DeliveryCoalescingBytes`: the connection shares that thread, and decodes on its next turn what
  the same read of the socket brought. A head goes alone only when nothing follows it within
  that round, as from a server that sends its headers early.
- Every read of that look, the status's included but for the one below, goes through `ReadGate` as
  today: the gate's turn, then the ledger's admission against the runtime's threshold. The look
  waits no longer than that round: a read not admitted, or admitted with nothing to read after
  it, ends the batch and goes on after the callback. A batch is also bounded by the delivery window. The turn
  opens at the previous message's delivery step, taken as soon as the message is in hand (level 1
  below), so `ReadTurn::delivered()` moves there from after the callback.
- On a one-response call, the read after the message takes the gate's turn, which the message's
  delivery step opens, and not the ledger's admission (decision 12): it can only yield the
  trailers or the refusal of further bytes below, and is charged nothing. Behind the admission it
  would wait, once the message takes the runtime's count to its limit, for the message to be given
  back, which the binding's reader does only once the status is in. `ReadGate` gains that second
  entry, the turn alone. Level 1 receives every status ungated, so this read is within it.
- On a call whose sends cross the ABI, a status found while a WRITE_DONE is owed waits for it, as
  `DeliverStatus` needs every send acquitted: the WRITE_DONE comes alone, and the status follows
  with what is staged.
- On a one-response call, the engine's own response body, below tonic, turns any byte of a second
  message into `INTERNAL` before tonic decodes it or the runtime is charged for it: tonic's
  `Streaming` decodes whatever DATA it reads, and the body already tracks the gRPC framing. The
  check is on bytes, not frames: a frame that ends the message and begins another is split, its
  first part passed up. The body counts messages for this, which it does not today.
- A unary call's typical delivery is then one callback carrying INITIAL_METADATA, MESSAGE and
  STATUS, and one `ak_events_consumed`.

**Level 1.** A batch is the sequence of level-1 steps its events are today, mapped so that every
guard holds and the delivery flag is conservative - TRUE whenever a data callback is on the
host's stack, and at times when none is:

- The engine builds the batch event by event, under each event's guard, before it calls the host.
  An event's delivery step - `DeliverInitialMetadata`, `DeliverMessage`, `DeliverStatus` - is
  taken as soon as the event is in hand, before the engine looks for the next one. Its
  `DeliveryCallbackReturns` is taken just before the next event's delivery step, if one follows:
  the host has not been called, so no callback is on its stack, which is what the returned flag
  says, and the next event's `HasFreeDeliverySlot` holds.
- The last event's return is taken when the real callback returns. So while the host runs a batch,
  the call's `delivery_callback_running` is TRUE, and every guarantee tied to a callback's
  duration - `ReleaseCallHandle` waiting for it, the terminal being the call's last callback,
  level 2's roots surviving their callbacks - covers the whole real callback. The flag is also
  TRUE while the engine looks for the next event, which level 1 allows: neither `AdmitRead` nor
  `NetworkReceive` nor `ReceiveStatus` has a guard on it.
- On a one-request call, `write_done_callback_running` is TRUE between the engine's two steps with
  no callback on the host's stack: conservative too.
- What the host does inside the callback - giving back a payload, lending - is a step after the
  delivery of everything in the batch, which level 1 already allows.
- `ak_events_consumed` of `n` payloads is `n` `HostConsumesEvent` steps, in order.
- Every message of a batch is read after `AdmitRead`, whose guard - every received message
  delivered - holds because each message's delivery step is taken as soon as it is in hand,
  before the look for what follows. Trailers are `ReceiveStatus`, and on a one-response call,
  message bytes refused after the message are the engine's own `INTERNAL` status, that same
  `ReceiveStatus`. No message is received that `AdmitRead` did not admit.

So no action, guard or variable changes at levels 0 and 1. Their prose does: the meaning of the two
flags in `formal-model.md`'s list of variables, "a callback is on the host stack", which becomes
conservative - the delivery flag's with batches, the write-done flag's with one-request calls; and
wherever it says one event per callback - `formal-model.md`'s paragraph on the `Deliver` and `Emit`
actions ("an `ak_callback` carries one `ak_event`"), and the comment on `DeliverCancelled` in
`FfiGrpc.tla`, whose rule - the initial metadata out before a cancellation settles the call -
stays, as a property of the model rather than of the callback.

## The binding (level 2)

- `OnEvent` copies a batch's events into the call's ring - the array does not outlive the
  callback - and signals its reader once per batch. A WRITE_DONE, alone, goes to the sender as
  today.
- A one-response call's reader is woken once the terminal is in the ring, the delivery window is
  full, or the call is cancelled, and takes head, message and status in one pass with one
  `ak_events_consumed`. The terminal always comes, the engine reading the status past the
  ledger's admission (decision 12). Without the window's condition, a call whose window is one
  credit would wait for a message the engine holds until the head is given back.
- The task that answers `ResponseHeadersAsync` starts when a caller asks for the headers, not on
  every call. Once started, it waits on the ring's arrival as it does today, so every batch wakes
  it, and a head that arrives alone and early answers it at once. The head is then consumed by
  whichever of it and the reader comes first, the phase arbitrating as it does today. A head
  nobody asks for is consumed by the reader in its pass. No message takes a side path: the
  reader parses every one.
- A one-request send commits its buffer, which ends the sending: one wake of the channel's thread
  for the request, and no acquittal to wait for.

`DotNetBinding.tla` describes the binding as built, so it follows. What its new steps were to
achieve was set here before they were written, with the traps that lay on the way; step 5 says
where the model departs from it.

What the steps must achieve:

- Every level-2 step still refines one level-1 step, as `RefinesNext` states it.
- A call has a shape, chosen at `StartCall`.
- A one-request commit refines `SendMessage` then `EndSend`, with nothing of that call between
  the two, as the engine linearizes it: `EndSend`'s guard - the sending open, no status pending,
  the call active - holds from the first to the second, and the second always comes, a dispose
  included. The writer gains a state between the two, and then is closed: no acquittal follows.
  `CloseWriter` is disabled on a one-request call.
- On a one-request call, `EmitWriteDone` and `WriteDoneReturns` are runtime steps with no binding
  counterpart: `WriteDoneCompletes` is not enabled for it, and both are weakly fair as runtime
  steps, `WriteDoneReturns` included - today only the binding's `WriteDoneCompletes` makes it
  fair, and without it the call's terminal would wait on `HasNoSendInFlight` for good.
- A batch refines the level-1 sequence of the level-1 section above: its returns before the last
  are the engine's, the last is the binding's - `OnEventReturns`, or `TerminalCallbackReturns`,
  which frees the call's root - and the model tells them apart, with the engine's new steps fair
  as the runtime's are (`RuntimeOwedFairness`).
- `ak_events_consumed` is `n` binding steps, one per payload, each `FinishConsumePayload`
  refining one `HostConsumesEvent`.
- `BeginParseEvent`, the step from waiting to parsing, gains the one-response wake condition.
  Level 1 receives a status ungated, so the model cannot show the wait decision 12 removes - a
  status held behind a message the reader gives back only once the status is in: step 3 tests
  it, with a message that takes the runtime's count to its limit.
- The headers task on demand changes `ConsumeHeader`, the prologue phase,
  `PastPrologueHeadersAnswered` and the liveness of `HeadersEventuallyResolved`: a head answers a
  headers request whenever one is made, before or after the reader consumed it.
- `PendingWriteEventuallySettled` gains the state between the commit's two steps.
  `AwaitingWriteDoneHasOneComing` is unchanged: a one-request writer never awaits a WRITE_DONE.

Traps:

- `EndSend`'s guard falls to any step of the call that sets `status_pending` -
  `ReceiveStatus`, and `EndCallPastHardCeiling` too, which an admitted read enables at any time -
  and to any that ends the call; those are what the state between the commit's two steps must
  hold off. A write settles only by `EmitWriteDone` and `WriteDoneReturns`; `NetworkSend` touches
  neither the guard nor the settling.
- `CloseWriter` needs an idle writer, which `CommitWrite` does not leave: the closing step of a
  one-request call is a new one.
- `DeliveryCallbackReturns` is enabled whenever the delivery flag is TRUE, so without a batch
  state the engine's return could be taken as the last one, skipping the root's release.
- The binding's `WriteDoneCompletes` conjoins `L1!WriteDoneReturns` today; left enabled on a
  one-request call, it would wait for a return the engine has already taken.

Level 1's own theorems are untouched.

## What else changes

- `abi.rs`, and so the header and `NativeMethods.g.cs`: the callback's type and documentation,
  that a batch carries data events only, `ak_events_consumed`, `AK_CALL_ONE_REQUEST` and
  `AK_CALL_ONE_RESPONSE`, what `ak_call_send_message` and `ak_call_end_send` do on a one-request
  call, and WRITE_DONE's promise - "exactly once per accepted send" - which no longer holds on a
  one-request call, where there is none; and the `AK_STATUS_SLOT_BUSY` wake-up on the next
  WRITE_DONE, which a one-request call answers with `AK_STATUS_INVALID_STATE` after its commit
  instead. They are stated in `lib.rs`'s `ak_get_call_buffer` and `ak_call_send_message` (so the
  header and `NativeMethods.g.cs`), `abi.md`'s WRITE_DONE paragraph, `architecture.md` where it
  lists the refusals and their wake-ups and where it checks the acquittals, `contract.md` where
  "the next must wait for a WRITE_DONE to free a slot", the comment in `call/mod.rs` on the slot's
  wake-up, and the comments in `FfiGrpc.tla` on the slot refusal and on WRITE_DONE. A search for
  `SLOT_BUSY`, "exactly once" and "WRITE_DONE to free" finds those and the tests, models and
  comments beside them.
- `abi.md` and `contract.md`, wherever a callback carries one event or every send is acquitted,
  and `architecture.md`, whose sketches are normative: the trampoline, the ring's publication and
  `ak_event_consumed`.
- The hand-kept layout checks - `tests/layout.rs`, `AbiLayoutTests.cs` and the test host in
  `tests/support/host.rs` - for the callback.
- `formal-model.md`'s mapping table: the rows of `SendMessage` and `EndSend` name their new
  linearization point, `EmitWriteDone` and `WriteDoneReturns` theirs on a one-request call,
  `ak_events_consumed` gets a row - `n` `HostConsumesEvent` steps - and
  `tla/ci/check_abi_coverage.py` checks that every ABI entry has one.
- Rules that a declared cardinality reverses: `contract.md`'s note that "the cardinality is not
  declared at start", and `decisions.md`'s row on the replay ceiling, whose ceiling "needs no
  cardinality declared at start" - it still needs none, but calls may declare one.
  `architecture.md`'s "Unary is not a special case" holds: the response's message still travels
  the ring and is parsed by the reader, typically woken once.
- The rule that a status waits behind the gate as its messages do, decided on 2026-10-03, which
  decision 12 reverses for a one-response call's status: `architecture.md`'s "A status the peer
  sends is read where its messages are, behind the gate", `tasks.md`'s "A status the peer sends
  waits behind the gate too", the documentation of `ReadGate` in `armonik-transport`'s
  `grpc/call.rs`, and the test `the_gate_is_asked_before_each_read_the_status_included` in
  `tests/grpc_read_gate.rs`.
- Requirement 2.5 and its status, and the promise on `timeout_ns` of zero in `abi.rs` (so the
  header and `NativeMethods.g.cs`) and in `abi.md`, as the deadline paragraph says.
- The engine's structure that step 1 replaces - a driver, an actor with a writer and a reader -
  wherever `architecture.md` and `formal-model.md`'s mapping table describe it.
- `decisions.md`: the row "How much of tonic the engine reuses" (decided 2026-09-28) has the
  engine run every call through tonic's client and accept its encoder's copy. That is amended for
  the copy only: a one-request call still goes through tonic's client, which writes its request
  head and reads its response, and its body is swapped below tonic for the framed buffer.

## What it should buy

Per unary call, in the typical case: the server sends its head, message and status together, and
the delivery window holds them. A head sent early or a window of one credit each cost one callback
more. The times are the measured costs of what goes, and the whole is measured after each step
rather than added up:

| | today | then |
|---|---|---|
| downcalls | 7 | 4: the start, the lend, the send, one consume |
| wake-ups of the channel's thread from the host | up to 5 | 1 |
| callbacks | 4 | typically 1 |
| .NET pool wake-ups | about 3 | typically 1, when no caller asks for the headers |
| tasks on the channel's thread | 5 | 2 or 3: the call's, hyper's request task, and its body pipe unless step 4's measure shows it gone |
| copies of the sent message | 1 | 0 |
| engine allocations | 80 | counted after each step; the command queue, the request and response channels, the writer's and reader's task cells and tonic's encoding buffer go |

## Decisions

Taken on 2026-10-03:

1. **`AK_ABI_VERSION` stays 1**, for the reason T4.0 gives: the ABI is not published, and the
   crate is `publish = false`.
2. **The delivery window applies to one-response calls** as to every call. With one credit, head
   and message cannot share a batch and the response takes two callbacks; the default of four
   makes the case rare.
3. **The deadline of a call not yet committed is checked at its commit**, as in the deadline
   paragraph: the hot path, where the deadline never fires, saves its wake-up, and a host that
   holds its buffer past its deadline is the one that sees the difference.
4. **The request body is swapped below tonic**, in the engine's HTTP/2 service, tonic's client
   keeping the request head and the response. Whether the response leaves tonic too, to slice a
   message out of its DATA frame without a copy, is a later decision.
5. **A unary call has no AK_EVENT_WRITE_DONE**: an acquittal tells its host nothing, and the
   engine settles the send for itself.
6. **`content-length`** is checked at step 4, on the test server and on Kestrel.
7. **The empty request of a one-request call** is accepted as it is, a C host's mistaken commit of
   a zeroed buffer included.
8. **The request's buffer is not lent with `ak_call_start`.** That would save one downcall - a
   crossing and a lookup in the handle table - and move the native start into the serializer's
   announcement, where a call cancelled, asked for its headers or disposed before it, and an
   argument refused at the start, would all need a new answer.
9. **`INTERNAL` for a broken shape; `ak_call_end_send` refused on a one-request call; the headers
   task on demand.**

Taken on 2026-10-04:

10. **No WRITE_DONE on any one-request call**, server streaming as well as unary: the reason of
    decision 5 holds for both, the host sending one message either way.
11. **A batch carries data events only.** A WRITE_DONE on a call whose sends cross the ABI comes
    alone, as today, rather than heading a batch of data events ready with it. That keeps the
    binding's acquittal as it is and adds no level-2 step for it; a stream whose WRITE_DONE is
    ready with its data takes one callback more than a batch would have, as many as today.
12. **A one-response call's status takes the read gate's turn and not the ledger's admission**: it
    can only yield the trailers or refuse further bytes, and is charged nothing, and the binding's
    reader keeps its one wake. That reverses, for this read, the rule decided on 2026-10-03 that a
    status waits behind the gate as its messages do. Its cost: the message stays in the ring, and
    in the runtime's count, until the terminal comes, the window fills or the call is cancelled, so
    a server that sends its message and then delays its trailers keeps that charge for the whole
    delay. The other way kept the rule and woke the reader on the message as well, at the cost of a
    second wake whenever the status is not in the message's batch.

## Steps

Each step is measured, in alternating passes, before the next starts, and brings with it the
documents it makes false - `architecture.md`, `formal-model.md`'s mapping table and the models'
prose included - so that they never describe what is no longer built. `DotNetBinding.tla` was
the one exception: its new steps are proved, not reworded, so it came last, at step 5.

1. **One task per call**, with no ABI change: the driver, the writer and the reader become one
   task, joined rather than spawned. Done: the transport hands the caller what drives a call
   rather than spawning it, and the engine joins it with the writer and the reader in one task.
   The actor, its writer and its reader stay, as three futures of that task, so no mapping row
   and no passage of the models' prose that names them needs rewriting. Measured with the two
   engines loaded in one process and their unary calls interleaved, 10 000 pairs: `ak_call_start`
   from 10.8 to 8.8 us at the median, the round trip unchanged (a paired difference of 2 us at
   the median, against a spread of 70 between its quartiles), two allocations fewer per call. On
   the .NET 8 benchmark, alternated over six runs, the unary median and the server-streaming
   throughput move within their run-to-run spread.
2. **Batched delivery**: the array callback and `ak_events_consumed`, with its row and its
   arguments' in the mapping table, which `check_abi_coverage.py` requires of every function in the
   header, and the rows of the delivery steps and `DeliveryCallbackReturns`, which a batch moves;
   level 1's prose on one event per callback and on the delivery flag; the binding copies,
   publishes and signals per batch. Done: the transport's driver hands the response to a sink the
   engine gives it, polling each read once before it waits, and the engine's sink stages each event
   with its credit and calls the host when nothing more is ready, the window is spent or the
   terminal is in. The reader task goes. A unary call takes two callbacks, its WRITE_DONE and one
   batch, where it took four: on the .NET benchmark, 7 364 of 11 000 unary answers came as one
   batch of head, message and status, the others as the head alone and then the message with the
   status, Kestrel having written the head first. Measured with the two engines in one process,
   their unary calls interleaved, 10 000 pairs, twice: the round trip 10 us shorter at the median
   of the paired differences (-11.5 and -9.3). On the .NET 8 benchmark, alternated over eight runs,
   the unary median moves within its run-to-run spread (paired medians -12 us under load and -2
   idle), and server streaming gains 2 and 9 %. Four allocations fewer per call.
3. **One response**: `AK_CALL_ONE_RESPONSE`, which the binding sets on a call whose method returns
   one message; what follows the message looked at in the engine's response body; the binding's
   single-pass reader and on-demand headers; the test of a message that takes the runtime's count
   to its limit; the gate's turn alone, for a one-response call's status, with the documents and
   the test of the rule decision 12 reverses. Done: `AK_CALL_ONE_RESPONSE`, which the binding sets
   on unary and client-streaming calls; the response body refusing the first byte of a second
   message, the frame that holds it cut where the first ends; `ReadGate::turn`, asked for the read
   after the message; the binding's single-pass reader and its headers task started when a caller
   asks. On the engine, the flag measured neutral: the two engines in one process, their calls
   interleaved, 10 000 pairs twice, paired medians of 1.7 and 4.1 us. On the .NET 8 benchmark,
   alternated and pinned to the performance cores - unpinned, the spread between runs on this
   hybrid processor hid any difference under 100 us - the unary median moves within its spread (340
   and 348 us idle, paired median +3.5 us; 378 and 376 under load), as does server streaming. Step
   2's one wake-up per callback had already brought a unary answer's pool wake-ups to one, the
   reader going on inline from the prologue's end on the same thread, so this step had none left to
   save.
4. **One request**: `AK_CALL_ONE_REQUEST`, which the binding sets on a call whose method takes one
   message; the commit that ends the sending and spawns the task, the one-shot slot for a call
   spawned early, the framed body swapped below tonic, the replay of the same buffer, no WRITE_DONE
   across the ABI for a one-request call, with the new linearization points of `SendMessage`,
   `EndSend`, `EmitWriteDone` and `WriteDoneReturns` in the mapping table, and the statements on
   WRITE_DONE's "exactly once", the SLOT_BUSY wake-up and the write-done flag that what else
   changes and level 1 above list; and the measure of hyper's body pipe task, of the socket writes
   per request and of `content-length`. Done: `AK_CALL_ONE_REQUEST`, which the binding sets on
   unary and server-streaming calls; the transport's `FramedRequest` and `OneRequest`, a slot the
   commit fills only while the call takes it, so a refused commit leaves the host its buffer whole;
   the call's task spawned by the commit, or earlier by a cancellation or a lend refused for the
   budget; the body swapped in the engine's HTTP/2 service and kept, the same buffer, for a replay.
   Measured: hyper states a framed body's length in a `content-length`, which a stream's request
   has none of, and the test server and Kestrel take it, every one-request call of their suites
   carrying one; the socket writes per request are what they were, one with the writes gathered and
   two without, so hyper's body pipe still takes the DATA after the HEADERS. With the two engines
   in one process and their calls interleaved, 10 000 pairs twice: `ak_call_start` from 8.1 to 3.8
   us at the median, one callback per unary call where there were two, the round trip a paired
   median of 3.4 and 1.7 us shorter, the channel's thread being woken at the commit rather than at
   the start; 177.4 allocations per call and 53 KB, where a stream's call takes 193.9 and 59 KB.
5. **Level 2**: `DotNetBinding.tla` following the binding with its refinement proved again, and the
   TLC instantiability check. Done, as `formal-model.md` describes; the model departs from what
   the steps were to achieve in four places. A call's shape is two constants, `OneRequestCalls`
   and `OneResponseCalls`, not a choice at `StartCall`: a call identity is started at most once,
   so fixing its shape beforehand loses no behaviour and adds no state. The one-response wake is
   in the fairness, not in `BeginParseEvent`: the read's steps under it, `ReaderTakesHead` and
   `ReaderParses`, go on past the wake within a pass, which a guard in `Next` would forbid. A
   batch needs no state: the status is always a batch's last event, so the engine's returns
   between events are `OnEventReturns` and the root's release stays at the batch's own return.
   And `AwaitingWriteDoneHasOneComing` states that the writer awaiting an acquittal is a
   stream's. The refinement and the seventeen liveness properties are proved again, 33437
   obligations at level 2: `SealingPasses` carries the status's and the acquittal's fairness past
   the seal, and `AsleepReaderIsWoken` the consumption on a one-response call.
