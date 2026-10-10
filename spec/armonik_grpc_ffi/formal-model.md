# Formal model — TLA+

The three levels of the specification, what is proved of each, and how each level-1 action
maps to the code. The modules are under [tla/](tla/).

## TLA+ Formal Model

### Structure and location

TLA+ files live in `spec/armonik_grpc_ffi/tla/`. The model is structured in three
refinement levels:

```text
Level 0 - Abstract spec (what the user observes)
    AbstractGrpc.tla

Level 1 - FFI spec (what happens at the C boundary)
    FfiGrpc.tla  refines  AbstractGrpc

Level 2 - .NET binding spec (what happens on the managed side)
    DotNetBinding.tla  refines  FfiGrpc
```

### Level 0 — AbstractGrpc

State variables:
- `runtime_state`: NOT_INIT | RUNNING | STOPPING | RELEASED | FAILED_UNQUIESCED.
  `RELEASED` is the abstract "this runtime is finished" and the ABI's
  `AK_RUNTIME_QUIESCENT` is what refines it. Destroying the runtime has no level-0
  counterpart at all - freeing a handle is not a gRPC concept - so it appears only at
  level 1, exactly like `ReleaseCallHandle`
- `channels`: set of channels (open | closed)
- `calls`: set of calls with their state
- `send_closed`: boolean per call (end_send called)
- Per call, 4 message sequences:
  - `submitted`: messages submitted by the client to the library (via send_message)
  - `sent`: messages actually sent over the network (HTTP/2)
  - `received`: messages received from the network (HTTP/2)
  - `delivered`: messages delivered to the client by the library (via callback/next_message)
- `events_delivered`: ordered sequence of events per call (INITIAL_METADATA, MESSAGE*, STATUS)

#### Safety invariants (to be proved by TLAPS)

Every name below is a conjunct of `SafetyCore` in `AbstractGrpc.tla`, and
`ci/check_property_manifest.py` refuses this list and that conjunction
diverging in either direction. Safety is required only while no runtime has entered the
deliberately unconstrained failed state: `SafetyInvariant == NotFailed => SafetyCore`.

**Event sequencing per call:**
- **MetadataFirst**: the first event of `events_delivered` is INITIAL_METADATA
- **NoEventAfterStatus**: a status event is the last event, which also makes it unique -
  two would each have to be last
- **TerminalStatusEquivalence**: a call is terminal exactly when it carries a status
  event, so "the call is over" and "the host has been told" are one fact and not two
- **EventStreamShape**: the whole event grammar in one predicate: the first event is
  INITIAL_METADATA, every later one is MESSAGE or a status kind, and every interior one
  is MESSAGE - a status can only sit last. The two bullets above are its ends, kept as
  their own conjuncts; the shape is what the event counting builds on
- **MessageEventsMatchDelivered**: one MESSAGE callback per delivered message, stated as
  a count: the events of a used call number one metadata, plus one per delivered
  message, plus one status once it has arrived. With the shape this pins the event
  stream to the delivery sequence - no MESSAGE event without its message, none missing,
  none doubled

**Message integrity (prefix invariants, liveness of equality):**
- **SubmittedPrefixOfSent**: `sent` is a prefix of `submitted` at all times
- **ReceivedPrefixOfDelivered**: `delivered` is a prefix of `received` at all times
- **CompleteDelivery**: a call that reached its terminal without being cancelled
  delivered everything it received. This is safety, not liveness: it constrains the
  terminal step rather than promising one
- No property equates `sent` with `submitted`, even on success: a server may answer
  without reading everything, and a cancellation abandons accepted sends. The only
  safety relation between the two is `SubmittedPrefixOfSent`, and the liveness one is
  positional with termination as an escape. What is owed for an abandoned send is its
  WRITE_DONE, not its transmission

**Send-side sequencing:**
- **SendAfterEndSend**: once `end_send` is called the call is half-closed or terminal,
  and `SendMessage` is guarded on neither, so no send can follow

**Ownership - everything belongs to a runtime:**
- **SingleRuntime**: at most one runtime with state ∈ {RUNNING, STOPPING, FAILED_UNQUIESCED} at all
  times. The ABI enforces it as well: `ak_runtime_create` and `ak_runtime_create_from` refuse a
  second live runtime with `AK_STATUS_INVALID_STATE`, so the model states the contract's rule rather
  than a convenience of its own. The binding holds one runtime, the object its caller created, from
  which every channel is made - the diagram above shows the invoker's view, not a per-invoker
  runtime. It bounds the state space and lets the shutdown chain be stated per runtime without
  quantifying over interleavings; a second runtime would need it lifted and the shutdown proofs
  redone
- **ChannelOwnership**: a created channel names a runtime
- **CallOwnership**: a started call names a created channel, and therefore a runtime

**Channel ↔ runtime link:**
- **ActiveChannelImpliesActiveRuntime**: an open or closing channel's runtime is
  RUNNING, STOPPING or FAILED_UNQUIESCED - never RELEASED
- **StoppingClosesChannels**: runtime STOPPING ⇒ each of its channels is closing or closed
- **ReleasedNoChannels**: runtime RELEASED ⇒ each of its channels is closed

**Call ↔ channel ↔ runtime link:**
- **ActiveCallImpliesActiveChannel**: an active call sits on an open or closing channel
- **ClosedChannelNoCalls**: a closed channel has only terminal calls
- **ReleasedNoCalls**: runtime RELEASED ⇒ every call of every channel is terminal. It
  says nothing about a callback still unwinding on the host stack; that is
  `ShutdownSignalInv` at level 1, and it is deliberately a separate claim

**Enforced by the action guards, not stated as invariants.** These are true of every
behaviour of the model, but by construction rather than by an inductive proof, so they
carry no theorem and this document must not imply one:
- monotone runtime transitions NOT_INIT → RUNNING → STOPPING → RELEASED, with
  STOPPING → FAILED_UNQUIESCED as the only branch: each action's guard names the state
  it leaves, and `RemainReleased` and `RemainFailed` are the only steps enabled from the
  two terminal states
- a channel is created only from a RUNNING runtime, and a call started only on an open
  channel: `ChannelCreate` and `CallStart` are guarded on exactly that
- no channel or call exists outside a runtime: the sentinel equivalences
  (`ChannelSentinelEquivalence`, `CallSentinelEquivalence`) make "unused" and "no owner"
  the same state, and they are conjuncts of the inductive invariant rather than of the
  safety contract

#### Liveness (conditional on fairness)

The four `~>` guarantees and `EventualMetadata` are the conjuncts of
`LivenessProperties` in `AbstractGrpc.tla`; the same checker binds this list to it.
Every antecedent embeds a trigger the model leaves unfair - `CallStart`, `SendMessage`,
`RuntimeBeginShutdown` - so the library promises what follows a trigger, never that one
occurs. Each is discharged only while no runtime has failed, with `~NotFailed` as the
escape.

- **EventualMetadata**: a started call eventually gets its response head
  (under: scheduler fairness, network progresses)
- **EventualTerminal**: a started call eventually reaches STATUS. Applied to a stopping
  runtime this is what drains it: closing latches cancellation on the channel's active
  calls, so the drain asks the host for no ownership return - though it does need the
  callbacks already dispatched to return
  (under: scheduler fairness, network progresses, client and server each produce a
  finite number of messages)
- **EventualShutdown**: runtime STOPPING ⇒ ◇ RELEASED
  (under: callbacks return, peer responds or timeout. Not under anything the host
  consumes: shutdown deliberately does not wait for payloads to be released)
- **SubmitProgress**: for every position i ≤ Len(submitted), eventually
  `sent[i] = submitted[i]` or the call terminated
- **DeliveryProgress**: for every position i ≤ Len(received), eventually
  `delivered[i] = received[i]` or the call terminated. This is the guarantee a received
  message eventually reaches the host, under WF on the delivery actions and on
  `ak_event_consumed`

Progress is stated per position, not per value: `SubmitProgressAt(c, i)` says the i-th
submitted message reaches the wire at position i with `sent[c][i] = submitted[c][i]`, and
`DeliveryProgressAt(c, i)` is its mirror. Two identical messages are therefore distinct
obligations, which membership in a sequence could not express.

### Level 1 — FfiGrpc

The state is the level-0 state (shared through `AbstractGrpcState`, never redeclared),
two FFI constants — `MaxSendsInFlight` and `DeliveryCredits`, the pipelining depths of
the ABI contract, each assumed a positive natural, plus the buffer identity space — the two
thresholds over the runtime's one count of bytes - `Ceiling`, where work waits, and
`HardCeiling` at or above it, where the engine stops - and twenty-three FFI variables:

- `buffers_held_by_host`: per call, the number of buffers `ak_get_call_buffer` has lent
  and that have not come back. Committing one moves it out of this count and into
  `submitted`, so `SendWindowOccupancy(c) == buffers_held_by_host[c] + Len(submitted[c])
  - write_dones_emitted[c]` is what `MaxSendsInFlight` bounds: the memory the call's arena
  holds for the send path, whichever side is looking at it. It counts against the
  *emitted* WRITE_DONEs, not the returned ones, so the slot goes back when the event goes
  out. `WriteDonesReturned(c) == write_dones_emitted[c] - (1 if the acquittal callback is
  running)` is the other count, and it says which sends are acquitted - the terminal and
  quiescence read it, the window does not
- `write_dones_emitted`: per call, the monotone count of WRITE_DONEs emitted. A send is
  identified by the index of its message in the level-0 `submitted` sequence; WRITE_DONE
  acquits in send order, so this one counter says exactly which sends are acquitted
- `write_done_callback_running`: per call, a WRITE_DONE callback is on the host stack. The
  engine keeps it conservative: on a call that declared one request it is TRUE between the
  commit's `EmitWriteDone` and `WriteDoneReturns`, with no callback
- `delivery_callback_running`: per call, a delivery callback is on the host stack. The
  engine keeps it conservative: TRUE also while it gathers the events of one callback, from
  the first one's delivery step until the callback returns
- `payloads_consumed_by_host`: per call, the monotone count of payloads released. A
  payload is identified by the index of its event in `events_delivered`; release follows
  delivery order, so this one counter says exactly which payloads the host still owes -
  the mirror of the send side, and what the host owes is the gap to `events_delivered`
- `handle_released`: per call, the runtime has reclaimed the call - the handle is stale and
  the arena may go. Set by `ReleaseCallHandle`, the runtime's own step and not a downcall of
  its own: the ABI has no `ak_call_release`, and the step is taken by the thread that clears the
  call's last debt, inside the host's downcall or as the last callback returns
- `cancel_requested`: per call, cancellation latched - by `ak_call_cancel` or by a
  channel or runtime closing. Reclamation does not latch it: it only happens past the
  terminal, so by then there is nothing left to cancel. The latching step models the moment the call's delivery
  task takes the request into account, not the downcall's return: the window in
  between is level-1 stutter (the level-2 command queue), like the send path, so the
  guard `~IsCancelRequested` on `DeliverMessage` costs the implementation one test in
  the task's own loop, never a cross-thread lock
- `shutdown_event_emitted`, `shutdown_callback_running`: per runtime, the
  SHUTDOWN_COMPLETE discipline
- `second_event_owed`: per runtime, the tag SHUTDOWN_COMPLETE carried. The event
  says the runtime stopped running and reports whether the host still holds any of its
  memory; the answer is recorded because the second event is owed only when it was yes.
  A host told nothing was outstanding is spared the work of returning anything; it
  still reaches quiescence through the status, which is the only permission to
  destroy or unload
- `resources_released_emitted`, `resources_released_callback_running`: per runtime, the
  RESOURCES_RELEASED discipline. This is where the difference between the two observable
  statuses lives: `AK_RUNTIME_GRPC_STOPPED` and `AK_RUNTIME_QUIESCENT` are two refinements of
  the one level-0 RELEASED state, so the level-0 model cannot carry it and level 1 never
  writes level-0 state
- `runtime_destroyed`: per runtime, `ak_runtime_destroy` happened. `RuntimeDestroy` is
  guarded on `IsRuntimeQuiescent` - released, neither callback on the stack, the second
  event out if the first one said it was owed, nothing owed or lent across any call of
  the runtime, and no bytes given back but not yet released - and it stutters on the
  level-0 state,
  because destroying a handle is not a gRPC concept. Like every other downcall it carries
  no fairness. It is also what makes the handles stale: `IsRuntimeOfCallDestroyed` guards
  `ReleaseCallHandle` and `RequestCallCancellation`, the only two downcalls a finished
  call could otherwise still accept
- `buffer_state`: per call and per buffer, where that allocation is in its life -
  `none`, `lent`, `returned`, `freed`. Monotone, so an identity is used once. This is
  the one place the model carries a name the counters cannot, and it is tied back to
  `buffers_held_by_host` by `LentCountMatchesBufferStates`, which is what keeps every
  proof that reads the counter standing. `returned` and `freed` are two states because
  they are two events: giving the buffer back is the host's, releasing the memory is the
  runtime's, and the transport still reading the bytes is the gap
- `buffer_send`: per call and per buffer, the index of the send living in that
  allocation, zero for none. Keyed by the buffer because that is how
  `ak_call_send_message` keys it, so both questions the model asks are lookups - may
  these bytes go, and are they still needed. Without it nothing forbade releasing the
  memory of a message still on its way to the wire

- `buffer_charge`: per call and per buffer, the bytes the allocator handed out for that
  allocation. It is what the budget counts, and it is written once on a fresh buffer and
  rewritten only by an exchange (`ResizeSendBuffer`), to zero, when it becomes the new
  buffer's
- `buffer_length`: per call and per buffer, the bytes `ak_buffer.len` exposes. Distinct from
  the charge because the allocator may round a request up: `FitsInBuffer` reads the length,
  so a commit must fit the view the host was given, while the ceiling counts what was really
  taken. `CoversRequest` ties them at the lend and nothing relates them afterwards
- `memory_used`: the runtime-wide counter, moved the way an implementation moves it rather
  than evaluated as a sum on demand: up by the lend and by a received message as it arrives
  decoded, by the difference on an exchange of a lent buffer, down by the free, by the consumption of a message payload, and by a cancelled
  call's end, which gives back what it received and never delivered; up and down by what the
  engine takes for itself and gives back. Typed `Int`;
  `MemoryAccountingExact` is what makes it non-negative and what makes the thresholds mean
  anything
- `engine_held`: the runtime-wide bytes the engine holds for itself, the compressed copies of
  the messages it sends. A part of `memory_used`, and one abstract number: the model names
  neither a copy nor its message, and what the ceiling sees is the sum. Typed `0..Ceiling`: the
  engine takes against the first threshold. Nothing the host owes is in it, so no quiescence
  condition reads it
- `last_lend_status`: per call, what its last `ak_get_call_buffer` returned - `OK` or one
  of the three refusals. Nothing else reads it, which is the point: it is the observable
  frontier of the downcall, the state a level-2 binding refines its retry decisions
  against, without giving any other action a new way to be blocked. It records the
  backpressure sub-machine only - `OK`, `MESSAGE_TOO_LARGE`, `SLOT_BUSY`, `BUDGET_BUSY`;
  the rest of the downcall's result matrix (`HANDLE_STALE`, `INVALID_STATE`,
  `INVALID_ARG`, `INTERNAL`, and `*out` untouched on every refusal) is the ABI matrix's
  rows and the conformance tests' burden, not this variable's. An identity of
  attempts finer than the call - positions, tickets - is level 2's to introduce if its
  retry model needs one
- `read_admitted`: per call, the call may read its next message. A read is two steps, as the
  engine's is: `AdmitRead` takes the decision against the first threshold, and the message is
  charged when it arrives decoded, where the second threshold applies. Calls admitted together
  may pass the first threshold by a message each, which is the overshoot the second bounds and
  which an atomic read could not represent. A call is admitted only once it has delivered
  everything it received, so it holds at most one decoded message the host has not been handed
- `lend_waiting`: per call, the length a send refused for room waits on, zero for none. While
  one waits, reads are admitted only below the first threshold lowered by it, so a refused send
  is served before new reads. The length and not the charge: a refused charge may exceed the
  first threshold, and a lendable length cannot. Cleared when the send is served and when the
  call ends
- `budget_wake_owed`: per call, a release that gave bytes back happened while its send
  waited. `EmitBudgetWake` pays it, one step with its callback's return: the engine raises the
  event from the task that drives the call, the one that also delivers the terminal, so the two
  never overlap. Not on a call being cancelled or whose trailers are in

**The exchange of a lent buffer.** `ResizeSendBuffer(cId, b, nb, len, charge)` is
`ak_resize_call_buffer`: the host holds `b` and asks for another buffer, `nb`, of `len` bytes.
It is a lend of `nb` and the return of `b` in one step, which is the point of it: `memory_used`
moves by `charge - buffer_charge[b]`, so the two charges are never both on the counter, and the
room `b` held is not offered to another call between the return and the lend, which two
downcalls would leave open. `IsMemoryAvailableForExchange` is the guard, the lend's with `b`'s
charge taken off first, and asked only for a growth: received messages may have taken the
counter past the first threshold, and giving memory back is never refused. `b` is `returned` with its charge set to zero, since the charge is now
`nb`'s: its release, `FreeReturnedBuffer`, is owed at once, carries no send, and takes nothing
off the counter. It stays a state of the chain `lent`, `returned`, `freed`, which is what keeps
the liveness proofs about a lent buffer standing. `buffers_held_by_host`,
`last_lend_status`, `lend_waiting` and `buffer_send` do not move. The call's one lent buffer
and its slot of the send window are `b`'s and become `nb`'s, and nothing waits: the host holds
a buffer, and a call whose send waits holds none, which is why the exchange has no
`EndWaitOf`. A release owes the waiting calls their wake-up, so a shrink does, and a growth
does not. The bytes kept are not an argument: what the host wrote is no state of the model,
and `FitsInBuffer` is read at the commit against `nb`.

Its refusals - the ceiling, a call that is over, a buffer that is not lent - take no step, as a
refusal of any other downcall that is not a lend does, and a refusal for room records no wait:
a host that waits gives the buffer back and lends, and that is the wait the model has. The
overrun the ABI answers with `AK_STATUS_CORRUPTED` is not modelled, as at the commit. The
action carries no fairness - exchanging is never owed - and the buffer it lends is
under the fairness `HostReturnsBuffer` already has, per buffer. Each exchange spends a fresh
identity, so `BufferIds` bounds the exchanges of a behaviour as it bounds its lends.

**The engine's own bytes.** When a call sends a message on a channel that compresses, the
engine builds a compressed copy and charges it against the ceiling as a lend is charged, for as
long as the message that holds the copy lives; `CopyBudget` in `ledger.rs` is that charge. The
model has two steps for it and no copy. `EngineTakesBytes(cId, n)` is `hold_copy`: for an
active call that has sent a message, `n` bytes join `engine_held` and `memory_used` when the first
threshold has room for them. A copy that does not fit is not made and the message goes out
uncompressed, so a refusal is no step: taking never blocks and is never owed. `EngineGivesBackBytes(n)` is
`release_copy`: the messages that held `n` bytes of copies are dropped, the bytes leave both
numbers, and the calls whose send waits are owed their wake-up, as after any release. The
lend's room check and both ceiling invariants read `memory_used`, so they see these bytes: a
lend can be refused because the engine holds part of the budget, which a model without the term
would have admitted. `RuntimeCreate` waits for the engine to hold nothing, as it waits for the
predecessor to be destroyed: a restriction of the model, which keeps one counter where the code
keeps a ledger per runtime.

The fairness is on one instance, `EngineGivesBackAllBytes`, which gives back everything the
engine holds. Giving back some would not serve: takes are unbounded in the model, and an
engine that gave a byte back and took a byte would keep the count up for ever. The code makes
one copy per message, so its count reaches zero once its messages stop: the conjunct is the
runtime's step and an assumption on the application, that its compressed sending stops, and
the fairness table lists it apart from what the runtime owes. The model has no copy to
count, so the fairness says the engine's holdings drain, and `EngineBytesEventuallyGivenBack`
is what it buys. The takes carry none, nor does a partial give-back. The weak form is enough:
whenever the engine holds something the step is enabled, and it stays enabled until it is taken.

Deliberately absent: no handle registry (validity is modeled, not the numbering, so the
registry's counter is an implementation of handle validity rather than a modelled object), no read-credit variable (`ak_event_consumed` frees and arms in one
gesture, so credits available + payloads owed = `DeliveryCredits` on a live call and
one variable suffices), no start gate (derivable from `runtime_state`), no boundary
message lists (the in-flight gaps are the derived differences between the level-0
sequences).

Every action either refines a level-0 action (conjoining FFI guards and updates onto
the instantiated `L0!` action) or stutters on the level-0 variables; the level-0
machinery is the only writer of the level-0 state.

Guards, invariants and properties are written through named state predicates
(`HasFreeSendSlot`, `IsCancelRequested`, `IsRuntimeDrained`, ...); reading a
variable directly is reserved to update expressions, `Init` and `TypeOK`.

Three naming rules hold throughout, and they are written here because a reader who
has to induce them cannot tell a predicate from a step:

- **A stative verb is a predicate, a dynamic verb is an action.** `HostHoldsNoBuffer`,
  `HostOwnsNoPayload` and `RuntimeOwesFree` describe a state; `HostReturnsBuffer`,
  `HostConsumesEvent` and `RuntimeRelease` are steps. The party prefix says whose
  obligation or whose step it is, and never which of the two it is - the verb does that.
  There is no exception in either module.
- **`Is…` describes the object its argument names, `Has…` describes the call.**
  `IsLentBuffer(c, b)` is about the buffer, `HasFreeSendSlot(c)` about the call.
- **`Deliver…` carries a payload and spends a delivery credit; `Emit…` carries neither.**
  That is why WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED are emitted and the
  four data events are delivered. Note that `DeliverStatus` and `DeliverCancelled` both
  produce `AK_EVENT_STATUS`: two model actions for one event kind, distinguished by the
  status the payload carries. Every delivery action adds exactly one owned event, and the
  model counts the host's debt off `events_delivered`. A callback that carries several
  events is the sequence of their delivery actions, each but the last returning before the
  next is taken (the mapping table below), so no action appends two. `DeliverCancelled`
  therefore requires the initial metadata to be out already: a call cancelled before
  anything was delivered takes two steps, INITIAL_METADATA first - `DeliverInitialMetadata`
  fires on its own weak fairness - and the cancellation second, which may share a
  callback.

Additional invariants (the FFI conjuncts of the level-1 inductive invariant):
- **SubmittedOccurrencesGloballyUnique**: an occurrence token is committed at most once,
  across every call - the submitted sequences are jointly injective, `NeverSubmitted`
  guarding the commit and this invariant making the guard citable. It is what lets a
  submitted message name its send, and the k-th WRITE_DONE name its message
- **ReceivedOccurrencesGloballyUnique**: the receive side of the same discipline - one
  reception per token across every call, `NetworkReceive` being guarded on a token never
  received anywhere
- **DirectionsShareNoToken**: a token names one occurrence in one direction, so a received
  token never reappears in emission nor an emitted one in reception - across all calls.
  Position orders each direction; the tokens are what `buffer_send` and the k-th
  WRITE_DONE key on, and what makes "the same bytes twice" two occurrences rather than
  one ambiguous value. Payload content, were it modelled, would be a separate function of
  the token, as `MessageLength` already is
- **UnusedCallsAreFfiClean**: no FFI state before `ak_call_start`
- **ReleasedCallIsClean**: a released call is terminal, every payload consumed, every
  buffer given back and freed, no delivery callback on the stack and no send in
  flight - the release's full postcondition, carried as an invariant so a client can
  cite what the reclaim guaranteed rather than re-deriving it from the guard. And it
  stays that way, because nothing can lend or deliver afterwards. This is the end-of-call guarantee: whatever the ending, Rust has everything
  back before the arena goes. The reclaiming step also waits for the call's own delivery
  callback to return and for every buffer given back to be released, both conditions only
  the runtime can read, which is why they moved into the guard when reclamation stopped
  being a downcall the host could be asked to establish
  it, and the runtime defers its teardown instead
- **SendsInFlightWithinLimit**: never more buffers out of one call's arena than
  `MaxSendsInFlight`, counting those the host is filling and those awaiting WRITE_DONE
- **WriteDonesNeverExceedSends / RunningWriteDoneWasEmitted**: the send-side
  no-double-free — never more acquittals than accepted sends
- **ReleasesNeverExceedDeliveries**: the host never releases more payloads than were
  delivered. This is conservation of a count, not of identities: the model does not track
  which `owner` a release names, so two releases of the same payload are indistinguishable
  from the correct release of two. It therefore proves *no over-consumption under a
  conformance assumption* - the host releases the right owner, once, in order - and that
  assumption sits beside the fairness hypotheses rather than being discharged here. A
  defensive ABI that detects a duplicate token would need the identities modelled; this
  design chooses assume-guarantee instead, and the level-2 obligations are where the
  binding pays it
- **TerminalCallHasNoSendInFlight**: WRITE_DONE always precedes the terminal
- **ClosingChannelCallsCancelRequested**: both paths into closing latch cancellation on the
  channel's active calls, so a closing channel drains without the host
- **ActiveCallPayloadsWithinCredits / PayloadsOwnedWithinCreditsPlusOne**: at most
  `DeliveryCredits` payloads owed while the call is active, one more only when the last
  is the terminal
- **LentCountMatchesBufferStates**: the bridge between the two views of the send buffers -
  the count `buffers_held_by_host` and the per-buffer state - is that the count is the
  cardinality of the lent ones. Both are kept on purpose: the count is what the send window
  and every proof about it read, the states are what carry an identity, and without this
  conjunct they could drift. That is the same failure as the send window and the ABI comment
  describing it coming apart, which is why it is stated rather than assumed
- **UnusedCallsHaveFreshBuffers**: a call that has not started has allocated nothing, the
  per-buffer form of `UnusedCallsAreFfiClean`
- **BufferSendIndicesExist**: a send index recorded against a buffer names a send that
  exists. `buffer_send[c][b]` is the send living in allocation `b`, zero for none - the
  identity `ak_call_send_message` takes and the counters could not express. Keyed by the
  buffer rather than by the send index, on purpose: both questions the model asks are then
  lookups, and one allocation carrying two sends is unrepresentable instead of being
  something an invariant has to forbid
- **CommittedBuffersAreGivenBack**: a buffer that carries a send is `returned` or `freed`,
  never lent again. Committing is one of the two ways to give a buffer back, so this is what
  stops one allocation being handed out twice
- **EverySendHasItsBuffer / SendsLiveInOneBuffer**: each accepted send lives in exactly
  one allocation. Keying `buffer_send` by the buffer makes one allocation carrying two
  sends unrepresentable, but it says nothing about the other two directions: that no send
  exists without a buffer, and that no two buffers claim the same send. Both are true by
  construction - a commit records the index the sequence is about to reach, which is above
  every index already recorded - and both are now stated rather than left to be read off
  the actions
- **UnacquittedSendKeepsItsBytes**: the bytes of an unacquitted send are still there. This
  is the conjunct that closes a real hole: without the send-to-buffer link, nothing forbade
  releasing the memory of a message the transport had not read yet, and no counter could
  have caught it because a counter does not know which allocation carries which send
- **NoDeliveryImpliesNoDebt**: a call that has never received a delivery owes no payload
  and has no delivery callback running. The delivery debt is created by the same step
  that appends the first event, which is what gives the initial metadata its credit
  without any help from the host
- **ActiveCallHasNoStatus / UnusedCallHasNoEvents**: the status event terminates the call
  in the step that appends it, so an active call never carries one and a call that has
  not started carries nothing. Both are guard-based, so they survive a failure - the
  cancellation drain needs them there
- **ShutdownSignalInv**: two halves, stated apart because they are preserved by
  different arguments. `ShutdownSignalCore` says SHUTDOWN_COMPLETE is emitted exactly
  once, from a drained runtime, and that release waits for its callback to return.
  `ReleaseSignalInv` says the second event is owed before it is sent, its callback
  never outlives it, the tag is accurate - a runtime whose event said nothing was
  outstanding really had an empty ledger - and the second event is honest: once
  `AK_EVENT_RESOURCES_RELEASED` has gone out, nothing of the runtime is in the host's
  hands and nothing the host gave back is still waiting to be freed. That last conjunct
  is what makes the event mean what the ABI says it means rather than only arrive; the
  two that read a ledger are the ones that need a drained runtime's ledger not to grow,
  which is why the split keeps each preservation obligation the size it was
- **MemoryAccountingExact**: the runtime-wide counter equals the charges of the send
  buffers actually out plus the lengths of the received messages held - by the host, until it
  consumes them, or by the engine, until it delivers them or the call ends - plus the bytes
  the engine holds for itself. This is the
  accounting claim with content, and it can fail - a lend that forgets its increment, a free
  that forgets its decrement or subtracts the wrong charge, a second credit for one buffer, a
  consumption or a cancellation that gives back the wrong bytes, a copy charged and not
  counted. Defined as the sum it
  would be a tautology, which is why `memory_used` is a variable the actions move rather
  than an expression evaluated on demand: that is how the implementation keeps it, and it
  is the number `ak_runtime_memory_usage` publishes. Five categories of the detailed
  observer are definitions over the same charges and lengths, and the sixth is
  `BytesHeldByEngine`; that the five add up with it to the total is
  `CategoriesPartitionTotal` and `ReceivedCategoriesPartitionTotal` and the accounting
  itself - lemmas and not invariants, since the first two hold of any state.
  Both sit outside the `NotFailed` umbrella: failing changes neither the counter nor any
  charge, so a host still gets its memory back afterwards and the observers still answer
- **MemoryWithinHardCeiling**: the counter never passes the second threshold. Carried by
  the guards of the four steps that add - a lend, the growth of an exchange and the engine's take below the
  first threshold, a received message below the second - every other step only subtracting. The first threshold is not an
  invariant: calls admitted together may pass it, by a message each
- **ReceiveAccountingInv**: what the received side's accounting reads of a call's counts. It
  delivers no more than it received, holds at most one message past its deliveries, and an
  admitted read follows its deliveries; an active call has delivered its metadata and one
  event per message at most, and a delivered message follows the metadata. Together they
  bound what a call holds by its delivery window, which is what the room argument descends
  on. Guard-based, and outside the `NotFailed` umbrella with the accounting it serves

- **DestroyedRuntimeIsClean**: a destroyed runtime is quiescent, and stays quiescent -
  the whole gate, not half of it, so the invariant's name and `ak_runtime_destroy`'s
  precondition are the same sentence. Quiescence is absorbing for six separate reasons:
  RELEASED is a level-0 end state, neither runtime callback can be re-entered because
  the conjunct forbidding it is also what its writer's guard demands, the tag is frozen
  and the second event is a latch, and neither ledger can refill once the calls are
  quiet. It is kept apart from `ShutdownSignalInv` because it is the only runtime-level
  conjunct that reaches into the calls

Usability, proved of the ABI itself and assuming nothing of the host - what it permits it
does not then withdraw, and what it withdraws it withdraws completely:
- **DeliveryCallbacksReturn / WriteDoneCallbacksReturn / ShutdownCallbacksReturn /
  ResourcesReleasedCallbacksReturn**: every callback the runtime hands out comes back.
  These are the obligations the ABI imposes on the host, stated as guarantees so the
  fairness conjuncts they rest on are not hypotheses with nothing to buy. The last one
  is what makes `AK_EVENT_RESOURCES_RELEASED` safe to owe: without it the runtime could
  wait forever on a callback it dispatched and never reach `AK_RUNTIME_QUIESCENT`
- **DestroyedRuntimeRejectsHandles**: after `ak_runtime_destroy`, no downcall on any call
  of that runtime is enabled - not release, not cancel, not lend, not any of the three
  lend refusals (not even a status is written), not send, not end_send, not returning a
  buffer. Two of them are refused by a guard; the rest follow from what
  destruction already required, a released runtime having no live call and nothing of its
  memory outstanding. This is the formal content of "destroy invalidates every handle of
  the runtime": asserted here, and carried by a theorem rather than by the prose alone
- **WriteDoneFreesASlot**: `EmitWriteDone(c) ⇒ HasFreeSendSlot(c)'`. A host woken by a
  WRITE_DONE and asking for a buffer is never refused for want of a slot - cancellation, a
  closed send side or a retired handle each still refuse one on their own grounds. Nothing
  forced this to be stated -
  the send side is host-driven, so no fairness lift needed it - and its absence is what
  let the slot accounting drift from the ABI it documents. The receive side has the same
  property and got it by accident, because the `DeliverMessage` lift needed it

New liveness guarantees:
- **CancellationCompletes**: a cancelled call reaches its terminal without any ownership return from the application - the callbacks already in flight still return, which stays a host hypothesis
- **SendsEventuallyAcquitted**: per send — the k-th accepted send is acquitted by its
  in-order WRITE_DONE, whose callback returns
- **PayloadsEventuallyConsumed**: per payload — each payload handed over is
  individually consumed; rests only on the per-call host hypothesis
  `WF(HostConsumesEvent(call))`, which carries every payload of the call because
  release is FIFO
- **ShutdownEventEmitted**: a stopping runtime emits SHUTDOWN_COMPLETE
- **EventualChannelClosed**: a channel told to close closes, its calls cancelled and
  drained - what `ak_channel_release` promises. It rests on the runtime's fairness and
  on the host hypotheses too: the callbacks already dispatched must return for the
  drain to finish. Previously
  derivable and citable by nobody; the theorem makes it part of the interface
- **BufferEventuallyFreed**: every buffer the arena lends out is given back and then
  released. Two rungs with two owners: the host returns it, per buffer because returns are
  unordered, and the runtime releases the bytes once the send they carry is acquitted. The
  transport still reading them is the gap between the two
- **CallEventuallyReclaimed**: a terminal call is reclaimed - handle retired, arena gone -
  without the host doing anything beyond giving back what it holds. This is what removing
  `ak_call_release` from the ABI buys: the guarantee is unconditional where a downcall the
  host might never make could not be. The escape is `ak_runtime_destroy`, which takes the
  arena with the runtime; there is deliberately no failure escape, because every action the
  drain rests on is untouched by a runtime failure
- **RuntimeEventuallyQuiescent**: a runtime that has stopped running reaches
  `AK_RUNTIME_QUIESCENT`, so a host polling `ak_runtime_status` is not waiting for
  nothing, and may then destroy or unload - and once destroyed, start a new
  runtime. Note the shape - the
  runtime promises the permission, never the destruction, because destroying is the
  host's call. It is stronger than the host's ledger emptying: it also carries the last
  callback having returned, which is the only thing that can say the trampoline thread
  is gone, and no callback could ever report that about itself. It carries the same
  failure escape as `ShutdownEventEmitted`: what makes a released runtime's calls
  terminal is a level-0 invariant, and level-0 safety is asserted only while no runtime
  sits in the failed state. The drains themselves survive a failure - they rest on
  `FfiCallInv` and `BufferStateInv`, both outside that umbrella - so the escape covers
  the premise, not the mechanism

- **ResourcesReleasedEventually**: a runtime whose `SHUTDOWN_COMPLETE` carried
  `AK_HOST_MUST_RETURN` does emit `AK_EVENT_RESOURCES_RELEASED`. This is the tag's own
  promise, and it is why a host may wait for the second callback rather than poll from
  the first. It is stated on the tag rather than on the runtime state because the tag is
  what the host reads, and it is derived rather than re-proved: the level-0 shutdown
  settles, the runtime quiesces from there, and quiescence with the tag set is the event
  having gone out. Level 2 refines this one rather than re-deriving it

- **EngineBytesEventuallyGivenBack**: the bytes the engine holds for itself are given back -
  `HasEngineHeldBytes ~> ~HasEngineHeldBytes`. A copy goes with the message that holds it, so
  what the engine keeps is dropped once the application's compressed sending stops, and this is
  what the fairness on `EngineGivesBackAllBytes`, an assumption on the application, buys:
  without it the engine's share of the counter could stand for ever and no send refused for
  room would be promised room. It says nothing about what the engine takes meanwhile

- **RefusedSendEventuallyHasRoom**: a send refused for room eventually has room while it
  waits - or stops waiting, its call having ended, or the runtime failed. The room is in the
  model's accounting, not the allocator's: the allocator picks the charge, its size classes are
  its own, and whether a realizable class fits is the implementation's contract, carried by the
  ABI matrix and its tests. `HasAccountingRoomForSomeCharge(len)` - some charge in the model's
  range covers the request and fits - is the existential the property closes on; a
  `AK_STATUS_BUDGET_BUSY` refusal denies one charge, not all of them. Proved from the hold the
  waiting length puts on reads: once the reads already admitted have landed nothing more is
  received, every call then drains - its metadata and each message it holds delivered and
  given back, on a ladder its delivery window bounds - every buffer out is freed, the
  accounting leaves on the counter what the engine holds, and room is seen at a state where
  the engine holds nothing, which `EngineBytesEventuallyGivenBack` brings about under the same
  assumption on the application's compressed sending: there the
  counter is zero and the request is its own witness. The engine may take bytes again at once,
  so what the property gives is a state with room and not a room that stays. Without the
  hold, received bytes would keep the counter up under steady traffic and nothing would fall.
  It says nothing about who is served: lending carries no fairness and a competing caller may
  win the race. What the waiting send is owed is the wake-up, which `EmitBudgetWake` carries,
  and a host woken is obliged to try the send again or to cancel its call: one that gives up a
  send and keeps its call holds reads back for the whole runtime. What a retry loop is owed is
  level 2's `BudgetWaitEndsWhenHopeless`.

#### Fairness

Twenty-one weak-fairness conjuncts, all individual, and they do not all belong to the same
party. Which side owes each one is the whole point of listing them, because the ones the
host owes are exactly the obligations a level-2 binding has to discharge.

| Owed by | Conjuncts | What it means |
| --- | --- | --- |
| Rust runtime | `NetworkSend`, `ReceiveStatus`, `EmitWriteDone`, `EmitBudgetWake`, `RuntimeRelease`, `EmitShutdownComplete`, `EmitResourcesReleased`, `ChannelFinishClosing`, `FreeReturnedBuffer`, `ReleaseCallHandle` | Its own threads and its own allocator, and its own code inside a downcall: `ReleaseCallHandle` is taken where the last debt clears, on the host's thread too. Nothing outside the library can stall them |
| Rust runtime, assumed of the application | `EngineGivesBackAllBytes` | An assumption that the application's compressed sending stops, not something the library guarantees |
| FFI layer | `DeliverInitialMetadata`, `DeliverMessage`, `DeliverStatus`, `DeliverCancelled` | An event that reaches the queue reaches the host |
| Host (binding + application) | `DeliveryCallbackReturns`, `WriteDoneReturns`, `ShutdownCallbackReturns`, `ResourcesReleasedCallbackReturns`, `HostConsumesEvent`, `HostReturnsBuffer` | Six hypotheses the ABI imposes and cannot enforce |

The four callback-return conjuncts say that the callback the *host* installed eventually
returns. For our own .NET binding that is a property we implement and can point at; for
any other host it is an obligation the ABI imposes. The model is right to assume it -
nothing can make progress otherwise - but calling it a promise of the binding overstates
what is ours to guarantee.

The next two are about giving memory back. `HostConsumesEvent` is per call, which suffices
because payload release is FIFO: consuming past a payload without consuming it is not a
behavior the ABI admits. `HostReturnsBuffer` is per *buffer*, because buffer returns are
unordered - a per-call conjunct would let a host cycle some buffers while starving one.

`AdmitRead` carries none either: what arrives is the peer's to drive, as the level-0 receive
is, and a call held back below the threshold reads again only when a release makes room.

The remaining downcalls (`CallStart`, `LendSendBuffer`, `ResizeSendBuffer`, `SendMessage`, `EndSend`,
`RequestCallCancellation`, `RuntimeBeginShutdown`, `RuntimeDestroy`) carry no fairness:
the model never promises the host acts, only what follows when it does. `ReleaseCallHandle`
is not among them, because it is not a downcall - the runtime reclaims a settled call
itself, which is what removing `ak_call_release` from the ABI buys. Runtime shutdown
completes without any ownership return from the host, which discharges the level-0
directive on
`ShutdownFairness`.

Level-0 safety and the five level-0 liveness guarantees are not re-proved: the
refinement mapping is the identity on the level-0 variables, and `Spec => L0!Spec` is
proved with tlapm by lifting each level-0 fairness conjunct to the level-1 machinery.

### Level 2 — DotNetBinding

`DotNetBinding.tla` refines `FfiGrpc`: the state space, the actions, the fairness and
the properties below are the specification as written, and they cover three mechanisms -
the runtime's lifetime, the write machine, and the managed completions.  The modules are
TLC-vetted: thirty-nine of the forty actions fire, the fortieth being dead by
design, below.  No one configuration fires them all: a call's shape is fixed per
configuration, so the stream's write steps and the one-request call's are covered by
different runs.

**Scope.** The model is the generic bidirectional-streaming call.  The five
`CallInvoker` methods are refinements of it that fix the number of messages in each
direction, not separate machines.  What a call declares of them is modelled, as a shape:
two constants, `OneRequestCalls` and `OneResponseCalls` - a call identity is started once
at most, so fixing its shape beforehand loses no behaviour and adds no state.  A
one-request call's commit ends its sending and is acquitted by the engine; a one-response
call's reader waits for the whole response, and composes the same reader and the same
terminal consumer as any other.  The start/send command queue is level-1
stutter.  The .NET async runtime is not modelled: completions signal and continuations
run elsewhere - `RunContinuationsAsynchronously` on every TCS is an implementation rule
verified by review and tests, not a theorem, and the model carries no counter pretending
otherwise.  The memory model of the ring stays a coding rule for review, outside every
level.

**Reuse is direct.** `DotNetBindingState` extends `FfiGrpcState`, so the level-0 and
level-1 variables are the same variables, not mapped copies; the level-1 machinery is
reached through `L1 == INSTANCE FfiGrpcTheorems`.  A coupled action conjoins the level-1
action it rides on; a managed-only action leaves `L1!vars` unchanged; the runtime's own
actions pass through untouched.  `L2!Spec => L1!Spec` is therefore the only refinement to
prove - `L0!Spec` follows from level 1's `RefinesSpec` by transitivity.  Ownership needs
no new relation either: `call_channel` already says which channel a call belongs to, and
`ChannelIds` is the channel identity space, so the model needs no invoker identities and no
refcount -
"the sweep is over" is `EveryChannelSettled`, every channel unopened, rejected, released
or disposed. Nothing triggers on it: it is the guard `BeginRuntimeShutdown` reads, and no
step fires because it became true. What keeps a channel from being added behind the sweep
is the door rather than a count - `BeginCreateChannel` requires an active runtime, and
the caller's `DisposeAsync` shuts that door in its own step before any channel is
disposed, which is what the implementation's lock buys.

**The ring has no variables.** The trampoline publishes inside the delivery callback, so
the published prefix IS `events_delivered` and the head is its length; a release is
`ak_event_consumed`, so the tail IS `payloads_consumed_by_host`.  Both indexes are
level-1 state read through level-2 names (`RingHead`, `RingTail`), and two of the
obligations the ring owes come free of proof: release order is held by representation -
advancing a counter can only release the oldest - and a payload cannot be released twice
for the same reason.  What level 2 adds about the ring is who consumes it, and what the
reader and the writer are doing.

Added variables - all discipline, no capacity:
- `call_token_published`, `call_root_live`, `runtime_root_live`: the GCHandle plumbing -
  token and root born with the start, roots outliving the callbacks
- `current_runtime`, `runtime_dispose_state`: the caller's runtime by generation -
  `absent` before the first `Create` and again after a full teardown, so a promise about
  the destroy attaches to the generation that is current, not to some earlier one;
  `disposing` is the caller's `DisposeAsync` entered, the door shut and the sweep running
- `channel_dispose_state`: per channel, `unopened` / `constructing` / `rejected` /
  `active` / `disposing` / `released` / `disposed`.  The native half is live from the
  construction to the release; `rejected` is a refused creation, terminal and holding
  nothing; and the teardown's guard reads this function, never a counter
- `consumer_phase`, `reader_state`: which class of consumer has the ring, and what the
  application's read is doing - `idle`, `waiting` (a suspended `MoveNext`, which may be
  suspended on the metadata during the prologue as well as on a message), `parsing`,
  `parsing_cancelled` (a parse whose token fired, still holding its slot), or `finished`
  once the terminal was consumed and every later `MoveNext` answers at once
- `read_cancel_pending`: a `MoveNext` token has fired and the binding has not yet reacted.
  One flag rather than a read id: it is armed only on a read that has not completed and on
  a call still `active`, so its lifetime IS the identity of the operation it belongs to -
  which holds only under the registration discipline stated above, and needs a read id if
  that discipline cannot be met
- `writer_state`, `retry_len`: the write machine - `sealing` is a one-request commit
  between its send and its end of the sending - and the length its budget wait remembers
- `headers_completion`, `status_completion`: the public objects that must never be left
  pending
- `headers_asked`: a caller asked for the headers, which starts the task that answers
  them
- `call_dispose_state`: the call's own machine, driving the drain

**The runtime and the channels.** `CreateRuntime(rt)` is `NativeRuntime.Create`: the
shared root and `ak_runtime_create_from`, or `ak_runtime_create`, in one step, and no
channel is involved, a runtime being asked for by name rather than derived from the first
channel that wants one.
`BeginCreateChannel(ch)` is `runtime.Channel(...)` entering the channel's constructor -
the door read under the runtime's lock, no native step of its own; `CreateChannel(ch)` is
its `ak_channel_create`, after which the constructor returns and the object is exposed -
binding-owed, a constructor that began completes.  `BeginDisposeChannel` remembers the
public `DisposeAsync`; `DisposeCallForChannel` settles the calls of that channel and no
others; `FinishDisposeChannel` closes the native channel once they are all disposed, and
stops nothing else.

The teardown is the same shape one level up.  `BeginDisposeRuntime` is the caller's
`DisposeAsync` entered, and it shuts the door; `DisposeChannelForRuntime` is the sweep -
the owner disposing what it owns, by the same public step a caller would take, which is
why disposing a channel twice has to be a no-op and why both orders are ordinary;
`BeginRuntimeShutdown` fires only once every channel is settled, then
`FinishDisposeRuntime`, then `FreeRuntimeRoot` - which returns the runtime to `absent`,
so a caller may create another.

**A refused channel creation is not a runtime failure.** `ak_channel_create` performs no
I/O - connecting is a separate step - so it fails only on a bad configuration
(`AK_STATUS_INVALID_ARG`), on a runtime handle already gone (`AK_STATUS_HANDLE_STALE`),
or on a handle range for channels exhausted (`AK_STATUS_INTERNAL`). None of the three
fails the runtime, and the first two must not: a typo in an endpoint cannot be allowed
to kill the process-wide runtime and every other channel it serves. So the model carries
a rollback, `RejectChannelCreation`: the constructor's own provisional state is freed -
not the runtime root, which outlives every channel and is released only after
`ak_runtime_destroy` returns - and the constructor faults with a configuration error,
leaving the runtime exactly as it was. Its lifetime is the caller's and no business of a
channel that failed to open. The same principle governs a refused `ak_call_start`: the ABI promises no
callback for a call that failed to start, so the binding frees the `GCHandle` it prepared
instead of waiting for a terminal that will never come.

**The reader ends.** A consumed terminal leaves the reader finished, and every later
`MoveNext` answers false at once, as `IAsyncStreamReader` requires - it never waits for
anything, and nothing in the model represents such a call because it touches no state.
A finished reader holds nothing, so the drain takes the ring from it as it would from an
idle one; only a parse in flight or a suspended `MoveNext` makes the drain wait.
A finished reader reports the call's terminal result, which is not always the value
`false`: a call that ended in error keeps producing the same `RpcException`, and
`finished` names the stable terminal outcome rather than one particular answer.

**The reader, precisely.** `BeginMoveNext` commits the read whether or not a payload
exists; `BeginParseEvent` wakes it when a payload arrives and only while the call is still
active - a dispose that linearized first wins the race, and the waiter then resolves
through `CancelWaiter`, never parsing a ring the drain already claimed.
`FinishConsumePayload` conjoins `L1!HostConsumesEvent` when the parse completes, and it
is where the status is resolved if the slot it just decoded was the terminal one.
`ConsumeHeader` takes the head for the task a caller's request started (`AskHeaders`) or
for a read in flight, the phase arbitrating, and nothing takes it while neither exists.
A one-response call's read waits for the whole response - the terminal in the ring or
the delivery window full, `ReaderWakes` - and the wait is in the fairness rather than in
`Next`: `ReaderTakesHead` and `ReaderParses` are the read's steps under it, because within
a pass the read goes on past its wake.  A batch needs no state of its own: the status is
always a batch's last event, so `OnEventReturns` is also the engine's return between two
events, and the root's release stays at the batch's own return.

**The read's token, precisely - and who wins the race.** `MoveNext(ct)`'s cancellation
is three distinct things, and keeping them apart is what makes the path correct.
`RequestReadCancellation` is the *firing*: the environment's step, carrying no fairness,
because a token that never fires is the normal case and no promise may turn a possibility
into an obligation.  `CancelWaitingRead` and `CancelParsingRead` are the binding's
*reactions*: owed, and each cancels **the call** - which is what
`IAsyncStreamReader<T>.MoveNext(CancellationToken)` means - and moves it to its drain in
the same step, so settling needs no further act from the application.

The race that matters is between a request and the read finishing normally. A request
that linearized before the end of the read must win: otherwise the parse completes, the
flag is cleared, and a cancellation the application asked for is silently lost.
`ReadCancellationSettled` is the rule - while the call is still `active`, a posted request
blocks the normal completion, so only a reaction may discharge it. Once the call has left
`active`, something else already cancelled it or it had already ended, and the request has
nothing left to obtain; the completion may then carry it away.
`LiveRequestOnlyDischargedByReaction` states exactly this as an action theorem, and it is
the half no liveness property on the flag alone can express: a flag going away proves
nothing, since any step clearing it satisfies such a property.
`PendingReadCancellationEventuallyObserved` therefore targets the effect - the call has
left `active`, and either it was cancelled or it had already ended.

The other direction is the late token, and it is inert by construction: a request is
armed only on a read in flight (`ReadCancelPendingOnlyInFlight`), so a token firing after
its own read completed finds nothing to arm.  **This abstraction is not free, and the
implementation owes it a rule.** The identity of the operation is the flag's lifetime
rather than an epoch, which is sound only if the previous registration's callback cannot
still run after the next read is published. So the normative discipline is:
`await reg.DisposeAsync()` before publishing the reader back to `idle`/`finished` - it
returns only once no callback of that registration is running or ever will, which is
precisely the guarantee needed. `Unregister()` does **not** suffice: unlike the dispose it
does not wait for an executing callback. Nor does the synchronous `Dispose()`, whose
return cannot be awaited and which blocks the thread while it waits, so it must not be
called from the async path at all. The disarm must happen outside any lock the callback
itself could need, or waiting for that callback deadlocks against it. An implementation
that cannot honour this must instead carry a read id captured by the callback and refuse a
notification that no longer matches the current operation - and then that id belongs in
the model, because the flag alone would attribute a stale callback to the wrong read.

**A cancelled parse keeps its slot, and still decodes the terminal.** A synchronous
marshaller already writing cannot be preempted, so `CancelParsingRead` moves the reader to
`parsing_cancelled` rather than abandoning the payload it owns, and
`FinishCancelledParse` is the single point where that slot is released -
`CancelledParseReleasesItsSlotOnce` says no other step may advance this call's tail while
that parse is outstanding, and that the one that does moves it by exactly one.  When the
slot it held was the terminal one, the status is decoded and kept all the same: the read's
own result is exceptional because its token won, but `GetStatus`, the drain and the
settlement all need that status, and once the slot is released no other consumer can
decode it.  Leaving it undecoded would strand the call - the drain would find an empty
ring and the dispose would wait forever on a status nobody can produce.

**The writer, precisely.** `WriteLendSucceeds` enters `serializing`;
`WriteRefusedBudget` enters the cancellable wait, `RetryLendSucceeds` leaves it,
`CancelWriterWait` resolves it on cancellation or dispose; `WriteRefusedTooLarge` faults
without waiting; `CommitWrite` sends and enters `awaiting_write_done`, or `sealing` on a
one-request call; `WriteAborted` is the disposable wrapper closing over a throwing
marshaller or a refused commit; `WriteResizesBuffer` is a marshaller that needs another
length than it announced, the buffer exchanged while the writer stays `serializing` - level
1's `ResizeSendBuffer`, whose refusals are no step, the wrapper then giving the buffer back
and lending again at the length it asked for; `WriteDoneCompletes` conjoins the level-1 callback return
and completes the write.  A one-request commit is one downcall in the engine and two steps
here: `SealRequest` ends the sending and closes the writer, reading no binding guard, and
while the writer seals the runtime steps that would close the end of the sending's guard
wait - a status arriving, a message past the second threshold, and the acquittal, which
would let a cancellation end the call between the two.  The engine then takes the
acquittal and its return, `PassWriteDoneReturns`, with no callback, and `CloseWriter` and
`WriteDoneCompletes` refuse such a call.
There is no slot wait: with completion at WRITE_DONE and one writer per call, the next
lend always finds the window open, which `ManagedWriterNeverObservesSlotBusy` states.

**Fairness comes in three tiers, and the tiers are the point of the level.** No
conjunct anywhere is stated over level 1's tuple: all forty-four are `WF_vars` on
actions of this module, which is what makes level 1's twenty-one families *earned*
rather than restated.
- *Runtime-owed* (`RuntimeOwedFairness`), sixteen conjuncts: one named passthrough
  per level-1 family the runtime and the FFI dispatch owe - `PassNetworkSend`,
  `PassDeliverStatus`, `PassEmitWriteDone`, `PassEmitBudgetWake` and the rest, each of
  them the level-1 action beside a managed stutter - and `PassWriteDoneReturns`, the
  engine's return of a one-request call's acquittal.  One is the exception to the tier's
  name: `PassEngineGivesBackAllBytes` is the runtime's step but carries level 1's assumption
  that the application's compressed sending stops.  The transfer is one for one: a
  projection lemma says the level-2 step is the level-1 step, and PTL turns the pair into
  the level-1 weak fairness.  Two wait while a one-request commit seals, the status's and
  the acquittal's, so their transfer goes through `SealingPasses`: a call seals once at
  most and the seal ends.
- *Binding-owed* (`BindingOwedFairness`), twenty-four conjuncts: the binding's own
  machinery, and nothing else.  The four callback returns discharge level 1's four
  trampoline families - `DeliveryReturns` is a disjunction because the terminal one
  frees the call root as it goes, and the two split on `HasStatus` so the disjunction
  is enabled exactly when level 1's family is.  The rest is the level's own: the
  headers task takes the head once a caller asked, a woken read takes the head or
  parses, a one-request commit ends its sending, the waiter resolves, the hand-off
  happens, both dispose chains and the whole teardown complete, the constructor answers.
  Every conjunct here waits on the binding's code, on the thread pool, or on a downcall
  that cannot block; none waits on the application.
- *Application-owed* (`ApplicationOwedFairness`), four conjuncts, and the whole of
  what a conforming program owes: read the stream (`BeginMoveNext`), and let the code
  it handed us come back - the parse returns (`FinishConsumePayload`), a cancelled
  parse returns (`FinishCancelledParse`), serialization settles
  (`SerializationSettles`, a disjunction because whether it commits or aborts is the
  marshaller's business while *that it settles* is the hypothesis).  The tier is the
  contract: a hypothesis about user code is stated where a reader looks for what the
  binding expects of its caller, not buried among the binding's own promises.

  `WF_vars(BeginMoveNext)` alone is the whole read contract.  A disposing call
  *disables* the action, and a weak fairness is satisfied by an action that stops
  being enabled just as well as by one that fires, so nothing has to be disjoined for
  the early-dispose case.  Nothing at all is asked once the terminal has been
  consumed: `Dispose` is **not** required for a normally finished call, which is what
  `Grpc.Core` says of its own `AsyncUnaryCall.Dispose` and its streaming siblings -
  there, disposing a completed call does nothing, and the method carries the meaning
  of *early cancellation*.  A model that demanded it would prove a discipline
  stricter than the API it implements.

  **A call therefore settles by itself.** `SettleCall` is binding-owned and weakly fair,
  and its guard is the end of the call read through level 1's own ownership predicates:
  the terminal delivered and consumed, the reader finished, the writer idle or closed, the
  status resolved, and - the hinge - `L1!HostOwnsNoPayload` and `L1!HostHoldsNoBuffer`.
  Those last two are exactly what `L1!ReleaseCallHandle` waits on, so the managed
  settlement is the condition that unblocks the native reclamation rather than a parallel
  state ignoring it. `SettledCallOwesNothing` states the link, and the reclamation itself
  is not restated here: `L1!CallEventuallyReclaimed` promises it, `L1!ReleasedCallIsClean`
  describes what it leaves behind, and level 2 inherits both through the refinement.
  `BeginDisposeCall` remains, as the early-cancellation path it is in the API, with no
  fairness demanding that it ever occur.

  One case stays covered by hypothesis, legitimately: an application that abandons a
  readable stream, neither reading nor disposing. Its call never settles and its channel
  never releases - a misuse, and the same one level 1 already assumes away with
  `WF(HostConsumesEvent)`.  Nothing else is asked of the caller.  In particular a write
  left waiting on the send budget when the server ends the call is settled by that
  terminal, through `BudgetWaitEndsWhenHopeless`, and not by waiting for a `Dispose` the
  API does not require.
  Beside that one conjunct sit two conformity hypotheses of
  safety, not progression: `MoveNext` calls are serialized (`IAsyncStreamReader`) and
  writes are serialized (`IClientStreamWriter`), both encoded by the single
  `reader_state` and `writer_state` values.  Nothing else is asked: not feeding the
  request stream (sends are triggers, never owed), not completing it, not any cadence.

#### Level-2 safety invariants (to be proved by TLAPS)

Every name below is a conjunct of `ManagedSafety` in `DotNetBinding_defs.tla`, and
`ci/check_property_manifest.py` refuses this list and that conjunction
diverging in either direction.  `ManagedTypeOK` is also a conjunct, structural like
`TypeOK` in level 0's `SafetyCore`, with its own public theorem.

- **TokenPublishedBeforeStart**: no used call without its token - born in the same step
  as `ak_call_start`
- **RootSurvivesCallbacks**: a delivery or WRITE_DONE callback in flight resolves its
  `call_ctx` to a live root - freed by the terminal callback's own return, its last
  access
- **RuntimeRootSurvivesCallbacks**: every callback of every kind resolves `runtime_ctx`
  to a live root - the call callbacks included, freed only after `ak_runtime_destroy`
  returned
- **ConsumerPhaseMatchesDispose**: the phase machine and the call's dispose machine never
  disagree
- **AtMostOneReaderOutstanding**: an outstanding read exists only while a consumer of the
  application's own is on the ring - the prologue included, since `MoveNext` may be the
  first thing an application calls and the wait for the metadata is part of that read.  One
  value per call is what makes it unique.  Stated over `ReadInFlight`, so a cancelled parse
  counts too - it still holds a slot
- **DrainNeverOverlapsApplicationConsumer**: the drain never runs beside an application
  read, suspended or parsing
- **WaitingWriterHoldsNoBuffer**: a write waiting on the budget holds no lent buffer -
  waiting for capacity while holding capacity is the deadlock level 1 cannot see
- **SerializingWriterHoldsTheBuffer**: exactly the serializing state holds a buffer, and
  it holds one - the lend/return discipline as an equation
- **WaitMatchesRefusal**: a waiting write's last lend result is BUDGET_BUSY
- **ManagedWriterNeverObservesSlotBusy**: no lend of this binding is ever refused for a
  slot - the consequence of completing at WRITE_DONE with a single writer, stated so a
  defect would show
- **RetryLenMatchesWait**: the remembered length exists exactly while the wait does
- **DisposeAwaitsDestroy**: the teardown reaching `destroyed` means `ak_runtime_destroy`
  returned for the **current** generation
- **RuntimeStateMatchesNative**: the manager and the native runtime agree, through a
  table - `AdmissibleRuntimeStates` says which native states each manager state admits
  for the generation it names, its `destroy` has returned exactly when the manager says
  `destroyed`, and every other slot is idle.  The table is total: an unknown manager
  state admits nothing, so a sixth state added later breaks a preservation step rather
  than reading as an unspecified value.  Failure is admitted at every stop, once, being
  nobody's step
- **RuntimeManagerCoherent**: a materialized generation has an identity and a root, an
  absent one has neither - the manager never claims a runtime it does not hold
- **LiveChannelUsesCurrentRuntime**: a live channel hangs off the current generation,
  never an earlier destroyed one
- **ManagedShutdownHasNoHostDebt**: the shutdown never owes the second event. Every call
  is settled before its channel releases, and every channel releases before the
  teardown, so `SHUTDOWN_COMPLETE` finds no host debt and
  `AK_EVENT_RESOURCES_RELEASED` is unreachable at this level - the level-1 machinery
  stays modelled and passed through for the refinement
- **LiveChannelKeepsRuntimeAlive**: a live channel has an engine under it - the runtime
  is running, or running and sweeping, both of which admit the native `RUNNING` a
  downcall of that channel needs.  A channel is never left pointing at a torn-down
  runtime
- **NoRuntimeShutdownWhileChannelsLive**: the native shutdown never starts while a
  channel is still live - from the destroy on, the sweep is over and every channel is
  settled, which is "a channel cannot outlive its engine" stated where it can be checked
- **RejectedChannelHasNoNativeHalf**: a channel whose configuration was refused never
  got its native half - and is settled, which `ChannelSettled` covers
- **ReadCancelPendingOnlyInFlight**: a cancellation request is armed only on a read that
  is in flight.  That is what makes a late token inert: `MoveNext`'s registration dies
  with the read it belongs to, so a token firing after its own read completed has nothing
  to arm - the identity of the operation is the flag's lifetime rather than an epoch
- **ParsingReadOwnsItsSlot**: a parse owns its slot for as long as it lasts, cancelled or
  not.  A synchronous marshaller already writing cannot be preempted, so the reader holds
  the borrow until it returns and the release happens there - exactly once for a cancelled
  parse, which `CancelledParseReleasesItsSlotOnce` states on level 1's own consumption
  counter.  This is the lifetime the implementation must respect: native bytes are readable
  exactly while the reader is parsing, so a release moved before the read's result is
  decided violates it, and nothing in the actions' shape alone would say so
- **ChannelStateMatchesNative**: the channel machine and the native channel agree - no
  managed channel active without its `ak_channel`, none exposed before it
- **DisposeLeavesNoManagedWaiter**: a settled call has its reader idle, its writer
  settled, its headers resolved and its status resolved
- **AbsentRuntimeOwesNothing**: whenever the runtime is back to `absent` and every
  channel is settled, nothing is owed - no live generation root, no published call left
  unsettled, and `DisposeLeavesNoManagedWaiter` carries the rest.  The antecedent is
  deliberately that weak: it holds at the initial state and between generations, not only
  once the channel set has been used up.  That is what makes the terminal state of a
  configuration whose finite channel set IS spent recognizable as quiescence rather than a
  stall, which is why `DotNetBinding_MC` states `CHECK_DEADLOCK FALSE` and this invariant
  carries the content instead
- **SettledCallOwesNothing**: a settled call owes level 1 nothing - no payload, no lent
  buffer - which is exactly what `L1!ReleaseCallHandle` waits on, so the managed
  settlement is what unblocks the native reclamation.  Its other half is inherited:
  `L1!CallEventuallyReclaimed` promises the reclamation, `L1!ReleasedCallIsClean` says what
  it leaves behind
- **RingNeverOverflows**: `RingHead - RingTail <= DeliveryCredits + 1`.  Inherited, not
  re-proved: level 1's `PayloadsOwnedWithinCreditsPlusOne` read through the derived
  indexes, which is what lets the trampoline publish without a fullness test

#### Level-2 liveness (conditional on fairness)

The conjuncts of `ManagedLiveness` in `DotNetBinding_defs.tla`, bound by the same
checker.  Every promise crossing the native runtime carries the `~NotFailed` escape,
like every level-0 and level-1 promise: a failed runtime is the contract's one admitted
way out, and no termination is guaranteed past a failure.

- **BudgetWaitEndsWhenHopeless**: a write waiting on the budget stops waiting as soon as
  no lend of it could ever succeed - the call cancelled, the call no longer active, the
  call disposing, or the runtime gone.  The terminal belongs in that list and is the reason
  it is not simply "cancellation": once the server has ended the call, no budget signal can
  lead to a lend, so the task has to be faulted by the binding.  Leaving it out made the
  wait resolve only when the application disposed, which is a hidden obligation on the
  caller and not a guarantee - the more so since `Dispose` is optional on a call that has
  already finished.  The conditional shape is the whole property: the
  deadline is optional, cancellation may never come, and acquisition is deliberately not
  guaranteed since another call can always win the capacity - a behaviour polling
  forever is admitted.  Promising acquisition would need an arbitration the ABI does not
  have (a FIFO of waiters), and a fair `ak_get_call_buffer` cannot stay synchronous and
  non-blocking; that escalation stays available at no cost, since a poll remains correct
  once a signal exists
- **PendingWriteEventuallySettled**: a write that reached the buffer settles - it
  commits or aborts, and a committed one completes at its WRITE_DONE, which level 1
  guarantees before the terminal, or on a one-request call at its end of the sending
- **ChannelConstructionCompletes**: a constructor that began completes, one way or the
  other - the channel is exposed, or its configuration was refused and it ends in
  `rejected` holding nothing.  A rejection is a terminal outcome and a completion,
  not a stall, which is why the fairness is on the disjunction of the two issues: what is
  owed is a result, not a success
- **CallDisposeCompletes**: a draining call settles - the drain reaches the terminal,
  releases everything and resolves the status
- **ChannelHandleEventuallyReleased**: a disposing channel gives its native half back -
  its calls settled, then `ak_channel_release`
- **ChannelDisposeCompletes**: the public `DisposeAsync` task completes with the
  channel's own release, which is all it ever waited for: the engine is the caller's
  object and outlives every channel made from it, so nothing a channel's dispose
  promised depends on a destroy
- **RuntimeDisposeCompletes**: a disposal that began completes - the sweep disposes every
  channel the runtime holds, then the destroy, then the root, then `absent`.  It is the
  one promise the sweep is load-bearing for, and it rests on the per-channel promise
  above joined over the finite `ChannelIds`
- **CallRootEventuallyFreed / RuntimeRootEventuallyFreed**: every allocated root dies -
  the call's at its terminal callback, the generation's after destroy
- **InFlightPayloadEventuallyReleased**: a parse completes and its slot is released -
  under the stated hypothesis that user parsing terminates.  Both states that own a slot
  are covered, `parsing` and `parsing_cancelled`: a cancelled parse holds its payload
  exactly as a live one does
- **PendingReadCancellationEventuallyObserved**: a request that landed is acted on.  The
  token's firing carries no fairness - a token that never fires is the normal case, and no
  promise may turn a possibility into an obligation - but the binding's reaction to one
  that did is owed.  The target is the effect rather than the flag's disappearance,
  because only the effect is the promise: the call has left `active`, and either it was
  cancelled or it had already ended
- **CancelledReadEventuallyDrainsCall**: a cancelled read leaves the call on its way out,
  with no further user action.  `MoveNext`'s token cancels the call, so the binding drains
  it: the application does not have to read again or dispose to see it settle
- **ReadInFlightEventuallyResolved**: a read in flight - suspended or parsing - always
  resolves: by its payload, by its own token, or by the dispose.  `MoveNext(ct)` is
  modelled with the contract's two halves, each an action theorem: cancelling a read
  still in flight cancels **the call** (`ReadCancellationCancelsCall`), and a read that
  already completed has an inert token, no step cancelling on its behalf
  (`CompletedReadTokenArmsNothing`)
- **WaitingReaderEventuallyResolved**: a suspended `MoveNext` is resolved by payload or
  dispose, never abandoned
- **PublishedCallEventuallySettled**: every call the application created settles in the
  end - by `SettleCall` when it finishes normally, by the drain when it is disposed
  early. It is the binding's guarantee, not a user obligation: the only hypothesis it
  needs is that a readable stream is eventually read.  A physical system with an unbounded stream
  keeps the norm without the theorem
- **HeadersEventuallyResolved / StatusEventuallyResolved**: the public completions are
  never left pending - the headers a caller asked for are answered whether or not it
  reads, by the head's consumer or the dispose, and the terminal consumer resolves the
  status

#### Held by construction, not stated as invariants

Like level 0's monotone transitions: true of every behaviour, by the shape of the
actions rather than by induction, and this document must not imply a theorem exists.

- **ContinuationsAsync**: no completion runs a continuation on the callback's thread.
  In the model, no action both returns a callback and performs an application step; in
  the implementation, every TCS is `RunContinuationsAsynchronously`, the signals' included
  - an implementation rule verified by review and tests, deliberately not
  restated as a counter the model would prove things about.  With no dispatcher between
  the trampoline and the application this is the only thing keeping user code off the
  Tokio thread, and it is what makes the callback-return fairness the binding's to
  promise
- **MessageTooLargeIsNotRetried**: no action enters a wait from MESSAGE_TOO_LARGE - the
  refusal is permanent by construction, so a retry would poll against a constant.  The
  absence is the mechanism
- **PayloadsReleasedInOrder / ReleasedAtMostOnce**: the release counter can only advance
  by one, so the order is the data structure and a double release cannot be expressed.
  This is the price level 1's counter abstraction charged, paid by representation - and
  the hand-off preserves the tail for the same reason: `HandoffToDrain` touches no
  level-1 state, and `ConsumerHandoffPreservesTail` states it as a public theorem
- **SingleStreamConsumer / SingleStreamWriter**: one `reader_state` and one
  `writer_state` per call, every consuming or writing action guarded by them - two
  concurrent `MoveNext`, or two concurrent `WriteAsync`, are unrepresentable.  The
  representation encodes what the API requires of the caller: the serialization is the
  application's conformity hypothesis, not a guarantee the binding manufactures
- **NoDowncallAfterDestroy**: the call downcalls carry `BindingMayDowncall`, the channel
  downcalls their own channel-machine guards, and `BeginRuntimeShutdown` requires every
  channel settled - an ordering on dispose, not a safety net.  Level 1's
  `DestroyedRuntimeRejectsHandles` is the runtime's side of the same fact
- **The second event's unreachability is an invariant, not a construction**:
  `ManagedShutdownHasNoHostDebt` states it in the manifest and carries a theorem, because
  a passing model-checking run is not an argument about an action claimed unreachable
- **BuffersAlwaysReturned / ReleasedEventually**: not invariants but fairness conjuncts -
  the disposable wrapper's WF and the begin-or-dispose WF respectively
- **RetainedBytesAreEventuallyFreed** stays a native-Rust obligation: the retention is
  the runtime's own decision, no managed code observes it, and nothing level 2 models
  can discharge it.  It is the one place where the implementation is deliberately slower
  than the model rather than the reverse

The public interface, `DotNetBindingTheorems.tla`, declares the obligations the freeze
requires discharged - `RefinesInit`/`RefinesNext`/`RefinesSpec`, the six host families,
`ManagedTypeOKHolds`, `ManagedSafetyHolds`, six action theorems -
`ConsumerHandoffPreservesTail`, `ChannelDisposeIsolatesItsCalls`,
`ReadCancellationCancelsCall`, `CompletedReadTokenArmsNothing`,
`LiveRequestOnlyDischargedByReaction` and `CancelledParseReleasesItsSlotOnce` - and one
theorem per liveness promise plus their aggregate.  The ordering a channel's dispose
used to owe a destroy is not among them any more, and not because it stopped mattering:
with the runtime's lifetime declared rather than derived, "a channel cannot outlive its
engine" is a fact about every state past the destroy rather than about the one step that
decided it, so it is `NoRuntimeShutdownWhileChannelsLive` in the manifest.

The six host families - the four callback returns, `HostConsumesEvent` and
`HostReturnsBuffer`, each as a weak fairness over level 1's own tuple - are corollaries,
not assumptions.  This level states none of them: `RefinesSpec` gives `L1!Spec`,
`L1!Spec` gives `L1!Fairness`, and each family is one of its conjuncts.  The distinction
is the whole content of the tier design.  A binding may declare these six obligations
and satisfy them by restating them in level 1's vocabulary, which proves nothing at all
- the level would be assuming what it claims to earn - or it may state its fairness on
its own actions and let the families fall out.  This one does the second, which is why
`RuntimeOwedFairness` carries fifteen named passthroughs rather than fifteen citations.

**The refinement is closed.**  `RefinesInit`, `RefinesNext` - one projection lemma per
disjunct of `Next` - `FairnessRefines`, `ManagedIndInvHolds`, `ManagedSafetyHolds`,
`DerivedInvariantsHold` and `RefinesSpec`, which makes every theorem level 1 proved
about itself a theorem about this level.  Level 1's fifteen liveness properties come
back through it in one citation, `InheritedLiveness`, because level 1's variables *are*
these variables: the state module is extended, not instantiated, so nothing needs
translating but the prefix.

Three facts the refinement's proof established rather than assumed.  `RefinesNext` is
stated relative to `ManagedSafety`, unlike level 1's own, and the reason is that two
coupled actions witness a level-1 existential with managed state - `CreateChannel`
passes `current_runtime`, `RetryLendSucceeds` passes `retry_len[cId]` - and that those
values lie in the sets level 1 quantifies over is an invariant, not a syntactic fact.
The same proof tightened `retry_len`'s typing from `RequestLengths` to `Sizes`: only a
budget refusal parks a length, and one is only ever pronounced on a length the window
admits.  And the fairness transfer is the level's own content: `HostConsumesEvent` is
the one family no passthrough carries, so it is earned through seven leads-to edges
ending at the application's single obligation - the binding's machinery plus that one
hypothesis implies level 1's family.  On a one-response call one more edge is needed,
because the read waits for the whole response: `AsleepReaderIsWoken` shows from the
runtime's fairness alone, with no failure escape, that a read cannot stay asleep - that
would be a status never received, a send never acquitted, the one message the engine may
hold never delivered, or a terminal never delivered, each a weak fairness broken.  The
window has room throughout, being the asleep read's own condition, so no consumption is
needed for any of it.

**The managed liveness is proved.**  All seventeen promises hold, and `ManagedLivenessTheorem`
collects them.  What the argument cost is a second family of invariants, below.

#### The derived invariants

Fifteen invariants carry the liveness argument and appear in no manifest, because none of
them is a guarantee the library offers: each is a fact about the machine that the safety
proof never needed and a leads-to edge cannot do without.  Five theorems carry them -
`DerivedInvariantsHold`, `DrainInvariantsHold`, `ReaderGlueHolds`,
`StatusResolutionHolds` and `ServedRootsHold` - each of the shape
`Spec => []Inv`, and all fifteen are defined in `DotNetBinding.tla` beside the published
ones.  The manifest checker cannot see them, since it binds this document to the
manifests; `ci/check_derived_invariants.py` binds them to this table instead, by reading
that shape rather than a list.  It exists because this list was written from one theorem
and named seven of them on the day it was added.

| Invariant | What it says | Promises that rest on it |
|-----------|--------------|--------------------------|
| `PrologueHasReleasedNothing` | a reader still in its prologue has consumed nothing, so its ring tail is zero | the call dispose, the headers, the status, both read resolutions |
| `FinishedReaderDrainedTheRing` | a finished reader has the status and an empty ring | the call dispose, the status |
| `LiveCallHasLiveChannel` | a published, unsettled call has a channel, and that channel is active or disposing | seven of the seventeen, the payload release included |
| `BusyWriterIsOnAStartedCall` | a writer that is not idle is on a call that exists | the hopeless budget wait |
| `AwaitingWriteDoneHasOneComing` | a writer waiting on its acquittal is a stream's, and has a send in flight or the callback on the stack | the pending write |
| `SerializingWriterHoldsANamedBuffer` | a serializing writer holds a buffer that can be named | the pending write, through `SerializationHasAnExit` |
| `PublishedCallHasStarted` | a call whose token is published exists at level 0 | seven of the seventeen, the call root's release included |
| `DrainingCallIsCancelled` | a draining call exists and is either cancel-requested or already past active | the call dispose, both read resolutions, the cancelled drain |
| `InFlightReaderHoldsTheToken` | a read in flight is on a call whose token is published | the read resolutions and the cancellation observation |
| `ConsumedTerminalFinishesTheReader` | an active call whose ring is drained past its first slot has a finished reader | the read resolutions, the settlement |
| `StatusResolvedOnceTheRingIsDrained` | a drained ring past its first slot means the status is resolved | the status, the call dispose |
| `LiveRootIsServed` | a live call root has its token published, and past the status a delivery callback is running | the call root's release |
| `OneSendInFlight` | one send at most is in flight, none while the writer is idle, serializing or waiting, and a running WRITE_DONE acquits the last | the consumption on a one-response call, through the wake |
| `SealingHoldsTheEndOfSending` | a sealing writer's call is started or sending, its sending open, no status pending, its handle held and its acquittal not yet emitted | the pending write, and the status's and the acquittal's fairness |
| `MetadataLeads` | a call's first event is its initial metadata, past a failure too | the consumption on a one-response call, through the wake |

Two of the fifteen were forced by the send side and are worth naming for what they rule
out.  `AwaitingWriteDoneHasOneComing` is what makes the wait end without any hypothesis on
the application: one of the two disjuncts is always the enabled step.  And it is not enough
on its own - `TrampolineStaysUntilItReturns` says the callback cannot slip off the stack
while the writer waits, without which a single callback is not the standing enabling weak
fairness asks for.

With the engine's bytes in the model, the three proofs modules they reach verify by windows
with the fingerprint cache enabled: `FfiGrpcTheorems_proofs` 13803 obligations,
`FfiGrpcEnabledTheorems_proofs` 23 and `DotNetBindingTheorems_proofs` 34508, no failure, each
summed over windows, which may count an obligation twice at a boundary.
`AbstractGrpcTheorems_proofs`, which they do not touch, verifies with the cache disabled at
1809.  The cache matters here, because a green run over a
warm cache says only that the obligations were once discharged by a text that may since
have changed.

Refinement mapping, by direct reuse:
- `NativeRuntime.Create(...)` ↔ `CreateRuntime` - the shared root and
  `ak_runtime_create_from` or `ak_runtime_create`, and nothing of any channel
- `runtime.Channel(...)` ↔ `BeginCreateChannel` then `CreateChannel` - the door read
  under the lock, then this channel's `ak_channel_create`, the constructor returning
  only afterwards
- the call constructor ↔ `StartCall` - `GCHandle.Alloc`, `ak_call_start` and exposure in
  one step
- `OnEvent` publishes a slot ↔ `OnEventReturns`; the terminal one is
  `TerminalCallbackReturns`, which frees the call root and decodes nothing.  A batch's
  returns before its last are the engine's own, `OnEventReturns` too, and
  `ak_events_consumed` is one `FinishConsumePayload` - or `ConsumeHeader`, or
  `DrainRelease` - per payload, in order
- `ResponseHeadersAsync` ↔ `AskHeaders`, the first time
- `MoveNext`, its suspension and its parse ↔ `BeginMoveNext`, `BeginParseEvent`,
  `FinishConsumePayload` - the last conjoining `L1!HostConsumesEvent` and resolving the
  status when the slot it decoded was the terminal
- **a decode that throws** ↔ the same actions, with no state of its own. The level does not
  record *what* a decode produced, only that the slot was acquitted and the reader
  republished, so a marshaller that throws refines `FinishConsumePayload` exactly as one
  that returns - which is why the release must happen on both paths. A failing message
  decode is then followed by `RequestCallCancellation` and the drain, the read reporting the
  exception; a failing terminal decode still resolves `status_completion`, from the synthetic
  status, because that conjunct of `FinishConsumePayload` is what the settlement depends on.
  If the token won instead, the same slot is acquitted by `FinishCancelledParse`, which
  resolves the status too. No `decode_failure` state is needed at this level, and adding one
  would record a value the level has no use for
- `WriteAsync` ↔ `WriteLendSucceeds` or a refusal, `WriteResizesBuffer` as often as the
  serializer outgrows its buffer, `CommitWrite` or `WriteAborted`,
  then `WriteDoneCompletes`; `CompleteAsync` ↔ `CloseWriter`.  On a one-request call the
  commit is `CommitWrite` then `SealRequest`, one downcall, and the engine's
  `PassWriteDoneReturns` follows
- `Dispose` on a call ↔ `BeginDisposeCall`, `CancelWaiter` for a suspended read,
  `HandoffToDrain` behind any parse in flight, `DrainRelease` until drained, then
  `FinishDisposeCall`
- `DisposeAsync` on the channel ↔ `BeginDisposeChannel`, `DisposeCallForChannel` per
  owned call, `FinishDisposeChannel` for `ak_channel_release`, then
  `ResolveChannelDispose` at once: the task ends at the channel's own release and waits
  for nothing else
- `DisposeAsync` on the runtime ↔ `BeginDisposeRuntime`, which shuts the door,
  `DisposeChannelForRuntime` per channel it still holds - each of which runs the
  channel's own disposal above - then `BeginRuntimeShutdown` once every channel is
  settled, `FinishDisposeRuntime` for the destroy, and `FreeRuntimeRoot`

Level 2 re-proves none of the window reasoning: with `Spec => L1!Spec` established the
same way level 1 established `Spec => L0!Spec`, the send bound, the credit bound, the
per-object liveness, the end-of-call cleanliness and both no-double-free invariants are
inherited rather than restated.

#### One theorem statement may not write ENABLED

A level that instantiates another cannot reach a theorem whose statement *writes*
`ENABLED`. TLAPS normalizes an instantiated module's theorem statements eagerly, and the
operator its `ENABLED` elimination introduces belongs to the module where the `ENABLED`
is written; under substitution the prover looks for it in the importing module and aborts
outright rather than failing an obligation. Three facts bound the rule exactly, each
measured rather than assumed: a statement whose `WF` is literal instantiates cleanly - a
`WF` is a definition's body once expanded, and `PTL` reads it; a statement that merely
names a formula instantiates cleanly, which is the idiom the sibling refinement chains in
`ArmoniK.Spec` use (`RefineTaskProcessing2 == TP2!Spec`); and `EXTENDS` never triggers it,
substituting nothing.

Level 1 has exactly three such statements, the refusals' conditional enabledness, and
they live apart in `FfiGrpcEnabledTheorems` with their own proofs module - a sibling of
`FfiGrpcTheorems`, both extending `FfiGrpc_defs`, so instantiating either drags nothing
of the other. Level 2 therefore instantiates `FfiGrpcTheorems` and has level 1's
definitions *and* theorems as facts; it never needs the three, which speak of refusal
actions the managed writer does not realize. Whoever adds a theorem to a level that a
later one instantiates should keep its statement free of a written `ENABLED`, or name the
formula.

#### The implementation's risk register

The frontier below says what is not proved and what verifies it instead. This says what
is likely to go *wrong* while writing the code, and what gate catches it. The two are
different questions: a subject can be perfectly specified and still be implemented with a
race. Ordered by what a defect would cost.

| Risk | What it produces | Gate |
|------|------------------|------|
| The write claim or writer state published after the downcall | an immediate WRITE_DONE finds nothing to complete: the write hangs, or a later one is completed twice | publish before the native call, roll back only on synchronous refusal; a test whose callback fires before the downcall returns |
| The slot's ownership: a read taking the ring against the drain's handoff | two consumers believing they hold the same tail, so a payload acquitted twice or a drain parsing bytes already returned | the transition is what confers ownership, never a peek - `BeginParseEvent` against `HandoffToDrain`, one of which wins; `ParsingReadOwnsItsSlot` states the borrow's lifetime |
| The read's result: success against this read's token | a call believed cancelled that continues, or a value published after the token won | one CompareExchange per read, decided after the disarm and before the release - a different winner and a different point from the race above, which is why they are listed apart; `ReadCancellationCancelsCall` and `CompletedReadTokenArmsNothing` |
| Serializer running while the call is disposed | the buffer returned under a marshaller still writing into it - use after return | one owner for the wrapper, commit and abort atomic and exclusive, returned exactly once |
| GCHandle on a refused start, or a terminal arriving at once | a root leaked, or freed twice | root before the start; local rollback if the start refuses (no callback is promised); after acceptance the terminal callback is the only releaser |
| A channel created beside the runtime's disposal | a channel the sweep misses, whose handle nobody closes | one lock orders the creation and the sweep, so a creation is either swept or refused; a concurrent create/dispose test |
| The ring's memory ordering | a slot published half-visible, a lost wake-up - and only on ARM64 | documented `Volatile`/acquire-release pairs, padding, an ARM64 stress test, a wait taken before the look it follows |
| An exception crossing the reverse P/Invoke into the engine | undefined on the engine's side, which it would unwind through | catch-all at the trampoline, no user code inside it, and a `finally` that gives back the payload and the call's root a throw would have kept |
| A continuation running inline on the callback thread | arbitrary reentrancy, the Tokio thread blocked by user code | `RunContinuationsAsynchronously` everywhere, signals never inline, a test capturing the thread identity |
| Dispose called twice or concurrently | a double cancel, two drains, or two different tasks for one dispose | decide idempotence and share one completion; a test with N concurrent calls |
| A cancellation registration or timer outliving the terminal | a stale downcall, a root held, operational noise | disarm atomically at the terminal; the callback tolerates a stale handle |
| A read's registration callback running after the next read is published | the cancellation is attributed to the wrong read, and the model's flag-lifetime identity is false | `await reg.DisposeAsync()` before republishing the reader and before deciding the result, outside any lock the callback needs - neither `Unregister()` nor the synchronous `Dispose()` fits; failing that, a read id in the model |
| An arbitrary marshaller that allocates, throws, or keeps the sequence | the zero-copy claim overstated, a lifetime violated | generated fast path plus a copying fallback; a stated lifetime contract; exception and retention tests |
| A budget wake-up with no arbitration | one refused send overtaken by others at every release, admitted starvation | every refused call woken at each release, the host's obligation to try again or cancel, a wait level 2 promises cancellable and nothing more; metrics on refusals and waiting time |
| Calls admitted together past the first threshold | the count past `memory_ceiling` by a message per admitted call | `memory_hard_ceiling` ends the call whose message would pass it, `MemoryWithinHardCeiling` proved; size the gap between the two for the calls a process runs at once |
| Replay holding sent messages | memory the lending ceiling does not see | a separate budget, replay metrics, cancel and retry tests |
| Handle space exhaustion | a late refusal | the counter climbs past its range's end and every claim after it is refused, so one kind never spills into the next; tested on an artificially small space |
| `FAILED_UNQUIESCED` with no operational procedure | a process durably degraded, memory unrecoverable | an alert, a debt dump, a documented fail-fast or restart threshold |
| State tables in this document drifting from the modules | the model transcribed wrongly into the code | generate the tables from one source, or compare them in CI |

**The counters that make a violated hypothesis visible.** Several guarantees above rest on
the application behaving; production needs to see the breach before it becomes an opaque
leak. At minimum, in diagnostics: payloads owed, buffers lent, sends submitted and not
acquitted, callbacks in flight; the runtime's phase, generation and lease count; each
ring's head, tail and high-water mark; the number and duration of budget refusals, retries
and cancellations while waiting; stale handles refused and generation slots retired; the
longest callback; bytes held for replay past WRITE_DONE; and the debt `ak_call_debt_of`
reports at an abnormal teardown. None of these is a proof. Each is how a broken
conformance hypothesis is recognized while it is still cheap.

#### After a failed runtime, the binding still owes determinism

`AK_RUNTIME_FAILED_UNQUIESCED` is absorbing, and every promise above carries the
`~NotFailed` escape - the proofs stop there, legitimately. The binding may not, and what it
owes follows from where a failure arises. It arises only in a shutdown the caller started with
`DisposeAsync`, so new channels and calls are refused already. Each call's terminal comes from
that call's own task, not from the shutdown's, so the managed readers, writers and status still
resolve. And the disposal throws rather than waiting for a quiescence that will not come, which
leaves no `Task` without an outcome. Operationally the failure is terminal for the runtime: its
memory is unreclaimable while the process lives, so the policy is fail-fast with the debt
reported (`ak_call_debt_of`, the counters below) and a restart, not a silent degradation.

#### What no level of the specification covers

The models assume state updates are atomic and sequentially consistent; concrete memory
ordering, encodings and the code the models abstract on purpose sit outside that
assumption.  This is the closed frontier: the subjects below sit deliberately outside every
level, each with what verifies it instead. The list exists so that none of them is mistaken for a gap - a
proof obligation nobody wrote - and so that none is reopened as one. No further level of
refinement would help with any of them: they are properties of concrete memory, of
encodings, or of code the models abstract on purpose.

| Subject | Verified by |
|---------|-------------|
| **The ring's memory model** - `Volatile` pairing, acquire/release, false sharing | Code review and a race test, ARM64 included |
| **No inline continuation** - every TCS `RunContinuationsAsynchronously`, the signals' included | Review, plus a test that no user code runs on a callback thread |
| **Serialized bytes** - arbitrary `Marshaller<T>` round-trips | Protobuf round-trip tests |
| **Exact metadata, status and trailers**, and the .NET exception mapping | gRPC conformance tests |
| **The five `CallInvoker` shapes' cardinalities** - the model is the generic bidirectional call | A test per shape |
| **`CallOptions` in full** - credentials, headers; the deadline is the engine's timer, outside the model | Still declared missing work, not a hidden claim |
| **The runtime's lock** between a channel's creation and the disposal's sweep - level 2 takes both as atomic steps | Nothing directed: no test races a creation against the disposal |
| **The handles' concrete encoding** - widths, allocation, type discrimination | ABI header work and stale-handle tests |
| **Budget polling's cadence, backoff and starvation** | Nothing: deliberately not guaranteed, and the model says so |
| **Payload owner identity** - FIFO release is a conformance hypothesis | `ak_call_debt_of` in assertions and tests |
| **The write claim** around the send downcall | A directed test: a second write while one is in flight is refused |
| **A compilable C ABI**, layouts, versioning, protocol encodings, tri-language tests | The header and conformance phase of the binding plan |
| **The automatic retry** - attempts, backoff, replay buffer, retryable statuses | That requirement's own tests.  A policy over calls the models describe, not a mechanism of the protocol; the retry the models carry is the buffer lending one |

**The memory model is the price of the ring.** A missing `Volatile.Write` on `head`
produces a ring that violates everything proved above, and neither level 1 nor level 2
will catch it. The release/acquire pairing is a coding rule, and it belongs in review
rather than among the proof obligations, where listing it would suggest a coverage that
does not exist. It is the price of a zero-copy SPSC ring, and it is worth paying, but it
is worth naming.

Four of the twelve carry a decision the implementation must not improvise, so the
decision is here rather than in the code that will need it.

**Cancellation faults with `RpcException`.** A call disposed or cancelled before its
metadata arrives resolves `ResponseHeadersAsync` - and every other pending managed object
of that call - with an `RpcException` carrying `StatusCode.Cancelled`. One rule, one
exception type, whatever the path: no `ThrowOperationCanceledOnCancellation` option is
ported, so calling code stays in the `RpcException` world grpc-dotnet callers already
handle. The model states that nothing is left pending
(`DisposeLeavesNoManagedWaiter`); which exception carries the failure is this decision.

**There is no refcount, because nothing derives the runtime's lifetime.** It is an object
its caller creates and disposes, and what its lock guards is one thing: the set of
channels it made, so that a creation and a sweep cannot pass each other. It costs one
uncontended lock per channel construction and disposal, never anything on a hot path.

**The write TCS is published before the commit, and rolled back on refusal.** The
WRITE_DONE callback may run the moment `ak_call_send_message` accepts, so the TCS has to
be reachable from `CallState` before the downcall - a callback that finds nothing would
lose the completion the write is waiting on. If the commit is refused instead, the same
`using` scope that returns the buffer withdraws the TCS and faults it. Publishing after
the acceptance would need a landing slot for a callback that arrived early, which is more
machinery for the same guarantee.

**The handles' encoding is a property, not yet a layout.** Three properties must hold, and
the widths that carry them belong with the C header, where `ak_abi_version` and the struct
layouts are chosen and where the tri-language tests live:

- a stale handle is refused, never dereferenced - the generation counter is the
  implementation of the released and destroyed states the models carry, and nothing else;
- a live handle of the wrong kind is refused as an argument error rather than resolved
  against the wrong object: index spaces are per kind, so a channel's index is plausibly a
  live call's index and the generations of two spaces climb in parallel. Either a kind tag
  in the handle or a kind field in the slot discriminates them; the slot field is the
  cheaper of the two, and the choice belongs with the slot map's design;
- generation wraparound is impossible by construction, not merely improbable: a slot whose
  generation would saturate is retired instead of reused. One comparison at allocation
  buys a structural argument where a width alone would only buy a large number.

The arithmetic that will size them, recorded so it is not redone: an index space covers
*simultaneous* objects - a runtime, a handful of channels, the concurrent calls, and per
call at most `MaxSendsInFlight` buffers and `DeliveryCredits + 1` payload owners - while a
generation counts a slot's *reuses over the whole process lifetime*. Sustaining a hundred
thousand calls a second for ten years is some three times ten to the thirteenth calls; over
four thousand slots that is under two to the thirty-third reuses each. The index wants
twelve to sixteen bits, the generation something above thirty-two, and the two together
leave room in a machine word for the kind.

### TLAPS Proof

The proof strategy, applied to both written levels:
1. Prove the safety invariants of each level independently, by induction on `Next`
2. Prove that level's liveness from that level's fairness assumptions
3. Prove the refinement to the level above (simulation on the identity mapping)
4. Prove refinement liveness by lifting: the current level's fairness and safety
   implement each fairness conjunct of the level above, so the upper level's
   liveness is inherited rather than re-proved

Each level is split in four modules: the state (`*State.tla`, the variables and the
constant assumptions), the specification (`*.tla`), the invariants and the theorem
*statements* (`*_defs.tla`, `*Theorems.tla`), and the proofs (`*Theorems_proofs.tla`).
The split lets `check_theorem_statements.py` verify that every proved theorem restates
its declaration verbatim, so a proof can never quietly weaken what it claims. No proof
step is `OMITTED` at either level.

A second checker binds this document to those modules. `ci/check_property_manifest.py`
compares each level's property list here with the conjunctions that level proves -
`SafetyCore` and `LivenessProperties` at level 0, `FfiCallInv`, the extra conjuncts of
`SafetyInvariant` and `LivenessProperties` at level 1 - and fails in both directions: a
property claimed here that no manifest carries is a promise nothing proves, and a proved
conjunct absent here is a guarantee nobody can find. Both run in `ci/check.sh`. The
send-window deadlock got in through exactly that gap - the accounting and the ABI comment
describing it drifted apart with nothing comparing them - so the checker is part of the
fix rather than housekeeping.

#### What is actually verified, as of this revision

The document describes the target specification; the verified artefact trails it while a
change is in flight. This table is the honest reading, and it is meant to be updated with
the artefact rather than left to rot:

| Element | Status |
|---------|--------|
| Specification described in this document | Current |
| Model-checking configurations | At levels 1 and 0, twelve configurations exist - eight at level 1, four at level 0 - and running them is not part of this gate: every property they would check is proved by tlapm, over unbounded constants where the configurations would fix `Ceiling = 3` and unit messages. They are kept for exploration and debugging - a checker that prints a counterexample trace is the fastest way to understand a broken draft - not as evidence |
| Level 1, by windows at `--stretch 1` | **13803 obligations, all proved, at `--threads 2` with the cache enabled**, summed over three windows, plus **23 obligations** for `FfiGrpcEnabledTheorems_proofs` - the three conditional-enabledness theorems, which live in their own pair for the reason given below, so the level's total is 13826. A single pass with the cache disabled is the stronger verification: with the optimized tlapm build (`qdelamea-aneo/tlapm`, `/root/tlapm-opt-wil`) it is fast enough to iterate on, and it is the only count free of the obligations two adjacent windows would both cover |
| Level 0, one pass at `--stretch 1` | **1809 obligations, all proved, at `--threads 8` with the cache disabled** - the event-trace conjuncts `EventStreamShape` and `MessageEventsMatchDelivered` joined `SafetyCore`, so the level-0 module changed and was re-proved in full |
| A scatter of failures clustered by *backend* is a resource signature | At `--threads 4` on a machine where other provers were running, the same module returned 12 failures and **every one of them named `Isa`** - including steps untouched for weeks and unrelated to each other. Isabelle is the first backend to exhaust its budget under contention. Read the failing lines before theorizing about the goals they carry: the cluster was diagnosed twice as a property of `Fairness` before anyone looked at the method column. Every Isabelle call in the module carries `IsaT(600)` - a ceiling and not a cost, so a step needing two seconds still takes two, and an Isabelle failure now means a proof defect rather than contention |
| Where Isabelle is irreducible | Extracting one weak-fairness conjunct at a fixed identifier needs a backend that can instantiate a lemma whose conclusion is a conjunction of `WF_` atoms. `PTL` cannot instantiate; **Zenon cannot read `WF_` at all**. Four `QED` steps that were only doing modus ponens on a quantifier-free antecedent moved to `PTL`; the seven citations of `FairnessAtCall` and its siblings cannot move, and the three `QED`s whose antecedent crosses a bounded quantifier cannot either |
| `ExpandENABLED` and `TypeOK` | Never expand `TypeOK` in the `BY` of an `ExpandENABLED` call. `FreeBufferEnabled` resisted every backend, budgets to 300s and `--stretch 5` while its DEF list carried `TypeOK`: the expansion piles one membership conjunct per variable onto a goal that is already an existential over every primed variable, and the solver stops finding the witness. Use `TypeOK` only in the step that establishes `vars' # vars` beforehand - here a prime-free disequality on the `EXCEPT` - and cite it as an opaque fact in the `ExpandENABLED` step. The same proof then closes at `--stretch 1`. It surfaced when the free began writing a variable of its own, because while a variable is unconstrained the solver refutes "nothing changed" by varying it and never walks the long path |
| `ci/check_theorem_statements.py` | 111 declarations, each restated verbatim in its proofs module, across four declaration/proof pairs - the level-2 pair joined the three when `DotNetBindingTheorems` was declared |
| `ci/check_action_footprints.py`, `check_abi_coverage.py`, `check_proofs_present.py`, `check_arity.py` | Green |
| `ci/check_sketch_actions.py` | Green: 11 action citations in the sketches, all defined. The implementation sketches are normative, and each step names the action it realizes in a `// TLA:` comment; this checks the citations resolve. It does not check the ORDER - nothing short of a proof does - but a citation pointing at nothing is the first sign a sketch and the machine have parted, and it is mechanical where reading prose against prose is not: two reviews called one sketch consistent with the machine while it released a payload before the read's result was decided, a state the machine does not have |
| `ci/check_state_literals.py` | Green: 16 typed state variables, 2642 literals, all admissible. A retired value neither fails to parse nor fails to type - a comparison against it is simply always false, so a guard becomes dead and a model constraint prunes more than intended while every property still reports clean. A constraint reading `call_dispose_state = "disposed"` after that value became `settled` shrank two configurations that way. Assignments are covered as well as comparisons, and by choice rather than for symmetry: `TypeOK` catches a bad one only in a run that reaches that branch, so an assignment on a rare path can sit wrong indefinitely. The binding comes from the typing conjuncts rather than a table - including the sentinel idiom `var \in OtherIds \union {"none"}`, whose only admissible literal is that sentinel - so a renamed state is caught wherever it is still spelled |
| SANY, on the twenty-seven SANY-clean modules | Green |
| `ci/check_property_manifest.py` | Green: this document's property lists and the manifests name the same properties |
| The two memory observers' normative invariants | **Covered at level 1.** `buffer_charge` holds the bytes each lent buffer was granted, `ReceivedLength` the length of each message received, and `memory_used` the runtime-wide total; `MemoryAccountingExact` states `memory_used = BytesOutstanding + BytesReceived + BytesHeldByEngine` and `MemoryWithinHardCeiling` that the total never passes `HardCeiling`. Both are in `IndInv` and proved inductive. The category totals - `BytesHostLent`, `BytesSendInFlight`, `BytesRuntimeHeld` on the send side, `BytesHostReceived` and `BytesRuntimeReceived` on the receive side - are sums over the pairs each state selects, and the sixth, `BytesHeldByEngine`, is the compressed copies of sent messages that the engine charges to the same count: `engine_held`, which the engine's two steps move and the lend's room check reads. `CategoriesPartitionTotal` and `ReceivedCategoriesPartitionTotal` are the snapshot identities the observers must report, the six adding up to the total through `MemoryAccountingExact` |
| Level 2 | **Refined and proved, liveness included.** Fourteen modules exist, thirteen of them SANY-clean and registered in `ci/check.sh`; the manifests hold 27 safety conjuncts and 17 liveness properties, bound to this document by the manifest checker, and `DotNetBindingTheorems` declares the freeze's obligations. TLC, in nine configurations, with no invariant violation in any run that checks one: `DotNetBinding_MCdirected`, its one call declaring both shapes, is exhaustive - 628413 states, depth 39 - and its constraint prunes every state with a returned buffer before the shutdown, which an exchange and a send both leave, so it never explores past either, and the engine, which takes bytes only for a call that has sent, takes none there; `DotNetBinding_MC`, which has no such constraint, does: an exploratory run of four minutes, made before the engine's bytes were modelled, covered over a million states, `WriteResizesBuffer` taking 69350 of them, with no violation. `DotNetBinding_MCcall`, whose call declares one request, and `DotNetBinding_MC`, whose calls are streams, are bounded and run as instantiability checks, about 10 seconds each (`-Dtlc2.TLC.stopAfter=10`, which `ci/tlc.sh` does not pass): the proof carries the content, and what TLC adds is that the configuration binds every constant and every variable. `DotNetBinding_MClive` evaluates the seventeen liveness properties under the three fairness tiers, 17 branches, run the same way. Five configurations are witnesses rather than checks at level 2, and three more at level 1: their targets are stated negatively, so a violation trace is the result. `FfiGrpc_MCwitnessResize` and `DotNetBinding_MCwitnessResize` each show a lent buffer exchangeable for a larger one, the case `ResizeSendBuffer` and `WriteResizesBuffer` exist for, reached in under 400 and under 100 distinct states. `FfiGrpc_MCwitnessEngineBytes` and `DotNetBinding_MCwitnessEngineBytes` each show the engine taking bytes for itself and giving them back, the case `EngineTakesBytes` and `EngineGivesBackBytes` exist for, reached in under 4000 and under 16000 distinct states; `FfiGrpc_MCwitnessEngineLend` shows a lend refused for room it would have had were the engine holding nothing, the refusal `engine_held` exists for, reached in under 4000. `DotNetBinding_MCwitness` shows a cancelled parse holding the terminal slot on a healthy runtime - the case `FinishCancelledParse` decodes the status for; `DotNetBinding_MCwitnessPrologue` shows a token firing on a read suspended before the metadata - the case `BeginMoveNext`'s prologue guard exists for; `DotNetBinding_MCwitnessBudget` shows a write waiting on the send budget after the server has ended its call - the case `CancelWriterWait`'s guard on a call no longer active exists for. Without them any of these branches could be dead code, and a proof about a step that never fires proves nothing. A bounded run is evidence about what it explored and nothing more. TLAPS: the refinement is closed - `RefinesInit`, `RefinesNext` disjunct by disjunct, the twenty-one fairness lifts, `ManagedIndInvHolds`, `ManagedSafetyHolds`, `DerivedInvariantsHold` and `RefinesSpec`, which carries every level-1 theorem here, its fifteen liveness properties included.  The induction forced six invariant conjuncts into words that no safety statement had asked for, three of them under a passthrough - which is to say when the native side moves beneath the managed layer, where no managed action could have revealed them.  All seventeen managed liveness promises are proved. With the engine's bytes in the model, the three proofs modules they reach verify by windows with the fingerprint cache enabled - `FfiGrpcTheorems_proofs` 13803 obligations, `FfiGrpcEnabledTheorems_proofs` 23 and `DotNetBindingTheorems_proofs` 34508, no failure; `AbstractGrpcTheorems_proofs`, which level 0 alone carries, verifies with the cache disabled at 1809; the counts are sums over windows, which may count an obligation twice at a boundary.  The seventeen cost fifteen derived invariants, listed above. |
| Deadlock detection at level 2 | `ci/tlc.sh` passes `-deadlock`, which switches TLC's deadlock check off, so the gate has never used it at any level - worth knowing before reading a clean run as evidence of progress. Invoked directly, `DotNetBinding_MC` reaches a deadlock: every channel refused and the runtime torn down, the finite `ChannelIds` set spent, a rejected channel being terminal. That is quiescence rather than a stall, and an artefact of the bound rather than a property of the system, which the configuration now states. `AbsentRuntimeOwesNothing` carries the content instead, and a genuine mid-flight stall still breaks the liveness configuration |

There is an objection to modelling any of this, and it is half right, so it is worth stating.
The partition identity is close to true by construction: `BytesOutstanding` is a sum over the
pairs `buffer_state` selects, so `CategoriesPartitionTotal` discriminates no design and would
catch no defect on its own. Where the objection stops holding is `MemoryAccountingExact`, which
is not of that kind. It relates a counter
the actions update by arithmetic - `memory_used' = memory_used + charge` on the lend,
`- buffer_charge[<<cId, b>>]` on the free, the difference on an exchange - to a sum over a set those same actions reshape, and
nothing makes the two agree except the actions being written correctly. It is what makes the
ceiling mean anything: without it `MemoryWithinHardCeiling` bounds a number with no stated
relation to the memory that is out. It is also the only reason `memory_used` can be typed `Int` and still
be known non-negative.

The price the objection names is real and is paid: the sums over sets are the expensive
part of these proofs. `SumFunctionOnSet` from the standard `Functions` module and its theory in
`FunctionTheorems` carry it - `SumFunctionOnSetAddIndex` for the lend, `SumFunctionOnSetRemoveIndex`
for the free, `SumFunctionOnSetEqual` where a state moves without changing a charge. A fold taking
its summand as an *operator* parameter has no citable primed form, which is the trap that shape
walks into; `SumFunctionOnSet` is first order in both arguments and the prime distributes.

The refusals are modelled, and as their own actions: `RefuseLendTooLarge`,
`RefuseLendForSlot` and `RefuseLendForBudget` write `last_lend_status` and nothing else. They
carry no fairness - refusing is never owed - and every property proved of the other actions
crosses them, since the state they touch is read by none of them.

Modelling them is not decoration: the refusals are the observable frontier of the downcall,
the states a level-2 binding refines its retry decisions against, and what the ABI's status
codes mean is defined by which model action wrote them. `RefuseLendForBudget` takes the
charge as a parameter: it records that *this* one did not fit, not the claim that none would
- a smaller charge may already fit when the refusal lands.

A dead action would satisfy every safety proof - TLAPS happily proves that an action that
can never fire preserves everything - so each status carries its own conditional
enabledness theorem: `TooLargeRefusalEnabled`, `SlotRefusalEnabled` and
`BudgetRefusalEnabled` state that at any eligible state the refusal whose guard holds is
`ENABLED`. Conditional is the honest word: they do not prove the hypotheses reachable
from `Init` - `SlotRefusalEnabled`'s full window, in particular, is reachable only when
`Cardinality(BufferIds) >= MaxSendsInFlight` - but the first exhibits its own rigid
witness, `Ceiling + 1` being in the request domain and never lendable. That witness is
why the
refusals quantify over `RequestLengths == 0..(Ceiling + 1)` rather than over the lendable
sizes: one representative above the ceiling stands for every larger request, and without it
`RefuseLendTooLarge` would be unsatisfiable and every proof about it vacuously true. The
budget refusal's charge ranges over `CandidateCharges`, the same domain, for the same
reason.

What stays true is the shape of the refusal. **A refusal is not a state of the runtime**: it
records what a downcall returned, not a condition the runtime is in. A runtime-level
`RESOURCE_EXHAUSTED` state would be the same mistake as the fatal ceiling that preceded this
design, and promoting a refusal to a state is what made the runtime undestroyable.

What does deserve a model is the retry protocol, and it is level 2's because it is about the
binding's own scheduling rather than about bytes. One boolean per call - retrying or not -
carries `BudgetWaitEndsWhenHopeless` and `MessageTooLargeIsNotRetried` with no counter
anywhere, and it carries the deadlock that level 1 cannot see: level 1 *assumes* the host
gives back what it holds, so a host blocked polling for capacity while holding a lent buffer
is admitted there and fatal in practice. See `WaitingWriterHoldsNoBuffer`.

Nothing above is `OMITTED`, nothing fails, and both obligation counts were measured on the
model as it stands here rather than carried over. The proof's framing rests on thirteen
framing lemmas - `OnlyCallStartWritesCallChannel`, `EveryStepEitherLendsOrKeepsBuffers` and
their siblings, each naming the writers of one variable - over four projection lemmas
(`FfiOnlyStutters`, `StutterProjects`, `FfiOnlyStepsKeepL0`,
`RuntimeAndChannelStepsKeepCalls`), which trade one large obligation for several small
ones. The shape is load-bearing: frames that enumerate the whole action alphabet put
every new action in every frame obligation, and past nineteen frames no solver timeout
closes them - naming each variable's writers is what lets the alphabet keep growing.

---

## Mapping to the code

### Where each level-1 action happens

The table is the contract between the proof and the code. Every action of `FfiGrpc` has
exactly one linearization point; a change on either side that breaks a row breaks the
refinement.

| Level-1 action | Linearization point |
|----------------|---------------------|
| `RuntimeCreate` | `ak_runtime_create`, or `ak_runtime_create_from`, publishes the runtime as RUNNING |
| `ChannelCreate` | `ak_channel_create` publishes the channel as open |
| `ChannelStartClosing` | `ak_channel_release`, or the runtime's shutdown closing the gate |
| `ChannelFinishClosing` | the last call of a closing channel reaches its terminal |
| `CallStart` | `ak_call_start` registers the actor and returns `AK_STATUS_OK` |
| `LendSendBuffer` | the bounded CAS on the slot counter succeeds, inside `ak_get_call_buffer`. Its three refusals - `AK_STATUS_SLOT_BUSY` for this call's window, `AK_STATUS_BUDGET_BUSY` for the runtime-wide ceiling, `AK_STATUS_MESSAGE_TOO_LARGE` for a request past it - are the model actions `RefuseLendForSlot`, `RefuseLendForBudget` and `RefuseLendTooLarge`, linearizing at the check that fails; each writes the call's last-lend status and nothing else |
| `HostReturnsBuffer` | `ak_return_call_buffer` gives a lent buffer back unused |
| `FreeReturnedBuffer` | the runtime releases the buffer's bytes from its count, once no unacquitted send lives in it; the allocation goes when nothing holds it, which the model does not see. Not a downcall: giving a buffer back is the host's step, releasing its bytes is the runtime's |
| `ResizeSendBuffer` | `ak_resize_call_buffer`: the one bounded step on the ledger's count that moves it by the new charge less the old, once the new arena is in hand. Every refusal comes before that step or at it and leaves the old buffer lent and charged, so none is a step of the model; `AK_STATUS_CORRUPTED` is not modelled, as at the commit. The old arena goes to the channel's spares after the step, which the model does not see |
| `SendMessage` | `ak_call_send_message` hands the filled buffer to the actor; on a call that declared one request, it puts the request in the call's slot, the sending still open and no status pending |
| `EndSend` | `ak_call_end_send`: the actor takes the END_STREAM command off its queue; on a call that declared one request, `ak_call_send_message` right after `SendMessage`, in the same downcall, which ends the sending with the request |
| `EmitWriteDone` | the actor invokes the callback with `AK_EVENT_WRITE_DONE`; on a call that declared one request, the commit, after `EndSend`, gives back the send's slot and bytes with no callback |
| `WriteDoneReturns` | that callback returns to the actor; on a call that declared one request, the commit, right after `EmitWriteDone` |
| `DeliverInitialMetadata` / `DeliverMessage` / `DeliverStatus` / `DeliverCancelled` | the actor stages the event for the data callback once it has it in hand, having taken a credit; the callback carries every event staged before it calls the host |
| `DeliveryCallbackReturns` | that callback returns to the actor; for an event another of the same callback follows, the actor takes it just before it stages that one, the host not having been called |
| `HostConsumesEvent(c)` | `ak_event_consumed` frees the oldest outstanding payload, identified by its `owner`; `ak_events_consumed` of `n` payloads is `n` of these, in order |
| `RequestCallCancellation` | `ak_call_cancel`: the actor observes the flag, not the downcall's return |
| `ReleaseCallHandle` | the last debt of a terminal call clears: no payload owed, no buffer out, its own callbacks returned. Taken by the thread that clears it - the host's, in the downcall that gives the last payload or buffer back, or the engine's, as the last callback returns. Not a downcall of its own |
| `RuntimeBeginShutdown` | `ak_runtime_begin_shutdown` closes the start gate |
| `EmitShutdownComplete` | the runtime task invokes the callback with `AK_EVENT_SHUTDOWN_COMPLETE` |
| `ShutdownCallbackReturns` | that callback returns |
| `RuntimeRelease` | the runtime publishes `AK_RUNTIME_GRPC_STOPPED`, or `AK_RUNTIME_QUIESCENT` when nothing of it is outstanding. Both refine the level-0 RELEASED state; which one the host reads is the release signal, not a level-0 distinction |
| `EmitResourcesReleased` | the runtime task invokes the callback with `AK_EVENT_RESOURCES_RELEASED`, owed only when `SHUTDOWN_COMPLETE` carried `AK_HOST_MUST_RETURN` |
| `ResourcesReleasedCallbackReturns` | that callback returns, which completes the resources branch. It does not by itself make the status `AK_RUNTIME_QUIESCENT`: the order against `RuntimeRelease` is free, so the level-0 transition may still be owed |
| `RuntimeDestroy` | `ak_runtime_destroy` accepts, its precondition checked |
| `AdmitRead` | the call's read loop, before it asks for its next message, finds the count below the first threshold lowered by the largest length a refused send waits on. Held back, it waits for a release or for that send to be served |
| `NetworkReceive` | the read loop has the next message decoded and charges its length, the count staying at or below the second threshold |
| `EndCallPastHardCeiling` | the decoded message would take the count past the second threshold: the call ends with `RESOURCE_EXHAUSTED` and the message is dropped, never charged |
| `engine_held`, `EngineTakesBytes`, `EngineGivesBackBytes` | `CopyBudget` in `armonik-transport-ffi/src/ledger.rs`. The term is the bytes the ledger holds for compressed copies. `EngineTakesBytes` is `Ledger::hold_copy`, called by `CopyBudget::charge` once the copy is made: the bounded CAS on the byte count against the first threshold. A refusal returns no charge, the copy is dropped and the message goes out uncompressed, so it is no step of the model. `EngineGivesBackBytes` is `Ledger::release_copy`, run when the charge goes with the message that holds the copy, written or not: the bytes come off the count and the sends that wait are owed their wake-up. The count a shutdown waits on does not move for either, which is why no quiescence condition reads the term. The fairness instance, `EngineGivesBackAllBytes`, is every message that holds a copy being dropped, which holds once the application's compressed sending stops: an assumption on the application |
| `EmitBudgetWake` | the call's task invokes the callback with `AK_EVENT_BUDGET_WAKE`, after a release that gave bytes back while the call's send waited |
| `NetworkSend` / `ReceiveStatus` | internal to the `grpc` module, not observable at the ABI |
| `RuntimeFail` | any unrecoverable runtime fault - but not reaching the configured ceiling, which is a refusal, nor a genuine allocator failure inside `ak_get_call_buffer`, which refuses that lend with `AK_STATUS_INTERNAL` and changes nothing level 1 carries; the model leaves the state that follows unconstrained |
| `RemainFailed` / `RemainReleased` | explicit stutter, so a terminal runtime state has a step and the temporal proofs need no special case |
| none: outside the model | `ak_channel_delivery_window` writes the delivery window the channel ended up with. The constant `DeliveryCredits` stands for any one channel's window, and that the value read equals it is an assumption of how the model is instantiated. The window is fixed for the life of the channel and the read changes no state, so it takes no step and has no linearization point |
| none: outside the model | `ak_runtime_stats` and `ak_channel_stats` write the counters and gauges the engine keeps of its channels' calls, connections and waits, over the runtime and over one channel's endpoint, and `ak_channel_endpoint` writes the endpoint a channel is on. They are observational: they change no state the model carries and take no step, so they have no linearization point. Whether the library counts at all is a property of how it was built, which the record's flags report |

#### Which ABI argument becomes what

A function's arguments are as much of the contract as its name, and an argument the model
drops is a decision rather than an omission. This table is the record, and
`ci/check_abi_coverage.py` refuses an argument of an acting function that has no row.

| ABI argument | in the model |
|---|---|
| every `ak_*_handle` | the identifier parameter: `rtId`, `chId`, `cId` |
| `ak_get_call_buffer`'s `*out` | `b` in `LendSendBuffer(cId, b)` - the allocation lent |
| `ak_call_send_message`'s `buffer` | `b` in `SendMessage(cId, msg, b)`. An argument, not a choice made inside the action: the host names the allocation it commits, and letting the model pick would make `buffer_send` a record of nondeterminism rather than of what the caller passed |
| `ak_return_call_buffer`'s `buffer` | `(cId, b)` in `HostReturnsBuffer(cId, b)` - a buffer determines its call, so the pair *is* the buffer |
| `ak_call_send_message`'s `written` | **not modelled.** What the host wrote is no state: a commit fits when `MessageLength[msg]` is at most the buffer's length, which is `FitsInBuffer`. A `written` past the lend is the overrun answered with `AK_STATUS_CORRUPTED`, which a conforming host never commits |
| `ak_resize_call_buffer`'s `buffer` | `(cId, b)` in `ResizeSendBuffer(cId, b, nb, len, charge)`, as for `ak_return_call_buffer`: a buffer determines its call |
| `ak_resize_call_buffer`'s `new_len` | `len`, as `ak_get_call_buffer`'s: `IsLendable(len)` the request being in range, `CoversRequest(charge, len)` the allocator's rounding, and `IsMemoryAvailableForExchange(cId, b, charge)` the ceiling admitting the difference. A length of zero is refused as an invalid argument |
| `ak_resize_call_buffer`'s `keep` | **not modelled**, as `written`: the bytes the host wrote are no state of the model, and a `keep` past the lend or past `new_len` is a refusal or an overrun, which take no step |
| `ak_resize_call_buffer`'s `*out` | `nb` in `ResizeSendBuffer(cId, b, nb, len, charge)` - the allocation lent in place of `b` |
| `ak_events_consumed`'s `payloads` and `count` | **not modelled**, as `ak_event_consumed`'s `payload`: `count` is how many `HostConsumesEvent` steps the downcall is |
| `ak_event_consumed`'s `payload` | **not modelled.** Release is FIFO by ABI rule, so the release count already says which payload is owed. That makes `ReleasesNeverExceedDeliveries` conservation of a count under a conformance assumption rather than a proof about identities - the one place the send side is now stronger than the receive side, and an open item rather than an oversight |
| `ak_get_call_buffer`'s `len` | The model takes the length directly: `LendSendBuffer(cId, b, len, charge)`, with `charge` the size the allocator returned. The lend sees only a length, exactly as the C function does; the message identity is born at the commit, where `SendMessage` requires `MessageLength[msg] <= buffer_length` for the buffer it sends: the commit says how many bytes the host wrote, at most the lend, as the ABI's does. The overrun the ABI answers with `AK_STATUS_CORRUPTED` is not modelled, since a conforming host never commits it. `IsLendable(len)` is the request being in range, `IsMemoryAvailable(charge)` the ceiling admitting what backs it, and `CoversRequest(charge, len)` ties the two. A length of zero is refused as an invalid argument: an empty message needs no buffer, and `ak_call_send_message` sends it with none, a send the model does not represent since it takes no memory. Level 0 carries no sizes: its send window counts allocations |
| `ak_channel_create`'s `endpoint` | **not modelled.** The model's channels are identifiers, and what one connects to changes nothing it guarantees. An argument, or, empty, the `Endpoint` of the runtime's options; `TransportOptions` carries none |
| `config`, `config_json`, `options` | **not modelled**, `ak_runtime_create_from`'s configuration struct and the sources it lists included. Configuration reaches the model as the constants `MaxSendsInFlight`, `DeliveryCredits`, `Ceiling` and `MessageLength`; the rest does not change what the ABI guarantees |
| `callback`, `runtime_ctx`, `call_ctx` | **not modelled at level 1.** They are identity plumbing, and what must hold of them is level 2: `TokenPublishedBeforeStart` and `RootSurvivesCallbacks` |
| `ak_channel_endpoint`'s `buffer`, `capacity` and `length` | **not modelled.** The host's buffer for the endpoint's text, its size, and where the whole length is written: the endpoint is no state of the model, which keeps channels as identifiers, and the call is observational |
| every other `*out` | **not modelled.** A returned handle is the identifier the action already quantifies over |
| every `out_error` | **not modelled.** It is written only on a refusal, and a refusal takes no step: the model says why a downcall is refused by the guard that does not hold, and what `out_error` adds is the message for a human |

Two rows carry the ownership argument, and they are asymmetric. `EmitWriteDone` is the
only producer of WRITE_DONE and it is per-actor, so a send really is acquitted exactly
once and `WriteDonesNeverExceedSends` proves it. `HostConsumesEvent` is the other half,
and there the proof is weaker than the sentence one would like to write: the model counts
releases, it does not track which `owner` a release names, so `ReleasesNeverExceedDeliveries`
proves no over-consumption *given* that the host releases the right owner, once, in
order. That assumption sits with the fairness hypotheses, and the level-2 obligations are
where the binding pays it. A defensive ABI that rejected a duplicate token would need the
identities in the model; this design chooses assume-guarantee instead.

`ReleaseCallHandle` linearizes where the last debt clears, on the thread that clears it, not
at a downcall of its own - the ABI has none for it. What it establishes - `ReleasedCallIsClean` - is
proved to survive every later step, since nothing can lend or deliver on a retired call.
That is the formal content of "at the end of a call, whatever the ending, everything is
back".
