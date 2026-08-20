----------------------- MODULE FfiGrpcTheorems_proofs -----------------------
(***************************************************************************)
(* Proofs of every result declared in FfiGrpcTheorems.  Each theorem is    *)
(* restated verbatim (check_theorem_statements.py enforces it) and         *)
(* discharged with tlapm.  The level-0 results are cited through the L0    *)
(* instance and never re-proved.                                           *)
(***************************************************************************)

\* FunctionTheorems for the theory of SumFunctionOnSet: type, agreement, the
\* empty sum, adding and removing an index, and the disjoint union.  All of it
\* first order in the function and the index set, which is what makes the
\* primed total citable at all.
EXTENDS FfiGrpc_defs, TLAPS, NaturalsInduction, FiniteSetTheorems,
        FunctionTheorems,
        SequenceTheorems

\* The spec speaks through state predicates; expand them in every
\* obligation so the proof steps keep reasoning on the underlying state.
USE DEF IsAwaitingWriteDone, IsWriteDoneCallbackRunning, HasFreeSendSlot,
        WriteDonesReturned, SendWindowOccupancy, HasNoSendInFlight,
        HasAcceptedSendAt, IsSendAcquittedAt, PayloadIndices,
        IsDeliveryCallbackRunning, IsCancelRequested, IsHandleReleased,
        OwedPayloads, HostOwnsPayload, HostOwnsNoPayload, HostOwnsSomePayload,
        HostHasDeliveryCredit, HostOwnsAtMostCredits,
        HostOwnsAtMostCreditsPlusOne, IsShutdownEventEmitted,
        IsShutdownCallbackRunning, IsStoppingRuntime, IsReleasedRuntime,
        SecondEventOwed, IsResourcesReleasedEmitted,
        IsResourcesReleasedCallbackRunning,
        IsClosingChannel, IsClosedChannel, HasNoDeliveredEvents

(***************************************************************************)
(* LOCAL HELPERS                                                           *)
(***************************************************************************)

\* The split event sets, re-enumerated: flat memberships for SMT.
LEMMA StatusKindsExpansion ==
    /\ L0!StatusKinds = {"COMPLETED", "CANCELLED"}
    /\ L0!EventKinds = {"INITIAL_METADATA", "MESSAGE", "COMPLETED", "CANCELLED"}
<1>1. QED
    BY DEF L0!StatusKinds, L0!EventKinds

LEMMA IndInvProjects == IndInv => L0!IndInv
<1>1. QED
    BY DEF IndInv, L0!IndInv, TypeOK, StrongInv

\* The six constant assumptions in their instantiated spelling: citing
\* an instantiated theorem re-raises them as antecedents, and the
\* instance prefixes even the standard-library IsFiniteSet.
LEMMA L0Assumptions ==
    /\ "none" \notin RuntimeIds
    /\ "none" \notin ChannelIds
    /\ "none" \notin CallIds
    /\ L0!IsFiniteSet(CallIds)
    /\ L0!IsFiniteSet(ChannelIds)
    /\ L0!IsFiniteSet(RuntimeIds)
<1>1. QED
    BY NoneNotInRuntimeIds, NoneNotInChannelIds, NoneNotInCallIds,
       FiniteCallIds, FiniteChannelIds, FiniteRuntimeIds, Zenon
    DEF L0!IsFiniteSet, IsFiniteSet

\* The refining families project onto the level-0 safe families; the
\* group definitions match disjunct for disjunct once the level-1 action
\* is unfolded to expose its L0! conjunct.
LEMMA RuntimeOnlyProjects == NextSafeRuntimeOnly => L0!NextSafeRuntimeOnly
<1>1. QED
    BY DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
           L0!NextSafeRuntimeOnly

LEMMA RuntimeChannelProjects ==
    NextSafeRuntimeChannel => L0!NextSafeRuntimeChannel
<1>1. QED
    BY DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
           L0!NextSafeRuntimeChannel

LEMMA ChannelOnlyProjects == NextSafeChannelOnly => L0!NextSafeChannelOnly
<1>1. QED
    BY DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
           L0!NextSafeChannelOnly

LEMMA ChannelCallProjects == NextSafeChannelCall => L0!NextSafeChannelCall
<1>1. QED
    BY DEF NextSafeChannelCall, ChannelFinishClosing,
           L0!NextSafeChannelCall

LEMMA CallOnlyProjects == NextSafeCallOnly => L0!NextSafeCallOnly
<1>1. QED
    BY DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
           NetworkSend, NetworkReceive, ReceiveStatus,
           DeliverInitialMetadata, DeliverMessage, DeliverStatus,
           DeliverCancelled, L0!NextSafeCallOnly

LEMMA RefiningProjects == NextSafeRefining => L0!NextSafe
<1>1. QED
    BY RuntimeOnlyProjects, RuntimeChannelProjects, ChannelOnlyProjects,
       ChannelCallProjects, CallOnlyProjects
    DEF NextSafeRefining, L0!NextSafe

\* The FFI-only families stutter on the level-0 state by construction.
LEMMA FfiOnlyStutters == NextSafeFfiOnly => UNCHANGED l0_vars
<1>1. QED
    BY DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
           EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
           ResourcesReleasedCallbackReturns, RuntimeDestroy,
           RequestCallCancellation, ReleaseCallHandle, LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget,
           HostReturnsBuffer, FreeReturnedBuffer, EmitWriteDone,
           WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent

LEMMA FailProjects == NextFail => L0!NextFail
<1>1. QED
    BY DEF NextFail, RuntimeFail, L0!NextFail

LEMMA StutterProjects == NextExplicitStutter => L0!NextExplicitStutter
<1>1. QED
    BY DEF NextExplicitStutter, RemainFailed, RemainReleased,
           L0!NextExplicitStutter

\* Every FFI-only action carries UNCHANGED l0_vars: that is what makes it
\* FFI-only.  Stating it once is what keeps the frames below from growing
\* a case, and a DEF list, every time the action alphabet does - the cost
\* that made them outgrow the solver when the buffer and destroy
\* downcalls arrived.
LEMMA FfiOnlyStepsKeepL0 ==
    ASSUME NextSafeFfiOnly
    PROVE  UNCHANGED l0_vars
<1>1. QED
    BY DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer, EmitWriteDone,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent

\* The mirror on the level-0 side: runtime and channel steps leave every
\* call variable alone, so a frame about one call only ever has to look
\* at NextSafeChannelCall and NextSafeCallOnly.
LEMMA RuntimeAndChannelStepsKeepCalls ==
    ASSUME \/ NextSafeRuntimeOnly
           \/ NextSafeRuntimeChannel
           \/ NextSafeChannelOnly
    PROVE  UNCHANGED L0!CallVars
<1>1. QED
    BY DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextSafeChannelOnly,
        RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
        ChannelCreate, ChannelStartClosing,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!RuntimeBeginShutdown,
        L0!ChannelCreate, L0!ChannelStartClosing,
        RequestCancellationOfActiveCalls, L0!CallVars

\* Failure and the two explicit stutters leave the calls alone as well:
\* a failed runtime is unconstrained afterwards, not retroactively.
LEMMA FailAndStutterStepsKeepCalls ==
    ASSUME NextFail \/ NextExplicitStutter
    PROVE  UNCHANGED L0!CallVars
<1>1. QED
    BY DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
        RemainReleased, L0!RuntimeFail, L0!RemainFailed, L0!RemainReleased,
        L0!vars, L0!CallVars

\* Neither do they touch the delivery flag: the closing paths write the
\* cancel latch and nothing else of the FFI state.
LEMMA RuntimeAndChannelStepsKeepDeliveryFlags ==
    ASSUME \/ NextSafeRuntimeOnly
           \/ NextSafeRuntimeChannel
           \/ NextSafeChannelOnly
           \/ NextSafeChannelCall
    PROVE  UNCHANGED delivery_callback_running
<1>1. QED
    BY DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextSafeChannelOnly,
        NextSafeChannelCall, RuntimeCreate, RuntimeRelease,
        RuntimeBeginShutdown, ChannelCreate, ChannelStartClosing,
        ChannelFinishClosing, ffi_vars

\* A delivery callback starts in HandPayloadToHost, which belongs to the
\* four refining Deliver actions.  No FFI-only step can put one on the
\* stack; the one that writes the flag takes it off.
LEMMA FfiOnlyStepsNeverStartDelivery ==
    ASSUME TypeOK, NextSafeFfiOnly, NEW c \in CallIds,
           ~IsDeliveryCallbackRunning(c)
    PROVE  ~((IsDeliveryCallbackRunning(c))')
<1>1. QED
    BY SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer, EmitWriteDone,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent,
        IsDeliveryCallbackRunning, TypeOK, L0!TypeOK

LEMMA NextDecomposition == Next <=> NextByFootprint
<1>1. QED
    BY DEF Next, NextByFootprint, NextSafe, NextSafeRefining,
           NextSafeFfiOnly, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
           NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
           NextSafeShutdownFfi, NextSafeCallFfi, NextFail,
           NextExplicitStutter

\* Channel and call steps leave the runtime alone entirely: neither its
\* state nor its shutdown signal.
LEMMA ChannelAndCallStepsKeepRuntime ==
    ASSUME \/ NextSafeChannelOnly
           \/ NextSafeChannelCall
           \/ NextSafeCallOnly
    PROVE  UNCHANGED <<runtime_state, shutdown_event_emitted,
                       shutdown_callback_running>>
<1>1. QED
    BY SMTT(120)
    DEF NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, DeliverCancelled, HandPayloadToHost,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!vars, l0_vars, ffi_vars

\* The parallel sequence has exactly one writer: committing a buffer is
\* what creates a send, so it is what records which buffer carries it.
LEMMA OnlySendMessageWritesBufferSend ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds, m \in Messages,
                 bs \in BufferIds : SendMessage(c, m, bs)
           \/ UNCHANGED buffer_send
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* Four writers, and they are the whole buffer lifecycle: lending, giving
\* back, committing (which also gives back), and releasing the bytes.
LEMMA OnlyBufferStepsWriteBufferStates ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds, b \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(c, b, msg, ch)
           \/ \E c \in CallIds, b \in BufferIds : HostReturnsBuffer(c, b)
           \/ \E c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
           \/ \E c \in CallIds, m \in Messages, b \in BufferIds :
                  SendMessage(c, m, b)
           \/ UNCHANGED buffer_state
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
  <2>1. CASE NextSafeShutdownFfi
    <3>1. UNCHANGED buffer_state
      BY <2>1, SMT DEF NextSafeShutdownFfi, EmitShutdownComplete,
          ShutdownCallbackReturns, EmitResourcesReleased,
          ResourcesReleasedCallbackReturns, EmitResourcesReleased,
          ResourcesReleasedCallbackReturns, RuntimeDestroy
    <3>2. QED BY <3>1
  <2>2. CASE NextSafeCallFfi
    BY <2>2, SMT DEF NextSafeCallFfi, RequestCallCancellation,
        ReleaseCallHandle, EmitWriteDone, WriteDoneReturns,
        DeliveryCallbackReturns, HostConsumesEvent,
        RefuseLendTooLarge, RefuseLendForSlot, RefuseLendForBudget
  <2>3. QED BY <1>2, <2>1, <2>2 DEF NextSafeFfiOnly
<1>3. CASE NextFail
    BY <1>3, SMTT(120) DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* The budget has exactly two writers: the lend that may exhaust it and the
\* free that may relieve it.  Every other step leaves it alone, which is what
\* lets the interlock be read off one framing fact instead of an action
\* alphabet.  Lend and free stay opaque here - they are the disjuncts being
\* proved - so the case that expands NextSafeCallFfi has seven actions to
\* look at and two to hand back.
LEMMA OnlyBudgetStepsWriteBudget ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds, b \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(c, b, msg, ch)
           \/ \E c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
           \/ UNCHANGED <<buffer_charge, memory_used>>
<1>1. CASE NextSafeRefining
    BY <1>1, SMTT(120)
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMTT(120)
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        HostReturnsBuffer, WriteDoneReturns, DeliveryCallbackReturns,
        HostConsumesEvent, RefuseLendTooLarge, RefuseLendForSlot,
        RefuseLendForBudget, l0_vars, L0!vars
<1>3. CASE NextFail \/ NextExplicitStutter
    BY <1>3, SMTT(120)
    DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
        RemainReleased, IsRuntimeDrained, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>4. CASE UNCHANGED vars
    BY <1>4, SMT DEF vars, l0_vars, L0!vars, ffi_vars
<1>5. QED
    BY <1>1, <1>2, <1>3, <1>4, NextDecomposition
    DEF NextByFootprint, NextSafe

\* --- THE BUDGET'S ACCOUNTING ---
\* The total is SumFunctionOnSet(buffer_charge, OutstandingPairs): a function
\* summed over a set of indices, both arguments first order.  That matters for
\* one reason - tlapm does not distribute a prime through a higher-order
\* application, so a fold taking its summand as an operator has no citable
\* primed form.  This one primes like any other application.
\* Everything below is a citation of FunctionTheorems.  Nothing is hand-rolled.

LEMMA PairSetsFinite ==
    /\ IsFiniteSet(CallIds \X BufferIds)
    /\ IsFiniteSet(OutstandingPairs)
    /\ IsFiniteSet(LentPairs)
    /\ IsFiniteSet(InFlightPairs)
    /\ IsFiniteSet(HeldPairs)
<1>1. IsFiniteSet(CallIds \X BufferIds)
    BY FiniteCallIds, BufferIdsAreAFiniteNonemptySet, FS_Product
    DEF FiniteCallIds, BufferIdsAreAFiniteNonemptySet
<1>2. /\ OutstandingPairs \in SUBSET (CallIds \X BufferIds)
      /\ LentPairs \in SUBSET (CallIds \X BufferIds)
      /\ InFlightPairs \in SUBSET (CallIds \X BufferIds)
      /\ HeldPairs \in SUBSET (CallIds \X BufferIds)
    BY Zenon DEF OutstandingPairs, LentPairs, InFlightPairs, HeldPairs
<1>3. QED BY <1>1, <1>2, FS_Subset

\* The charges are naturals on every pair, which is the hypothesis every one
\* of the sum theorems asks for.
LEMMA ChargesAreNat ==
    ASSUME TypeOK, NEW S \in SUBSET (CallIds \X BufferIds)
    PROVE  /\ \A q \in S : buffer_charge[q] \in Nat
           /\ \A q \in S : buffer_charge[q] \in Int
<1>1. QED BY SMT DEF TypeOK, L0!TypeOK

LEMMA BytesOutstandingType ==
    ASSUME TypeOK
    PROVE  BytesOutstanding \in Nat
<1>1. \A q \in OutstandingPairs : buffer_charge[q] \in Nat
    BY ChargesAreNat, Zenon DEF OutstandingPairs
<1>2. QED
    BY <1>1, PairSetsFinite, SumFunctionOnSetNat DEF BytesOutstanding

\* The counter tracks the charges of the buffers out, across every step.
\* The lend adds one index and writes its charge; the written index is not in
\* the old set, so the sum there is unchanged and SumFunctionOnSetAddIndex
\* adds the new one.  The free removes an index and touches no charge, which
\* is SumFunctionOnSetRemoveIndex as it is written.  The return and the commit
\* move a buffer between two states that are both outstanding, so neither the
\* set nor the charges change and the primed total is the unprimed one by
\* substitution.  Everything else leaves all three variables alone.
LEMMA NextPreservesAccounting ==
    ASSUME TypeOK, MemoryAccountingExact, [Next]_vars
    PROVE  MemoryAccountingExact'
<1>1. CASE \E c \in CallIds, b \in BufferIds, msg \in Messages, ch \in Sizes :
              LendSendBuffer(c, b, msg, ch)
  <2>1. PICK c \in CallIds, b \in BufferIds, msg \in Messages, ch \in Sizes :
            LendSendBuffer(c, b, msg, ch)
    BY <1>1
  <2>2. /\ <<c, b>> \notin OutstandingPairs
        /\ OutstandingPairs' = OutstandingPairs \union {<<c, b>>}
    BY <2>1, SMTT(300)
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, OutstandingPairs, BufferOutstanding, IsFreshBuffer,
        IsLentBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>3. buffer_charge' = [buffer_charge EXCEPT ![<<c, b>>] = ch]
    BY <2>1, Zenon DEF LendSendBuffer
  <2>4. \A q \in OutstandingPairs \union {<<c, b>>} :
            buffer_charge'[q] \in Int
    BY <2>1, <2>3, ChargesAreNat, SMTT(300)
    DEF OutstandingPairs, Sizes, TypeOK, L0!TypeOK
  <2>5. SumFunctionOnSet(buffer_charge', OutstandingPairs \union {<<c, b>>})
            = buffer_charge'[<<c, b>>]
              + SumFunctionOnSet(buffer_charge', OutstandingPairs)
    BY <2>2, <2>4, PairSetsFinite, SumFunctionOnSetAddIndex
  <2>6. SumFunctionOnSet(buffer_charge', OutstandingPairs) = BytesOutstanding
    <3>1. \A q \in OutstandingPairs : buffer_charge'[q] = buffer_charge[q]
      BY <2>2, <2>3, SMT DEF OutstandingPairs, TypeOK, L0!TypeOK
    <3>2. QED
      BY <3>1, PairSetsFinite, SumFunctionOnSetEqual DEF BytesOutstanding
\* The primed total, written out.  A first-order application, so the prime
\* distributes: this is the step a fold with an operator argument cannot have.
  <2>65. BytesOutstanding'
             = SumFunctionOnSet(buffer_charge', OutstandingPairs')
    BY Zenon DEF BytesOutstanding
  <2>66. buffer_charge'[<<c, b>>] = ch
    BY <2>3, SMT DEF TypeOK, L0!TypeOK
  <2>67. SumFunctionOnSet(buffer_charge', OutstandingPairs')
             = SumFunctionOnSet(buffer_charge',
                                OutstandingPairs \union {<<c, b>>})
    BY <2>2, Zenon
  <2>68. memory_used' = memory_used + ch
    BY <2>1, Zenon DEF LendSendBuffer
\* Substitution first: the added index, the read-back, the agreement.  Chaining
\* equalities through SumFunctionOnSet needs congruence only, not the fact that
\* it returns a number.
  <2>685. SumFunctionOnSet(buffer_charge', OutstandingPairs')
              = ch + BytesOutstanding
    BY <2>5, <2>6, <2>66, <2>67, Zenon
\* Then the arithmetic, on summands now known to be numbers: commuting them is
\* all that is left, and it is the step that needs the type of the sum.
  <2>69. memory_used' = SumFunctionOnSet(buffer_charge', OutstandingPairs')
    BY <2>685, <2>68, BytesOutstandingType, SMTT(300)
    DEF MemoryAccountingExact, Sizes
  <2>7. QED
    BY <2>65, <2>69, Zenon DEF MemoryAccountingExact, BytesOutstanding
<1>2. CASE \E c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
  <2>1. PICK c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
    BY <1>2
  <2>2. /\ <<c, b>> \in OutstandingPairs
        /\ OutstandingPairs' = OutstandingPairs \ {<<c, b>>}
        /\ buffer_charge' = buffer_charge
    BY <2>1, SMTT(300)
    DEF FreeReturnedBuffer, OutstandingPairs, BufferOutstanding,
        IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>3. \A q \in OutstandingPairs : buffer_charge[q] \in Int
    BY ChargesAreNat, Zenon DEF OutstandingPairs
  <2>4. SumFunctionOnSet(buffer_charge, OutstandingPairs \ {<<c, b>>})
            = SumFunctionOnSet(buffer_charge, OutstandingPairs)
              - buffer_charge[<<c, b>>]
    BY <2>2, <2>3, PairSetsFinite, SumFunctionOnSetRemoveIndex
  <2>45. BytesOutstanding'
             = SumFunctionOnSet(buffer_charge', OutstandingPairs')
    BY Zenon DEF BytesOutstanding
  <2>46. SumFunctionOnSet(buffer_charge', OutstandingPairs')
             = SumFunctionOnSet(buffer_charge, OutstandingPairs \ {<<c, b>>})
    BY <2>2, Zenon
  <2>47. memory_used' = memory_used - buffer_charge[<<c, b>>]
    BY <2>1, Zenon DEF FreeReturnedBuffer
  <2>48. \A q \in OutstandingPairs : buffer_charge[q] \in Int
    BY ChargesAreNat, Zenon DEF OutstandingPairs
  <2>49. memory_used' = SumFunctionOnSet(buffer_charge', OutstandingPairs')
    BY <2>4, <2>46, <2>47, <2>48, SMTT(300)
    DEF MemoryAccountingExact, BytesOutstanding
  <2>5. QED
    BY <2>45, <2>49, Zenon DEF MemoryAccountingExact, BytesOutstanding
<1>3. CASE \/ \E c \in CallIds, b \in BufferIds : HostReturnsBuffer(c, b)
          \/ \E c \in CallIds, m \in Messages, b \in BufferIds :
                 SendMessage(c, m, b)
  <2>1. /\ OutstandingPairs' = OutstandingPairs
        /\ buffer_charge' = buffer_charge
        /\ memory_used' = memory_used
    BY <1>3, SMTT(300)
    DEF HostReturnsBuffer, SendMessage, OutstandingPairs, BufferOutstanding,
        IsLentBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>2. QED
    BY <2>1, Zenon DEF MemoryAccountingExact, BytesOutstanding
<1>4. CASE UNCHANGED <<buffer_state, buffer_charge, memory_used>>
  <2>1. OutstandingPairs' = OutstandingPairs
    BY <1>4, Zenon
    DEF OutstandingPairs, BufferOutstanding, IsLentBuffer, IsReturnedBuffer
  <2>2. QED
    BY <1>4, <2>1, Zenon DEF MemoryAccountingExact, BytesOutstanding
<1>5. QED
    BY <1>1, <1>2, <1>3, <1>4,
       OnlyBufferStepsWriteBufferStates, OnlyBudgetStepsWriteBudget, Zenon

\* The counter never passes the ceiling.  The lend's guard is the whole
\* argument; the free only subtracts, and nothing else moves it.
LEMMA NextPreservesCeiling ==
    ASSUME TypeOK, MemoryAccountingExact, MemoryWithinCeiling, [Next]_vars
    PROVE  MemoryWithinCeiling'
<1>1. CASE \E c \in CallIds, b \in BufferIds, msg \in Messages, ch \in Sizes :
              LendSendBuffer(c, b, msg, ch)
    BY <1>1, SMT
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsMemoryAvailable, MemoryWithinCeiling
<1>2. CASE \E c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
  <2>1. PICK c \in CallIds, b \in BufferIds : FreeReturnedBuffer(c, b)
    BY <1>2
  <2>2. buffer_charge[<<c, b>>] \in Nat
    BY SMT DEF TypeOK, L0!TypeOK
  <2>3. memory_used \in Int
    BY SMT DEF TypeOK, L0!TypeOK
  <2>35. memory_used' = memory_used - buffer_charge[<<c, b>>]
    BY <2>1, Zenon DEF FreeReturnedBuffer
  <2>36. memory_used <= Ceiling
    BY Zenon DEF MemoryWithinCeiling
  <2>37. memory_used' =< Ceiling
    BY <2>2, <2>3, <2>35, <2>36, CeilingIsPositive, SMT
    DEF CeilingIsPositive
  <2>4. QED
    BY <2>37, Zenon DEF MemoryWithinCeiling
<1>3. CASE UNCHANGED <<buffer_charge, memory_used>>
    BY <1>3, Zenon DEF MemoryWithinCeiling
<1>4. QED
    BY <1>1, <1>2, <1>3, OnlyBudgetStepsWriteBudget, Zenon

\* The three categories partition the buffers out, so their sums add up to the
\* total.  A lemma and not an invariant: it holds of any state, there being
\* nothing to preserve.  This is what the detailed observer publishes.
LEMMA CategoriesPartitionTotal ==
    ASSUME TypeOK
    PROVE  BytesHostLent + BytesSendInFlight + BytesRuntimeHeld
               = BytesOutstanding
<1>1. /\ LentPairs \cap InFlightPairs = {}
      /\ (LentPairs \union InFlightPairs) \cap HeldPairs = {}
    BY SMT
    DEF LentPairs, InFlightPairs, HeldPairs, IsLentBuffer, IsReturnedBuffer,
        TypeOK, L0!TypeOK
<1>2. OutstandingPairs = (LentPairs \union InFlightPairs) \union HeldPairs
    BY SMT
    DEF OutstandingPairs, LentPairs, InFlightPairs, HeldPairs,
        BufferOutstanding, IsLentBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
<1>3. IsFiniteSet(LentPairs \union InFlightPairs)
    BY PairSetsFinite, FS_Union
<1>4. /\ \A q \in LentPairs \union InFlightPairs : buffer_charge[q] \in Int
      /\ \A q \in (LentPairs \union InFlightPairs) \union HeldPairs :
             buffer_charge[q] \in Int
    BY ChargesAreNat, Zenon
    DEF LentPairs, InFlightPairs, HeldPairs
<1>5. SumFunctionOnSet(buffer_charge, LentPairs \union InFlightPairs)
          = BytesHostLent + BytesSendInFlight
    BY <1>1, <1>4, PairSetsFinite, SumFunctionOnSetDisjointUnion, Zenon
    DEF BytesHostLent, BytesSendInFlight
<1>6. SumFunctionOnSet(buffer_charge,
                       (LentPairs \union InFlightPairs) \union HeldPairs)
          = SumFunctionOnSet(buffer_charge, LentPairs \union InFlightPairs)
            + BytesRuntimeHeld
    BY <1>1, <1>3, <1>4, PairSetsFinite, SumFunctionOnSetDisjointUnion, Zenon
    DEF BytesRuntimeHeld
<1>7. QED
    BY <1>2, <1>5, <1>6, Zenon DEF BytesOutstanding

\* The shutdown signal has exactly two writers.
\* Only the two release steps write the release flags, which is what lets the
\* callback's stability be read off one framing fact.
LEMMA OnlyReleaseStepsWriteReleaseFlags ==
    ASSUME [Next]_vars
    PROVE  \/ \E rt \in RuntimeIds : EmitResourcesReleased(rt)
           \/ \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
           \/ UNCHANGED <<resources_released_emitted,
                          resources_released_callback_running>>
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent,
        l0_vars, L0!vars
<1>3. CASE NextFail \/ NextExplicitStutter
    BY <1>3, SMT
    DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
        RemainReleased, IsRuntimeDrained, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>4. CASE UNCHANGED vars
    BY <1>4, SMT DEF vars, l0_vars, L0!vars, ffi_vars
<1>5. QED
    BY <1>1, <1>2, <1>3, <1>4, NextDecomposition DEF NextByFootprint, NextSafe

LEMMA OnlyShutdownStepsWriteShutdownFlags ==
    ASSUME [Next]_vars
    PROVE  \/ \E rt \in RuntimeIds : EmitShutdownComplete(rt)
           \/ \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
           \/ UNCHANGED <<shutdown_event_emitted, shutdown_callback_running,
                          second_event_owed>>
<1>1. CASE NextSafeRefining
    BY <1>1, SMTT(120)
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        L0!ChannelCreate, L0!ChannelStartClosing,
        L0!ChannelFinishClosing, L0!CallStart, L0!SendMessage,
        L0!EndSend, L0!NetworkSend, L0!NetworkReceive,
        L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent,
        l0_vars, L0!vars
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, L0!RuntimeFail,
        L0!vars, l0_vars, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed,
        RemainReleased, L0!RemainFailed, L0!RemainReleased,
        L0!vars, l0_vars, ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, l0_vars, L0!vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* L0!NextSafe and L0!NextFail and the explicit stutters exhaust L0!Next.
\* Only the channel families and the runtime's own shutdown write a
\* channel's state; shutdown closes the channels of its runtime.
LEMMA OnlyChannelStepsWriteChannelState ==
    ASSUME [Next]_vars
    PROVE  \/ NextSafeChannelOnly
           \/ NextSafeChannelCall
           \/ NextSafeRuntimeChannel
           \/ UNCHANGED channel_state
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        L0!ChannelCreate, L0!ChannelStartClosing,
        L0!ChannelFinishClosing, L0!CallStart, L0!SendMessage,
        L0!EndSend, L0!NetworkSend, L0!NetworkReceive,
        L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, FfiOnlyStepsKeepL0, SMT DEF l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, L0!RuntimeFail,
        L0!vars, l0_vars, L0!ChannelVars, L0!CallVars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed,
        RemainReleased, L0!RemainFailed, L0!RemainReleased,
        L0!vars, l0_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, l0_vars, L0!vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* Ownership is written once, when the channel is created.
LEMMA OnlyChannelCreateWritesOwnership ==
    ASSUME [Next]_vars
    PROVE  \/ \E ch \in ChannelIds, rt \in RuntimeIds :
                  ChannelCreate(ch, rt)
           \/ UNCHANGED channel_runtime
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        L0!ChannelCreate, L0!ChannelStartClosing,
        L0!ChannelFinishClosing, L0!CallStart, L0!SendMessage,
        L0!EndSend, L0!NetworkSend, L0!NetworkReceive,
        L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, FfiOnlyStepsKeepL0, SMT DEF l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, L0!RuntimeFail,
        L0!vars, l0_vars, L0!ChannelVars, L0!CallVars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed,
        RemainReleased, L0!RemainFailed, L0!RemainReleased,
        L0!vars, l0_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, l0_vars, L0!vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

LEMMA VarsStutterProjects == UNCHANGED vars => UNCHANGED l0_vars
<1>1. QED
    BY SMT DEF vars, l0_vars, L0!vars, ffi_vars

LEMMA L0SafeIsNext == L0!NextSafe => L0!Next
<1>1. QED
    BY DEF L0!NextSafe, L0!NextSafeRuntimeOnly, L0!NextSafeRuntimeChannel,
           L0!NextSafeChannelOnly, L0!NextSafeChannelCall,
           L0!NextSafeCallOnly, L0!Next

LEMMA L0FailIsNext == L0!NextFail => L0!Next
<1>1. QED
    BY DEF L0!NextFail, L0!Next

LEMMA L0StutterIsNext == L0!NextExplicitStutter => L0!Next
<1>1. QED
    BY DEF L0!NextExplicitStutter, L0!Next

(***************************************************************************)
(* REFINEMENT - the step half                                              *)
(***************************************************************************)

THEOREM RefinesInit == Init => L0!Init
<1>1. QED
    BY DEF Init

THEOREM RefinesNext == [Next]_vars => [L0!Next]_l0_vars
<1>1. NextSafeRefining => L0!Next
    BY RefiningProjects, L0SafeIsNext
<1>2. NextSafeFfiOnly => UNCHANGED l0_vars
    BY FfiOnlyStutters
<1>3. NextFail => L0!Next
    BY FailProjects, L0FailIsNext
<1>4. NextExplicitStutter => L0!Next
    BY StutterProjects, L0StutterIsNext
<1>5. UNCHANGED vars => UNCHANGED l0_vars
    BY VarsStutterProjects
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

THEOREM RefinesSafeNext == [NextSafe]_vars => [L0!NextSafe]_l0_vars
<1>1. QED
    BY RefiningProjects, FfiOnlyStutters, VarsStutterProjects
    DEF NextSafe

(***************************************************************************)
(* TYPE PRESERVATION - the FFI half, by footprint                          *)
(* The level-0 half of TypeOK is L0!IndInvPreserved's business.            *)
(***************************************************************************)

FfiTypes ==
    /\ runtime_destroyed \in [RuntimeIds -> BOOLEAN]
    /\ buffers_held_by_host \in [CallIds -> Nat]
    /\ write_dones_emitted \in [CallIds -> Nat]
    /\ write_done_callback_running \in [CallIds -> BOOLEAN]
    /\ delivery_callback_running \in [CallIds -> BOOLEAN]
    /\ payloads_consumed_by_host \in [CallIds -> Nat]
    /\ handle_released \in [CallIds -> BOOLEAN]
    /\ cancel_requested \in [CallIds -> BOOLEAN]
    /\ shutdown_event_emitted \in [RuntimeIds -> BOOLEAN]
    /\ shutdown_callback_running \in [RuntimeIds -> BOOLEAN]
    /\ second_event_owed \in [RuntimeIds -> BOOLEAN]
    /\ resources_released_emitted \in [RuntimeIds -> BOOLEAN]
    /\ resources_released_callback_running \in [RuntimeIds -> BOOLEAN]
    /\ last_lend_status \in [CallIds \X Messages -> LendStatuses]
    /\ buffer_charge \in [CallIds \X BufferIds -> Nat]
    /\ buffer_length \in [CallIds \X BufferIds -> Nat]
    /\ memory_used \in Int

\* Kept apart from FfiTypes on purpose: the nested function space is the
\* only awkward typing in the state, and mixing it with ten flat conjuncts
\* is what put the type obligations out of the solver's reach.  The
\* send-to-buffer link travels with it: both are the buffer side.
BufferTypes ==
    /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
    /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]

LEMMA TypeOKSplit == TypeOK <=> L0!TypeOK /\ FfiTypes /\ BufferTypes
<1>1. QED
    BY DEF TypeOK, FfiTypes, BufferTypes

\* Actions that leave every FFI variable unchanged.
\* Reading back a nested EXCEPT, in the shape every call site has: the
\* write is a hypothesis and the conclusion is about the primed lookup, so
\* citing it needs neither a substitution nor a set instantiation.  A
\* version parameterised over the index sets asked Zenon to instantiate set
\* parameters, and one concluding about the EXCEPT term asked every site to
\* substitute first; both failed for those reasons rather than for content.
LEMMA BufferStateWriteLookup ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           NEW d \in CallIds, NEW e \in BufferIds,
           NEW v \in BufferStates,
           buffer_state' = [buffer_state EXCEPT ![d][e] = v],
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  (buffer_state[x][y])' =
               IF x = d /\ y = e THEN v ELSE buffer_state[x][y]
<1>1. QED
    BY SMT

LEMMA BufferSendWriteLookup ==
    ASSUME buffer_send \in [CallIds -> [BufferIds -> Nat]],
           NEW d \in CallIds, NEW e \in BufferIds, NEW v \in Nat,
           buffer_send' = [buffer_send EXCEPT ![d][e] = v],
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  (buffer_send[x][y])' =
               IF x = d /\ y = e THEN v ELSE buffer_send[x][y]
<1>1. QED
    BY SMTT(120)
\* One writer, one entry, and nothing else: stated with the typing and the
\* action as its only hypotheses, so citing it costs no instantiation in a
\* context carrying Next and three invariants.  Trying to do this inline was
\* what put the link preservation out of reach - the third time this session
\* that pulling a step out of the big context is what made it go through.
LEMMA LendMovesOneEntry ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           buffer_send \in [CallIds -> [BufferIds -> Nat]],
           NEW d \in CallIds, NEW e \in BufferIds,
           NEW msg \in Messages, NEW ch \in Sizes,
           LendSendBuffer(d, e, msg, ch),
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  /\ (buffer_state[x][y])' =
                 IF x = d /\ y = e THEN "lent"
                                     ELSE buffer_state[x][y]
           /\ (buffer_send[x][y])' = buffer_send[x][y]
           /\ (submitted[x])' = submitted[x]
           /\ (write_dones_emitted[x])' = write_dones_emitted[x]
           /\ buffer_state[d][e] = "none"
<1>1. buffer_state' = [buffer_state EXCEPT ![d][e] = "lent"]
    BY Zenon DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer
<1>2. /\ (buffer_send[x][y])' = buffer_send[x][y]
      /\ (write_dones_emitted[x])' = write_dones_emitted[x]
      /\ (submitted[x])' = submitted[x]
    BY SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer, l0_vars, L0!vars
<1>25. (buffer_state[x][y])' =
           IF x = d /\ y = e THEN "lent" ELSE buffer_state[x][y]
    BY <1>1, BufferStateWriteLookup DEF BufferStates
<1>26. buffer_state[d][e] \in BufferStates /\
           buffer_send[d][e] \in Nat
    BY Zenon
<1>3. QED
    BY <1>2, <1>25, SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer, BufferStates

LEMMA ReturnMovesOneEntry ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           buffer_send \in [CallIds -> [BufferIds -> Nat]],
           NEW d \in CallIds, NEW e \in BufferIds,
           HostReturnsBuffer(d, e),
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  /\ (buffer_state[x][y])' =
                 IF x = d /\ y = e THEN "returned"
                                     ELSE buffer_state[x][y]
           /\ (buffer_send[x][y])' = buffer_send[x][y]
           /\ (submitted[x])' = submitted[x]
           /\ (write_dones_emitted[x])' = write_dones_emitted[x]
<1>1. buffer_state' = [buffer_state EXCEPT ![d][e] = "returned"]
    BY Zenon DEF HostReturnsBuffer
<1>2. /\ (buffer_send[x][y])' = buffer_send[x][y]
      /\ (write_dones_emitted[x])' = write_dones_emitted[x]
      /\ (submitted[x])' = submitted[x]
    BY SMT DEF HostReturnsBuffer, l0_vars, L0!vars
<1>25. (buffer_state[x][y])' =
           IF x = d /\ y = e THEN "returned" ELSE buffer_state[x][y]
    BY <1>1, BufferStateWriteLookup DEF BufferStates
<1>26. buffer_state[d][e] \in BufferStates /\
           buffer_send[d][e] \in Nat
    BY Zenon
<1>3. QED
    BY <1>2, <1>25, SMT DEF HostReturnsBuffer, BufferStates

LEMMA FreeMovesOneEntry ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           buffer_send \in [CallIds -> [BufferIds -> Nat]],
           NEW d \in CallIds, NEW e \in BufferIds,
           FreeReturnedBuffer(d, e),
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  /\ (buffer_state[x][y])' =
                 IF x = d /\ y = e THEN "freed"
                                     ELSE buffer_state[x][y]
           /\ (buffer_send[x][y])' = buffer_send[x][y]
           /\ (submitted[x])' = submitted[x]
           /\ (write_dones_emitted[x])' = write_dones_emitted[x]
           /\ buffer_send[d][e] <= write_dones_emitted[d]
<1>1. buffer_state' = [buffer_state EXCEPT ![d][e] = "freed"]
    BY Zenon DEF FreeReturnedBuffer, CarriesNoUnacquittedSend
<1>2. /\ (buffer_send[x][y])' = buffer_send[x][y]
      /\ (write_dones_emitted[x])' = write_dones_emitted[x]
      /\ (submitted[x])' = submitted[x]
    BY SMT DEF FreeReturnedBuffer, CarriesNoUnacquittedSend, l0_vars, L0!vars
<1>25. (buffer_state[x][y])' =
           IF x = d /\ y = e THEN "freed" ELSE buffer_state[x][y]
    BY <1>1, BufferStateWriteLookup DEF BufferStates
<1>26. buffer_state[d][e] \in BufferStates /\
           buffer_send[d][e] \in Nat
    BY Zenon
<1>3. QED
    BY <1>2, <1>25, SMT DEF FreeReturnedBuffer, CarriesNoUnacquittedSend, BufferStates

\* Committing writes both the entry and the link, and appends the send whose
\* index it records.
LEMMA SendMovesOneEntry ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           buffer_send \in [CallIds -> [BufferIds -> Nat]],
           submitted \in [CallIds -> Seq(Messages)],
           NEW d \in CallIds, NEW m \in Messages,
           NEW e \in BufferIds,
           SendMessage(d, m, e),
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  /\ (buffer_state[x][y])' =
                 IF x = d /\ y = e THEN "returned"
                                     ELSE buffer_state[x][y]
           /\ (buffer_send[x][y])' =
                 IF x = d /\ y = e THEN Len(submitted[d]) + 1
                                     ELSE buffer_send[x][y]
           /\ Len((submitted[x])') =
                 IF x = d THEN Len(submitted[d]) + 1 ELSE Len(submitted[x])
           /\ (write_dones_emitted[x])' = write_dones_emitted[x]
<1>1. /\ buffer_state' = [buffer_state EXCEPT ![d][e] = "returned"]
      /\ buffer_send' = [buffer_send EXCEPT ![d][e] = Len(submitted[d]) + 1]
      /\ submitted' = [submitted EXCEPT ![d] = Append(submitted[d], m)]
      /\ UNCHANGED write_dones_emitted
    BY Zenon DEF SendMessage, L0!SendMessage, l0_vars, L0!vars
<1>2. Len(submitted[d]) + 1 \in Nat
    BY LenProperties, SMT
<1>3. Len(Append(submitted[d], m)) = Len(submitted[d]) + 1
    BY AppendProperties, SMT
<1>4. QED
    BY <1>1, <1>2, <1>3, BufferStateWriteLookup, BufferSendWriteLookup, SMT
    DEF BufferStates

\* Emitting an acquittal moves the count of one call and nothing else.
LEMMA EmitMovesOneCount ==
    ASSUME buffer_state \in [CallIds -> [BufferIds -> BufferStates]],
           buffer_send \in [CallIds -> [BufferIds -> Nat]],
           write_dones_emitted \in [CallIds -> Nat],
           NEW d \in CallIds, EmitWriteDone(d),
           NEW x \in CallIds, NEW y \in BufferIds
    PROVE  /\ (buffer_state[x][y])' = buffer_state[x][y]
           /\ (buffer_send[x][y])' = buffer_send[x][y]
           /\ (submitted[x])' = submitted[x]
\* The primed count is typed here rather than at each use: without it a
\* caller has an inequality between terms it cannot know are numbers.
           /\ (write_dones_emitted[x])' \in Nat
           /\ (write_dones_emitted[x])' >= write_dones_emitted[x]
<1>1. /\ write_dones_emitted' =
             [write_dones_emitted EXCEPT ![d] = write_dones_emitted[d] + 1]
      /\ UNCHANGED <<buffer_state, buffer_send, submitted>>
    BY Zenon DEF EmitWriteDone, l0_vars, L0!vars
<1>2. QED
    BY <1>1, SMT

\* The nested EXCEPT is the only awkward typing in the buffer states, and
\* it is the same in all three writers, so it is proved once.
LEMMA BufferStateUpdateStaysTyped ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds,
           NEW s \in BufferStates,
           buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
    PROVE  [buffer_state EXCEPT ![cId][b] = s] \in [CallIds -> [BufferIds -> BufferStates]]
<1>1. QED
    BY SMT

LEMMA UnchangedFfiPreservesFfiTypes ==
    FfiTypes /\ UNCHANGED ffi_vars => FfiTypes'
<1>1. QED
    BY SMT DEF FfiTypes, ffi_vars

\* The delivery cases add the fresh index Len(events_delivered)+1 to the
\* payload set, so they need the level-0 typing of events_delivered and
\* the finite-set lemmas; hence the TypeOK hypothesis.
LEMMA RefiningPreservesFfiTypes ==
    TypeOK /\ NextSafeRefining => FfiTypes'
<1>1. ASSUME TypeOK, NextSafeRefining
      PROVE  FfiTypes'
  <2>0. FfiTypes
    BY <1>1, TypeOKSplit
  <2>1. CASE NextSafeRuntimeOnly
    BY <2>0, <2>1, UnchangedFfiPreservesFfiTypes
    DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease
  <2>2. CASE NextSafeRuntimeChannel
    BY <2>0, <2>2, SMT
    DEF NextSafeRuntimeChannel, RuntimeBeginShutdown, RequestCancellationOfActiveCalls,
        FfiTypes, L0!ChannelsOf, L0!IsActiveCall, L0!ActiveCallStates
  <2>3. CASE NextSafeChannelOnly
    BY <2>0, <2>3, SMT
    DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
        RequestCancellationOfActiveCalls, ffi_vars,
        FfiTypes, L0!IsActiveCall, L0!ActiveCallStates
  <2>4. CASE NextSafeChannelCall
    BY <2>0, <2>4, UnchangedFfiPreservesFfiTypes
    DEF NextSafeChannelCall, ChannelFinishClosing
  <2>5. CASE NextSafeCallOnly
    <3>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
      BY <2>0, <3>1, UnchangedFfiPreservesFfiTypes DEF CallStart
    \* The commit decrements the held level, which the guard keeps above
    \* zero, so the level stays a natural.
    <3>2. CASE \E cId \in CallIds, msg \in Messages,
                  bs \in BufferIds : SendMessage(cId, msg, bs)
      BY <2>0, <3>2, SMT
      DEF SendMessage, HostHoldsSomeBuffer, BufferStates, IsFreshBuffer, IsLentBuffer, IsReturnedBuffer,
          FfiTypes
    <3>3. CASE \E cId \in CallIds : EndSend(cId)
      BY <2>0, <3>3, UnchangedFfiPreservesFfiTypes DEF EndSend
    <3>4. CASE \E cId \in CallIds : NetworkSend(cId)
      BY <2>0, <3>4, UnchangedFfiPreservesFfiTypes DEF NetworkSend
    <3>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
      BY <2>0, <3>5, UnchangedFfiPreservesFfiTypes DEF NetworkReceive
    <3>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
      BY <2>0, <3>6, UnchangedFfiPreservesFfiTypes DEF ReceiveStatus
    <3>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
      BY <1>1, <2>0, <3>7, FS_AddElement, SMTT(120)
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
          HandPayloadToHost, HasFreeDeliverySlot, FfiTypes,
          TypeOK, L0!TypeOK
    <3>8. CASE \E cId \in CallIds : DeliverMessage(cId)
      BY <1>1, <2>0, <3>8, FS_AddElement, SMTT(120)
      DEF DeliverMessage, L0!DeliverMessage,
          HandPayloadToHost, HasFreeDeliverySlot, FfiTypes,
          TypeOK, L0!TypeOK
    <3>9. CASE \E cId \in CallIds : DeliverStatus(cId)
      BY <1>1, <2>0, <3>9, FS_AddElement, SMTT(120)
      DEF DeliverStatus, L0!DeliverStatus,
          HandPayloadToHost, HasFreeDeliverySlotForTerminal, FfiTypes,
          TypeOK, L0!TypeOK
    <3>10. CASE \E cId \in CallIds : DeliverCancelled(cId)
      BY <1>1, <2>0, <3>10, FS_AddElement, SMTT(120)
      DEF DeliverCancelled, L0!CallCancel,
          HandPayloadToHost, HasFreeDeliverySlotForTerminal, FfiTypes,
          TypeOK, L0!TypeOK
    <3>11. QED BY <1>1, <2>5, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>7,
                   <3>8, <3>9, <3>10 DEF NextSafeCallOnly
  <2>6. QED BY <1>1, <2>0, <2>1, <2>2, <2>3, <2>4, <2>5 DEF NextSafeRefining
<1>2. QED BY <1>1

LEMMA FfiOnlyPreservesFfiTypes ==
    FfiTypes /\ NextSafeFfiOnly => FfiTypes'
<1>1. ASSUME FfiTypes, NextSafeFfiOnly
      PROVE  FfiTypes'
  <2>1. CASE NextSafeShutdownFfi
    BY <1>1, <2>1, SMT
    DEF NextSafeShutdownFfi, EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
    ResourcesReleasedCallbackReturns, EmitResourcesReleased,
    ResourcesReleasedCallbackReturns,
        RuntimeDestroy, IsRuntimeDrained, FfiTypes
  <2>2. CASE NextSafeCallFfi
    <3>1. CASE \E cId \in CallIds : RequestCallCancellation(cId)
      BY <1>1, <3>1, SMT DEF RequestCallCancellation, FfiTypes
    <3>2. CASE \E cId \in CallIds : ReleaseCallHandle(cId)
      BY <1>1, <3>2, SMT DEF ReleaseCallHandle, FfiTypes
    <3>3. CASE \E cId \in CallIds : EmitWriteDone(cId)
      BY <1>1, <3>3, SMT DEF EmitWriteDone, FfiTypes
    <3>4. CASE \E cId \in CallIds : WriteDoneReturns(cId)
      BY <1>1, <3>4, SMT DEF WriteDoneReturns, FfiTypes
    <3>5. CASE \E cId \in CallIds : DeliveryCallbackReturns(cId)
      BY <1>1, <3>5, SMT DEF DeliveryCallbackReturns, FfiTypes
    <3>6. CASE \E cId \in CallIds : HostConsumesEvent(cId)
      BY <1>1, <3>6, SMT DEF HostConsumesEvent, FfiTypes
    <3>60. CASE \E cId \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                   LendSendBuffer(cId, bb, msg, ch)
      BY <1>1, <3>60, MessageLengthIsNat, SMTT(120)
      DEF LendSendBuffer, FfiTypes, Sizes, LendStatuses, MessageLengthIsNat
    <3>61. CASE \E cId \in CallIds, bb \in BufferIds :
                   HostReturnsBuffer(cId, bb)
      BY <1>1, <3>61, SMT DEF HostReturnsBuffer, HostHoldsSomeBuffer, FfiTypes
    <3>62. CASE \E cId \in CallIds, bb \in BufferIds :
                   FreeReturnedBuffer(cId, bb)
      BY <1>1, <3>62, SMTT(120) DEF FreeReturnedBuffer, FfiTypes
    <3>63. CASE \E cId \in CallIds, msg \in Messages :
                   RefuseLendTooLarge(cId, msg)
      BY <1>1, <3>63, SMTT(120) DEF RefuseLendTooLarge, FfiTypes, LendStatuses
    <3>64. CASE \E cId \in CallIds, msg \in Messages :
                   RefuseLendForSlot(cId, msg)
      BY <1>1, <3>64, SMTT(120) DEF RefuseLendForSlot, FfiTypes, LendStatuses
    <3>65. CASE \E cId \in CallIds, msg \in Messages, charge \in Sizes :
                   RefuseLendForBudget(cId, msg, charge)
      BY <1>1, <3>65, SMTT(120) DEF RefuseLendForBudget, FfiTypes, LendStatuses
    <3>7. QED BY <1>1, <2>2, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>60,
                  <3>61, <3>62, <3>63, <3>64, <3>65 DEF NextSafeCallFfi
  <2>3. QED BY <1>1, <2>1, <2>2 DEF NextSafeFfiOnly
<1>2. QED BY <1>1

LEMMA NextPreservesFfiTypes == TypeOK /\ Next => FfiTypes'
<1>1. ASSUME TypeOK, Next
      PROVE  FfiTypes'
  <2>0. FfiTypes
    BY <1>1, TypeOKSplit
  <2>1. CASE NextSafeRefining
    BY <1>1, <2>1, RefiningPreservesFfiTypes
  <2>2. CASE NextSafeFfiOnly
    BY <2>0, <2>2, FfiOnlyPreservesFfiTypes
  <2>3. CASE NextFail
    BY <2>0, <2>3, UnchangedFfiPreservesFfiTypes
    DEF NextFail, RuntimeFail
  <2>4. CASE NextExplicitStutter
    BY <2>0, <2>4, UnchangedFfiPreservesFfiTypes
    DEF NextExplicitStutter, RemainFailed, RemainReleased
  <2>5. QED BY <1>1, <2>0, <2>1, <2>2, <2>3, <2>4, NextDecomposition
        DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

\* Only the three buffer actions touch buffer_state, and each writes one
\* entry, so its type is preserved by the factored EXCEPT lemma and by
\* nothing else having to be said.
LEMMA NextPreservesBufferTypes == TypeOK /\ Next => BufferTypes'
<1>1. ASSUME TypeOK, Next
      PROVE  (buffer_state')\in [CallIds -> [BufferIds -> BufferStates]]
  <2>0. BufferTypes
    BY <1>1, TypeOKSplit
  <2>1. CASE \E cId \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                LendSendBuffer(cId, bb, msg, ch)
    <3>1. PICK c0 \in CallIds, b0 \in BufferIds, msg \in Messages, ch \in Sizes :
              LendSendBuffer(c0, b0, msg, ch)
      BY <2>1
    <3>2. buffer_state' = [buffer_state EXCEPT ![c0][b0] = "lent"]
      BY <3>1, Zenon DEF LendSendBuffer
    <3>3. QED
      BY <2>0, <3>2, BufferStateUpdateStaysTyped, Zenon
      DEF BufferTypes, BufferStates
  <2>2. CASE \E cId \in CallIds, bb \in BufferIds :
                HostReturnsBuffer(cId, bb)
    <3>1. PICK c0 \in CallIds, b0 \in BufferIds :
              HostReturnsBuffer(c0, b0)
      BY <2>2
    <3>2. buffer_state' = [buffer_state EXCEPT ![c0][b0] = "returned"]
      BY <3>1, Zenon DEF HostReturnsBuffer
    <3>3. QED
      BY <2>0, <3>2, BufferStateUpdateStaysTyped, Zenon
      DEF BufferTypes, BufferStates
  <2>3. CASE \E cId \in CallIds, bb \in BufferIds :
                FreeReturnedBuffer(cId, bb)
    <3>1. PICK c0 \in CallIds, b0 \in BufferIds :
              FreeReturnedBuffer(c0, b0)
      BY <2>3
    <3>2. buffer_state' = [buffer_state EXCEPT ![c0][b0] = "freed"]
      BY <3>1, Zenon DEF FreeReturnedBuffer
    <3>3. QED
      BY <2>0, <3>2, BufferStateUpdateStaysTyped, Zenon
      DEF BufferTypes, BufferStates
  <2>4. CASE \E c \in CallIds, m \in Messages,
                bs \in BufferIds : SendMessage(c, m, bs)
    <3>1. PICK c0 \in CallIds, m0 \in Messages, bs \in BufferIds :
              SendMessage(c0, m0, bs)
      BY <2>4
    <3>2. PICK b0 \in BufferIds :
              buffer_state' = [buffer_state EXCEPT ![c0][b0] = "returned"]
      BY <3>1, Zenon DEF SendMessage
    <3>3. QED
      BY <2>0, <3>2, BufferStateUpdateStaysTyped, Zenon
      DEF BufferTypes, BufferStates
  <2>5. CASE UNCHANGED buffer_state
    BY <2>0, <2>5, Zenon DEF BufferTypes
  <2>6. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, OnlyBufferStepsWriteBufferStates,
       Zenon
\* The link has one writer, and it writes one entry.
<1>2. ASSUME TypeOK, Next
      PROVE  (buffer_send') \in [CallIds -> [BufferIds -> Nat]]
  <2>0. buffer_send \in [CallIds -> [BufferIds -> Nat]]
    BY <1>2, TypeOKSplit DEF BufferTypes
  <2>1. CASE \E c \in CallIds, m \in Messages,
                bs \in BufferIds : SendMessage(c, m, bs)
    <3>1. PICK c0 \in CallIds, m0 \in Messages, b0 \in BufferIds :
              SendMessage(c0, m0, b0)
      BY <2>1
    <3>2. buffer_send' =
              [buffer_send EXCEPT ![c0][b0] = Len(submitted[c0]) + 1]
      BY <3>1, Zenon DEF SendMessage
    <3>3. Len(submitted[c0]) + 1 \in Nat
      BY <1>2, LenProperties, SMT DEF TypeOK, L0!TypeOK
    <3>4. QED
      BY <2>0, <3>2, <3>3, SMT
  <2>2. CASE UNCHANGED buffer_send
    BY <2>0, <2>2, Zenon
  <2>3. QED
    BY <1>2, <2>1, <2>2, OnlySendMessageWritesBufferSend, Zenon
<1>3. QED BY <1>1, <1>2, Zenon DEF BufferTypes

(***************************************************************************)
(* FFI CALL INVARIANTS PRESERVED, BY FOOTPRINT                             *)
(* Guard-based only: no NotFailed hypothesis anywhere in this section,     *)
(* which is what lets IndInv carry FfiCallInv outside the umbrella.        *)
(***************************************************************************)

\* A step that leaves the level-0 state alone leaves every level-0
\* reading alone.  The FFI-only cases consume this instead of expanding
\* the twelve-variable tuple inside each invariant obligation.
LEMMA L0FrameTransfers ==
    ASSUME UNCHANGED l0_vars
    PROVE  /\ \A c \in CallIds :
                  /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
                  /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
                  /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
                  /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                  /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                  /\ (events_delivered[c])' = events_delivered[c]
                  /\ Len((submitted[c])') = Len(submitted[c])
                  /\ (call_channel[c])' = call_channel[c]
           /\ \A chan \in ChannelIds :
                  (IsClosingChannel(chan))' = IsClosingChannel(chan)
<1>1. UNCHANGED <<call_state, call_channel, channel_state, channel_runtime,
                  submitted, sent, received, delivered, events_delivered,
                  send_closed, status_pending, runtime_state>>
    BY SMT DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars,
        L0!CallVars
<1>2. ASSUME NEW c \in CallIds
      PROVE  /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
    BY <1>1, SMT DEF L0!IsActiveCall, L0!IsTerminalCall, L0!IsUnusedCall,
        L0!HasStatus
<1>3. ASSUME NEW chan \in ChannelIds
      PROVE  (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, SMT
<1>4. QED
    BY <1>2, <1>3, Zenon

\* The predicate-level deltas of the FFI-only steps.  Each pairs with
\* L0FrameTransfers: the level-0 side is frozen, so only the touched FFI
\* component moves and the preservation cases read both as plain facts.
\* One frame per state component: a step that leaves the component alone
\* leaves every reading of it alone.  The delta lemmas compose the frames
\* of what they do not touch and prove only what they do, so no single
\* obligation ever mixes a cardinality with a function update.
\* The debt is read against two variables - the events delivered and the
\* payloads released - so the frame needs both, exactly as the send-side
\* frame needs submitted alongside the acquittal counter.
LEMMA PayloadReadingsFrame ==
    ASSUME UNCHANGED <<payloads_consumed_by_host, events_delivered>>
    PROVE  \A c \in CallIds :
               /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
               /\ (OwedPayloads(c))' = OwedPayloads(c)
               /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
               /\ (HostOwnsSomePayload(c))' = HostOwnsSomePayload(c)
               /\ (HostHasDeliveryCredit(c))' = HostHasDeliveryCredit(c)
               /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
               /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                      HostOwnsAtMostCreditsPlusOne(c)
<1>1. QED
    BY SMT

LEMMA SendReadingsFrame ==
    ASSUME UNCHANGED <<write_dones_emitted, write_done_callback_running,
                       submitted>>
    PROVE  \A c \in CallIds :
               /\ (write_dones_emitted[c])' = write_dones_emitted[c]
               /\ (IsWriteDoneCallbackRunning(c))' =
                      IsWriteDoneCallbackRunning(c)
               /\ (WriteDonesReturned(c))' = WriteDonesReturned(c)
               /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
<1>1. QED
    BY SMT

\* The buffer level alone: the two actions that move it leave the send
\* counter and submitted alone, so they read the send side through the
\* frame above and prove only the level they touch.
LEMMA BufferReadingsFrame ==
    ASSUME UNCHANGED buffers_held_by_host
    PROVE  \A c \in CallIds :
               /\ (buffers_held_by_host[c])' = buffers_held_by_host[c]
               /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
               /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
<1>1. QED
    BY SMT DEF HostHoldsSomeBuffer, HostHoldsNoBuffer

\* The slot count reads both components, so its frame is the conjunction
\* of theirs.  A step that moves one of them proves this reading by hand,
\* which is where the bound is actually maintained.
LEMMA SendWindowFrame ==
    ASSUME UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running, submitted>>
    PROVE  \A c \in CallIds :
               (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
<1>1. QED
    BY SMT DEF SendWindowOccupancy, WriteDonesReturned

\* The outstanding count is an integer.  Stated apart because the cases
\* that hide the send predicates still have to do arithmetic on it.
LEMMA SendWindowCountsAreIntegers ==
    ASSUME TypeOK
    PROVE  \A c \in CallIds :
               /\ SendWindowOccupancy(c) \in Int
               /\ WriteDonesReturned(c) \in Int
               /\ buffers_held_by_host[c] \in Nat
               /\ write_dones_emitted[c] \in Nat
<1>1. /\ buffers_held_by_host \in [CallIds -> Nat]
      /\ write_dones_emitted \in [CallIds -> Nat]
      /\ write_done_callback_running \in [CallIds -> BOOLEAN]
      /\ submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>2. \A c \in CallIds : Len(submitted[c]) \in Nat
    BY <1>1, LenProperties, Zenon
<1>3. QED
    BY <1>1, <1>2, SMT

LEMMA HandleFlagReadingsFrame ==
    ASSUME UNCHANGED handle_released
    PROVE  \A c \in CallIds : (IsHandleReleased(c))' = IsHandleReleased(c)
<1>1. QED
    BY SMT

LEMMA CancelFlagReadingsFrame ==
    ASSUME UNCHANGED cancel_requested
    PROVE  \A c \in CallIds : (IsCancelRequested(c))' = IsCancelRequested(c)
<1>1. QED
    BY SMT

LEMMA DeliveryFlagReadingsFrame ==
    ASSUME UNCHANGED delivery_callback_running
    PROVE  \A c \in CallIds :
               (IsDeliveryCallbackRunning(c))' = IsDeliveryCallbackRunning(c)
<1>1. QED
    BY SMT

LEMMA CancelRequestTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, RequestCallCancellation(cId)
    PROVE  /\ ~IsHandleReleased(cId)
           /\ ~L0!IsUnusedCall(cId)
           /\ \A c \in CallIds :
                  /\ (IsCancelRequested(c))' =
                         (IF c = cId THEN TRUE ELSE IsCancelRequested(c))
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
                  /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                  /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                  /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
<1>1. /\ cancel_requested' = [cancel_requested EXCEPT ![cId] = TRUE]
      /\ ~handle_released[cId]
      /\ ~L0!IsUnusedCall(cId)
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     write_done_callback_running,
                     delivery_callback_running,
                     payloads_consumed_by_host, handle_released>>
    BY SMT DEF RequestCallCancellation, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>15. cancel_requested \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK
<1>2. \A c \in CallIds :
          (IsCancelRequested(c))' =
              (IF c = cId THEN TRUE ELSE IsCancelRequested(c))
    BY <1>1, <1>15, Zenon
<1>3. \A c \in CallIds :
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' =
                 IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY <1>1, PayloadReadingsFrame, SendReadingsFrame, BufferReadingsFrame,
       SendWindowFrame,
       HandleFlagReadingsFrame, DeliveryFlagReadingsFrame, Zenon
<1>4. QED
    BY <1>1, <1>2, <1>3, Zenon

\* Release exports its guards as well as its frame: the four conjuncts of
\* ReleasedCallIsClean hold for cId in the pre-state, and nothing this
\* step touches can disturb any of them.
LEMMA ReleaseTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, ReleaseCallHandle(cId)
    PROVE  /\ ~L0!IsUnusedCall(cId)
           /\ ~L0!IsActiveCall(cId)
           /\ HostOwnsNoPayload(cId)
           /\ HostHoldsNoBuffer(cId)
           /\ \A c \in CallIds :
               /\ (IsCancelRequested(c))' = IsCancelRequested(c)
               /\ (IsHandleReleased(c))' =
                      (IF c = cId THEN TRUE ELSE IsHandleReleased(c))
               /\ (IsDeliveryCallbackRunning(c))' =
                      IsDeliveryCallbackRunning(c)
               /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
               /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
               /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                      HostOwnsAtMostCreditsPlusOne(c)
               /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
               /\ (IsWriteDoneCallbackRunning(c))' =
                      IsWriteDoneCallbackRunning(c)
               /\ (write_dones_emitted[c])' = write_dones_emitted[c]
               /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
               /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
               /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
               /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
<1>1. /\ handle_released' = [handle_released EXCEPT ![cId] = TRUE]
      /\ ~L0!IsUnusedCall(cId)
      /\ ~L0!IsActiveCall(cId)
      /\ HostOwnsNoPayload(cId)
      /\ HostHoldsNoBuffer(cId)
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     write_done_callback_running,
                     delivery_callback_running,
                     payloads_consumed_by_host, cancel_requested>>
    BY SMT DEF ReleaseCallHandle, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>15. handle_released \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK
<1>2. \A c \in CallIds :
          (IsHandleReleased(c))' =
              (IF c = cId THEN TRUE ELSE IsHandleReleased(c))
    BY <1>1, <1>15, Zenon
<1>3. \A c \in CallIds :
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsDeliveryCallbackRunning(c))' =
                 IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY <1>1, PayloadReadingsFrame, SendReadingsFrame, BufferReadingsFrame,
       SendWindowFrame,
       CancelFlagReadingsFrame, DeliveryFlagReadingsFrame, Zenon
<1>4. QED
    BY <1>1, <1>2, <1>3, Zenon

\* Lending charges one slot and touches nothing else.  Its two guards are
\* what the preservation cases need: the call is started, so it is not
\* unused, and the handle is live, so a released call never grows a debt.
LEMMA LendBufferTransfers ==
    ASSUME NEW cId \in CallIds, NEW bb \in BufferIds,
           NEW msg \in Messages, NEW ch \in Sizes, TypeOK,
           LendSendBuffer(cId, bb, msg, ch)
    PROVE  /\ L0!IsActiveCall(cId)
           /\ ~L0!IsUnusedCall(cId)
           /\ ~IsHandleReleased(cId)
           /\ SendWindowOccupancy(cId) < MaxSendsInFlight
           /\ \A c \in CallIds :
                  /\ (SendWindowOccupancy(c))' =
                         (IF c = cId THEN SendWindowOccupancy(c) + 1
                          ELSE SendWindowOccupancy(c))
                  /\ (c # cId =>
                         /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                         /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
<1>1. /\ buffers_held_by_host' =
             [buffers_held_by_host EXCEPT ![cId] = @ + 1]
      /\ L0!IsActiveCall(cId)
      /\ ~IsHandleReleased(cId)
      /\ SendWindowOccupancy(cId) < MaxSendsInFlight
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                     delivery_callback_running, handle_released,
                     cancel_requested>>
    BY SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HasFreeSendSlot, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>12. ~L0!IsUnusedCall(cId)
    BY <1>1, SMT DEF L0!IsActiveCall, L0!ActiveCallStates, L0!IsUnusedCall
<1>2. buffers_held_by_host \in [CallIds -> Nat]
    BY Zenon DEF TypeOK
<1>3. \A c \in CallIds :
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (WriteDonesReturned(c))' = WriteDonesReturned(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' = IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
    BY <1>1, PayloadReadingsFrame, SendReadingsFrame,
       CancelFlagReadingsFrame, HandleFlagReadingsFrame,
       DeliveryFlagReadingsFrame, Zenon
<1>4. ASSUME NEW c \in CallIds
      PROVE  /\ (buffers_held_by_host[c])' =
                    (IF c = cId THEN buffers_held_by_host[c] + 1
                     ELSE buffers_held_by_host[c])
             /\ (c # cId =>
                    /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                    /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
    BY <1>1, <1>2, Zenon DEF HostHoldsNoBuffer, HostHoldsSomeBuffer
<1>5. ASSUME NEW c \in CallIds
      PROVE  (SendWindowOccupancy(c))' =
                 (IF c = cId THEN SendWindowOccupancy(c) + 1
                  ELSE SendWindowOccupancy(c))
  <2>1. /\ Len((submitted[c])') = Len(submitted[c])
        /\ (WriteDonesReturned(c))' = WriteDonesReturned(c)
        /\ (buffers_held_by_host[c])' =
               (IF c = cId THEN buffers_held_by_host[c] + 1
                ELSE buffers_held_by_host[c])
    BY <1>1, <1>3, <1>4, Zenon
  <2>2. submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>3. Len(submitted[c]) \in Nat
    BY <2>2, LenProperties, Zenon
  <2>4. /\ buffers_held_by_host[c] \in Nat
        /\ write_dones_emitted[c] \in Nat
    BY SendWindowCountsAreIntegers, Zenon
  <2>45. /\ Len((submitted[c])') = Len(submitted[c])
         /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>1, <1>3, Zenon
  <2>5. QED
    BY <2>1, <2>3, <2>4, <2>45, SMT DEF SendWindowOccupancy
<1>6. QED
    BY <1>1, <1>12, <1>3, <1>4, <1>5, Zenon

\* Returning a buffer gives the slot back and touches nothing else.  It
\* carries no guard beyond holding one, which is what makes it the exit
\* every ending of a call can take.
LEMMA ReturnBufferTransfers ==
    ASSUME NEW cId \in CallIds, NEW bb \in BufferIds, TypeOK,
           HostReturnsBuffer(cId, bb)
    PROVE  /\ HostHoldsSomeBuffer(cId)
           /\ \A c \in CallIds :
                  /\ (SendWindowOccupancy(c))' =
                         (IF c = cId THEN SendWindowOccupancy(c) - 1
                          ELSE SendWindowOccupancy(c))
                  /\ (c # cId =>
                         /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                         /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
<1>1. /\ buffers_held_by_host' =
             [buffers_held_by_host EXCEPT ![cId] = @ - 1]
      /\ HostHoldsSomeBuffer(cId)
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                     delivery_callback_running, handle_released,
                     cancel_requested>>
    BY SMT DEF HostReturnsBuffer, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars
<1>2. buffers_held_by_host \in [CallIds -> Nat]
    BY Zenon DEF TypeOK
<1>3. \A c \in CallIds :
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (WriteDonesReturned(c))' = WriteDonesReturned(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' = IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
    BY <1>1, PayloadReadingsFrame, SendReadingsFrame,
       CancelFlagReadingsFrame, HandleFlagReadingsFrame,
       DeliveryFlagReadingsFrame, Zenon
<1>4. ASSUME NEW c \in CallIds
      PROVE  /\ (buffers_held_by_host[c])' =
                    (IF c = cId THEN buffers_held_by_host[c] - 1
                     ELSE buffers_held_by_host[c])
             /\ (c # cId =>
                    /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                    /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
    BY <1>1, <1>2, Zenon DEF HostHoldsNoBuffer, HostHoldsSomeBuffer
<1>5. ASSUME NEW c \in CallIds
      PROVE  (SendWindowOccupancy(c))' =
                 (IF c = cId THEN SendWindowOccupancy(c) - 1
                  ELSE SendWindowOccupancy(c))
  <2>1. /\ Len((submitted[c])') = Len(submitted[c])
        /\ (WriteDonesReturned(c))' = WriteDonesReturned(c)
        /\ (buffers_held_by_host[c])' =
               (IF c = cId THEN buffers_held_by_host[c] - 1
                ELSE buffers_held_by_host[c])
    BY <1>1, <1>3, <1>4, Zenon
  <2>2. submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>3. Len(submitted[c]) \in Nat
    BY <2>2, LenProperties, Zenon
  <2>4. /\ buffers_held_by_host[c] \in Nat
        /\ write_dones_emitted[c] \in Nat
    BY SendWindowCountsAreIntegers, Zenon
  <2>45. /\ Len((submitted[c])') = Len(submitted[c])
         /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>1, <1>3, Zenon
  <2>5. QED
    BY <2>1, <2>3, <2>4, <2>45, SMT DEF SendWindowOccupancy
<1>6. QED
    BY <1>1, <1>3, <1>4, <1>5, Zenon

\* Emission takes the acquittal onto the host stack and frees the slot in
\* the same step: the network has the bytes, so the window shrinks now
\* rather than when the callback returns - which the host could not
\* observe, and would have had to wait for.
LEMMA EmitWriteDoneTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, EmitWriteDone(cId)
    PROVE  /\ write_dones_emitted[cId] < Len(submitted[cId])
           /\ ~HasNoSendInFlight(cId)
           /\ \A c \in CallIds :
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         (IF c = cId THEN TRUE
                          ELSE IsWriteDoneCallbackRunning(c))
                  /\ (write_dones_emitted[c])' =
                         IF c = cId THEN write_dones_emitted[c] + 1
                         ELSE write_dones_emitted[c]
                  /\ (SendWindowOccupancy(c))' =
                         (IF c = cId THEN SendWindowOccupancy(c) - 1
                          ELSE SendWindowOccupancy(c))
                  /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                  /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                  /\ (HasNoSendInFlight(c))' =
                         (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
<1>1. /\ write_dones_emitted' =
             [write_dones_emitted EXCEPT ![cId] = @ + 1]
      /\ write_done_callback_running' =
             [write_done_callback_running EXCEPT ![cId] = TRUE]
      /\ write_dones_emitted[cId] < Len(submitted[cId])
      /\ ~write_done_callback_running[cId]
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<buffers_held_by_host, delivery_callback_running,
                     payloads_consumed_by_host,
                     handle_released, cancel_requested>>
    BY SMT DEF EmitWriteDone, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>2. /\ write_dones_emitted \in [CallIds -> Nat]
      /\ write_done_callback_running \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK
<1>3. \A c \in CallIds :
          /\ (write_dones_emitted[c])' =
                 (IF c = cId THEN write_dones_emitted[c] + 1
                  ELSE write_dones_emitted[c])
          /\ (IsWriteDoneCallbackRunning(c))' =
                 (IF c = cId THEN TRUE
                  ELSE IsWriteDoneCallbackRunning(c))
    BY <1>1, <1>2, Zenon
<1>35. \A c \in CallIds :
          /\ (buffers_held_by_host[c])' = buffers_held_by_host[c]
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
    BY <1>1, BufferReadingsFrame, Zenon
\* Emitting frees the slot: the network has the bytes, and the callback
\* still on the stack is a separate count.
<1>4. ASSUME NEW c \in CallIds
      PROVE  /\ (SendWindowOccupancy(c))' =
                    (IF c = cId THEN SendWindowOccupancy(c) - 1
                     ELSE SendWindowOccupancy(c))
             /\ (HasNoSendInFlight(c))' =
                    (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
  <2>1. /\ (buffers_held_by_host[c])' = buffers_held_by_host[c]
        /\ Len((submitted[c])') = Len(submitted[c])
        /\ (write_dones_emitted[c])' =
               (IF c = cId THEN write_dones_emitted[c] + 1
                ELSE write_dones_emitted[c])
    BY <1>1, <1>3, <1>35, Zenon
  <2>2. /\ buffers_held_by_host[c] \in Nat
        /\ write_dones_emitted[c] \in Nat
        /\ Len(submitted[c]) \in Nat
    <3>1. submitted \in [CallIds -> Seq(Messages)]
      BY Zenon DEF TypeOK, L0!TypeOK
    <3>2. QED
      BY <1>2, <3>1, LenProperties, SendWindowCountsAreIntegers, Zenon
  <2>3. (SendWindowOccupancy(c))' =
            (IF c = cId THEN SendWindowOccupancy(c) - 1
             ELSE SendWindowOccupancy(c))
    BY <2>1, <2>2, SMT DEF SendWindowOccupancy
  <2>4. QED
    BY <1>1, <1>2, <1>3, <2>3, SMT
<1>50. \A c \in CallIds :
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
    BY <1>1, PayloadReadingsFrame, Zenon
<1>51. \A c \in CallIds :
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' =
                 IsDeliveryCallbackRunning(c)
    BY <1>1, CancelFlagReadingsFrame, HandleFlagReadingsFrame,
       DeliveryFlagReadingsFrame, Zenon
<1>52. ~HasNoSendInFlight(cId)
    BY <1>1, <1>2, SMT
<1>6. QED
    BY <1>1, <1>52, <1>3, <1>35, <1>4, <1>50, <1>51, ZenonT(120)

\* The acquittal callback returns: the returned count catches up with the
\* emitted one, which is what acquits the send.  The slot went back at
\* emission, so the window does not move here.
LEMMA WriteDoneReturnsTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, WriteDoneReturns(cId)
    PROVE  /\ IsWriteDoneCallbackRunning(cId)
           /\ ~HasNoSendInFlight(cId)
           /\ \A c \in CallIds :
                  c # cId => (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
           /\ \A c \in CallIds :
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         (IF c = cId THEN FALSE
                          ELSE IsWriteDoneCallbackRunning(c))
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                  /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                  /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
<1>1. /\ write_done_callback_running' =
             [write_done_callback_running EXCEPT ![cId] = FALSE]
      /\ write_done_callback_running[cId]
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     delivery_callback_running,
                     payloads_consumed_by_host, handle_released,
                     cancel_requested>>
    BY SMT DEF WriteDoneReturns, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>2. /\ write_dones_emitted \in [CallIds -> Nat]
      /\ write_done_callback_running \in [CallIds -> BOOLEAN]
      /\ submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>20. \A c \in CallIds : Len(submitted[c]) \in Nat
    BY <1>2, LenProperties, Zenon
<1>3. \A c \in CallIds :
          /\ (IsWriteDoneCallbackRunning(c))' =
                 (IF c = cId THEN FALSE
                  ELSE IsWriteDoneCallbackRunning(c))
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>1, <1>2, Zenon
<1>35. \A c \in CallIds :
          /\ (buffers_held_by_host[c])' = buffers_held_by_host[c]
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
    BY <1>1, BufferReadingsFrame, Zenon
\* The return acquits the send but the slot went back at emission, so the
\* window does not move here at all.
<1>4. ASSUME NEW c \in CallIds
      PROVE  (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
  <2>1. /\ (buffers_held_by_host[c])' = buffers_held_by_host[c]
        /\ Len((submitted[c])') = Len(submitted[c])
        /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>1, <1>3, <1>35, Zenon
  <2>2. QED
    BY <2>1, SMT DEF SendWindowOccupancy
<1>40. \A c \in CallIds :
          c # cId => (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY <1>1, <1>2, SMT
<1>5. \A c \in CallIds :
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' =
                 IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
    BY <1>1, PayloadReadingsFrame, CancelFlagReadingsFrame,
       HandleFlagReadingsFrame, DeliveryFlagReadingsFrame, Zenon
<1>6. QED
    BY <1>1, <1>3, <1>35, <1>4, <1>40, <1>5, Zenon

\* The notification is honest: WRITE_DONE really hands a slot back, so a
\* host woken by one and asking for a buffer is never refused.  Nothing
\* forced this to be stated - the send side is host-driven, so no fairness
\* lift needed it - and its absence is exactly what let the slot
\* accounting drift from the ABI it documents.
THEOREM WriteDoneFreesASlot ==
    ASSUME NEW cId \in CallIds, StrongInv, EmitWriteDone(cId)
    PROVE  (HasFreeSendSlot(cId))'
<1>1. SendWindowOccupancy(cId) <= MaxSendsInFlight
    BY Zenon DEF StrongInv, FfiCallInv, SendsInFlightWithinLimit
<1>2. (SendWindowOccupancy(cId))' = SendWindowOccupancy(cId) - 1
    BY EmitWriteDoneTransfers, Zenon DEF StrongInv
<1>3. \A c \in CallIds : SendWindowOccupancy(c) \in Int
    BY SendWindowCountsAreIntegers, Zenon DEF StrongInv
<1>4. QED
    BY <1>1, <1>2, <1>3, MaxSendsInFlightIsPositive, SMT
    DEF HasFreeSendSlot

\* The two set facts behind the bridge, stated without state so they are
\* pure set reasoning: writing one entry of one call moves that call's lent
\* set by exactly that element, and leaves every other call's alone.
LEMMA LentSetGainsTheLentOne ==
    ASSUME NEW bs, NEW bs2, NEW cId \in CallIds,
           NEW b \in BufferIds,
           bs \in [CallIds -> [BufferIds -> BufferStates]],
           bs[cId][b] # "lent",
           bs2 = [bs EXCEPT ![cId][b] = "lent"]
    PROVE  /\ {x \in BufferIds : bs2[cId][x] = "lent"} = {x \in BufferIds : bs[cId][x] = "lent"} \cup {b}
           /\ b \notin {x \in BufferIds : bs[cId][x] = "lent"}
           /\ \A c \in CallIds : c # cId =>
                  {x \in BufferIds : bs2[c][x] = "lent"} = {x \in BufferIds : bs[c][x] = "lent"}
<1>1. QED
    BY SMT

LEMMA LentSetLosesTheGivenBack ==
    ASSUME NEW bs, NEW bs2, NEW cId \in CallIds,
           NEW b \in BufferIds, NEW s \in BufferStates,
           bs \in [CallIds -> [BufferIds -> BufferStates]],
           bs[cId][b] = "lent", s # "lent",
           bs2 = [bs EXCEPT ![cId][b] = s]
    PROVE  /\ {x \in BufferIds : bs2[cId][x] = "lent"} = {x \in BufferIds : bs[cId][x] = "lent"} \ {b}
           /\ b \in {x \in BufferIds : bs[cId][x] = "lent"}
           /\ \A c \in CallIds : c # cId =>
                  {x \in BufferIds : bs2[c][x] = "lent"} = {x \in BufferIds : bs[c][x] = "lent"}
<1>1. QED
    BY SMT

\* And the third: writing an entry that was not lent to something that is
\* not lent either leaves every lent set alone.  This is FreeReturnedBuffer.
LEMMA LentSetsUnmovedOffTheLentState ==
    ASSUME NEW bs, NEW bs2, NEW cId \in CallIds,
           NEW b \in BufferIds, NEW s \in BufferStates,
           bs \in [CallIds -> [BufferIds -> BufferStates]],
           bs[cId][b] # "lent", s # "lent",
           bs2 = [bs EXCEPT ![cId][b] = s]
    PROVE  \A c \in CallIds :
               {x \in BufferIds : bs2[c][x] = "lent"} = {x \in BufferIds : bs[c][x] = "lent"}
<1>1. QED
    BY SMT

\* Cardinality zero on a finite set means the set is empty.  That is what
\* turns the bridge into a per-buffer fact, and it is the only place the
\* counted view and the named one have to be reconciled by hand.
LEMMA NoLentBufferWhenCountIsZero ==
    ASSUME TypeOK, LentCountMatchesBufferStates, NEW cId \in CallIds,
           HostHoldsNoBuffer(cId)
    PROVE  \A b \in BufferIds : ~IsLentBuffer(cId, b)
<1>1. {b \in BufferIds : IsLentBuffer(cId, b)} \subseteq BufferIds
    OBVIOUS
<1>2. IsFiniteSet({b \in BufferIds : IsLentBuffer(cId, b)})
    BY <1>1, BufferIdsAreAFiniteNonemptySet, FS_Subset
<1>3. Cardinality({b \in BufferIds : IsLentBuffer(cId, b)}) = 0
    BY Zenon DEF LentCountMatchesBufferStates, HostHoldsNoBuffer
<1>4. SUFFICES ASSUME NEW b0 \in BufferIds, IsLentBuffer(cId, b0)
               PROVE  FALSE
    OBVIOUS
<1>5. b0 \in {b \in BufferIds : IsLentBuffer(cId, b)}
    BY <1>4
<1>6. Cardinality({b \in BufferIds : IsLentBuffer(cId, b)}) >= 1
  <2>1. IsFiniteSet({b \in BufferIds : IsLentBuffer(cId, b)} \ {b0})
    BY <1>2, FS_Subset
  <2>2. {b \in BufferIds : IsLentBuffer(cId, b)} = ({b \in BufferIds : IsLentBuffer(cId, b)} \ {b0}) \cup {b0}
    BY <1>5
  <2>3. Cardinality(({b \in BufferIds : IsLentBuffer(cId, b)} \ {b0}) \cup {b0}) =
            Cardinality({b \in BufferIds : IsLentBuffer(cId, b)} \ {b0}) + 1
    BY <2>1, FS_AddElement
  <2>4. Cardinality({b \in BufferIds : IsLentBuffer(cId, b)} \ {b0}) \in Nat
    BY <2>1, FS_CardinalityType
  <2>5. QED
    BY <2>2, <2>3, <2>4, SMT
<1>7. QED
    BY <1>3, <1>6, SMT

\* Destroying a runtime really does void its call handles: nothing that
\* names a call of a destroyed runtime is ever enabled again.  Requesting
\* a cancellation is the only downcall a finished call could still accept,
\* and a guard says so; the rest follow from what destruction required -
\* a released runtime has no active call, and nothing of its memory is
\* out.  Reclamation is in the list too, and it is the runtime's own step:
\* it does not reclaim a call whose runtime is already gone.
THEOREM DestroyedRuntimeRejectsHandles ==
    ASSUME StrongInv, NEW rtId \in RuntimeIds, IsRuntimeDestroyed(rtId),
           NEW cId \in CallIds,
           call_channel[cId] \in L0!ChannelsOf(rtId)
    PROVE  /\ ~ReleaseCallHandle(cId)
           /\ ~RequestCallCancellation(cId)
           /\ ~EndSend(cId)
           /\ \A msg \in Messages, b \in BufferIds :
                 ~SendMessage(cId, msg, b)
           /\ \A b \in BufferIds, msg \in Messages, ch \in Sizes :
                 ~LendSendBuffer(cId, b, msg, ch)
           /\ \A b \in BufferIds : ~HostReturnsBuffer(cId, b)
<1>0. TypeOK
    BY Zenon DEF StrongInv
<1>1. IsReleasedRuntime(rtId) /\ NoHostDebt(rtId)
    BY Zenon DEF StrongInv, DestroyedRuntimeIsClean, IsRuntimeQuiescent
\* The guard reclamation and the cancel request both read.
<1>2. IsRuntimeOfCallDestroyed(cId)
    BY Zenon DEF IsRuntimeOfCallDestroyed
<1>3. ~ReleaseCallHandle(cId) /\ ~RequestCallCancellation(cId)
    BY <1>2, Zenon DEF ReleaseCallHandle, RequestCallCancellation
\* An active call hangs off an active channel, and an active channel
\* hangs off a runtime that is not released.
<1>4. ~L0!IsActiveCall(cId)
    BY <1>1, SMT
    DEF StrongInv, L0!StrongInv, L0!StructuralInv, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!UsedChannels, L0!UsedCalls,
        L0!ActiveChannels, L0!ActiveChannelStates, L0!ChannelsOf,
        L0!IsActiveCall, L0!ActiveCallStates, L0!IsUnusedCall,
        IsReleasedRuntime, TypeOK, L0!TypeOK, L0!ChannelStates,
        L0!CallStates, L0!RuntimeStates
<1>5. HostHoldsNoBuffer(cId)
    BY <1>1, Zenon DEF NoHostDebt
\* Lending and both level-0 send actions need an active call; giving a
\* buffer back needs one to be lent, and none is - the count is zero and
\* the bridge ties the count to the states.
<1>6. ~EndSend(cId)
    BY <1>4, Zenon DEF EndSend, L0!EndSend, L0!IsActiveCall,
        L0!ActiveCallStates
<1>7. \A b \in BufferIds, msg \in Messages, ch \in Sizes :
                 ~LendSendBuffer(cId, b, msg, ch)
    BY <1>4, Zenon DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, L0!IsActiveCall, L0!ActiveCallStates
<1>8. \A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs)
    BY <1>0, <1>5, SMT
    DEF SendMessage, HostHoldsSomeBuffer, HostHoldsNoBuffer, TypeOK
<1>85. \A b \in BufferIds : ~IsLentBuffer(cId, b)
    BY <1>0, <1>5, NoLentBufferWhenCountIsZero, Zenon
    DEF StrongInv, BufferStateInv
<1>9. \A b \in BufferIds : ~HostReturnsBuffer(cId, b)
    BY <1>85, Zenon DEF HostReturnsBuffer
<1>95. QED
    BY <1>3, <1>6, <1>7, <1>8, <1>9

LEMMA DeliveryReturnTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, DeliveryCallbackReturns(cId)
    PROVE  \A c \in CallIds :
               /\ (IsDeliveryCallbackRunning(c))' =
                      (IF c = cId THEN FALSE
                       ELSE IsDeliveryCallbackRunning(c))
               /\ (IsCancelRequested(c))' = IsCancelRequested(c)
               /\ (IsHandleReleased(c))' = IsHandleReleased(c)
               /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
               /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
               /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                      HostOwnsAtMostCreditsPlusOne(c)
               /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
               /\ (IsWriteDoneCallbackRunning(c))' =
                      IsWriteDoneCallbackRunning(c)
               /\ (write_dones_emitted[c])' = write_dones_emitted[c]
               /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
               /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
               /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
               /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
<1>1. /\ delivery_callback_running' =
             [delivery_callback_running EXCEPT ![cId] = FALSE]
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
      /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     write_done_callback_running,
                     payloads_consumed_by_host, handle_released,
                     cancel_requested>>
    BY SMT DEF DeliveryCallbackReturns, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>15. delivery_callback_running \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK
<1>2. \A c \in CallIds :
          (IsDeliveryCallbackRunning(c))' =
              (IF c = cId THEN FALSE ELSE IsDeliveryCallbackRunning(c))
    BY <1>1, <1>15, Zenon
<1>3. \A c \in CallIds :
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY <1>1, PayloadReadingsFrame, SendReadingsFrame, BufferReadingsFrame,
       SendWindowFrame,
       CancelFlagReadingsFrame, HandleFlagReadingsFrame, Zenon
<1>4. QED
    BY <1>2, <1>3, Zenon

\* Consuming shrinks the owned set by exactly its payload; the credit
\* readings can only improve, which is what the credit invariants need.
LEMMA ConsumeTransfers ==
    ASSUME NEW cId \in CallIds, TypeOK, HostConsumesEvent(cId)
    PROVE  /\ HostOwnsSomePayload(cId)
           /\ \A c \in CallIds :
                  /\ (payloads_consumed_by_host[c])' =
                         IF c = cId THEN payloads_consumed_by_host[c] + 1
                         ELSE payloads_consumed_by_host[c]
                  /\ (events_delivered[c])' = events_delivered[c]
                  /\ (OwedPayloads(c))' =
                         IF c = cId THEN OwedPayloads(c) - 1
                         ELSE OwedPayloads(c)
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
                  /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                  /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                  /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
<1>1. /\ payloads_consumed_by_host' =
             [payloads_consumed_by_host EXCEPT ![cId] = @ + 1]
      /\ HostOwnsSomePayload(cId)
      /\ UNCHANGED <<call_state, call_channel, channel_state,
                     channel_runtime, submitted, sent, received,
                     delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED events_delivered
      /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     write_done_callback_running,
                     delivery_callback_running, handle_released,
                     cancel_requested>>
    BY SMT DEF HostConsumesEvent, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars
<1>15. payloads_consumed_by_host \in [CallIds -> Nat]
    BY Zenon DEF TypeOK
<1>2. \A c \in CallIds :
          /\ (payloads_consumed_by_host[c])' =
                 (IF c = cId THEN payloads_consumed_by_host[c] + 1
                  ELSE payloads_consumed_by_host[c])
          /\ (events_delivered[c])' = events_delivered[c]
    BY <1>1, <1>15, Zenon
<1>16. /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
      /\ \A c \in CallIds : Len(events_delivered[c]) \in Nat
    <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
      BY Zenon DEF TypeOK, L0!TypeOK
    <2>2. QED BY <2>1, LenProperties, Zenon
<1>20. \A c \in CallIds :
           (OwedPayloads(c))' =
               IF c = cId THEN OwedPayloads(c) - 1 ELSE OwedPayloads(c)
    BY <1>2, <1>15, <1>16, SMT
<1>3. \A c \in CallIds :
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' =
                 IsDeliveryCallbackRunning(c)
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY <1>1, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, CancelFlagReadingsFrame,
       HandleFlagReadingsFrame, DeliveryFlagReadingsFrame, Zenon
<1>4. QED
    BY <1>1, <1>2, <1>20, <1>3, Zenon

LEMMA NoEventsMeansNoStatus ==
    ASSUME NEW cId \in CallIds, HasNoDeliveredEvents(cId)
    PROVE  ~L0!HasStatus(cId)
<1>1. QED
    BY SMT DEF L0!HasStatus
LEMMA CallStartTransfers ==
    ASSUME NEW cId \in CallIds, NEW chId \in ChannelIds,
           TypeOK, CallStart(cId, chId)
    PROVE  /\ L0!IsUnusedCall(cId)
           /\ ~IsClosingChannel(chId)
           /\ (call_channel[cId])' = chId
           /\ \A c \in CallIds :
                  /\ (L0!IsActiveCall(c))' =
                         (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
                  /\ (L0!IsTerminalCall(c))' =
                         (IF c = cId THEN FALSE ELSE L0!IsTerminalCall(c))
                  /\ (L0!IsUnusedCall(c))' =
                         (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
                  /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                  /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
                  /\ (events_delivered[c])' = events_delivered[c]
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
                  /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                  /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                  /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ Len((submitted[c])') = Len(submitted[c])
                  /\ (call_channel[c])' =
                         IF c = cId THEN chId ELSE call_channel[c]
           /\ \A chan \in ChannelIds :
                  (IsClosingChannel(chan))' = IsClosingChannel(chan)
<1>1. /\ call_state' = [call_state EXCEPT ![cId] = "started"]
      /\ call_channel' = [call_channel EXCEPT ![cId] = chId]
      /\ L0!IsUnusedCall(cId)
      /\ channel_state[chId] = "open"
      /\ UNCHANGED <<channel_state, channel_runtime, submitted, sent,
                     received, delivered, events_delivered, send_closed,
                     status_pending, runtime_state>>
      /\ UNCHANGED ffi_vars
    BY SMT DEF CallStart, L0!CallStart,
        L0!RuntimeVars, L0!ChannelVars, ffi_vars
<1>2. /\ call_state \in [CallIds -> L0!CallStates]
      /\ call_channel \in [CallIds -> ChannelIds \union {"none"}]
      /\ channel_state \in [ChannelIds -> L0!ChannelStates]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>3. ~IsClosingChannel(chId)
    BY <1>1, <1>2, Zenon DEF L0!ChannelStates
<1>4. ASSUME NEW c \in CallIds
      PROVE  /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
             /\ (events_delivered[c])' = events_delivered[c]
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ Len((submitted[c])') = Len(submitted[c])
  <2>1. UNCHANGED buffers_held_by_host
    BY <1>1, SMT DEF ffi_vars
  <2>2. QED
    BY <1>1, <2>1, BufferReadingsFrame, SMT DEF ffi_vars
<1>5. ASSUME NEW c \in CallIds
      PROVE  /\ (L0!IsActiveCall(c))' =
                    (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
             /\ (L0!IsTerminalCall(c))' =
                    (IF c = cId THEN FALSE ELSE L0!IsTerminalCall(c))
             /\ (L0!IsUnusedCall(c))' =
                    (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (call_channel[c])' =
                    IF c = cId THEN chId ELSE call_channel[c]
    BY <1>1, <1>2, SMT DEF L0!IsActiveCall, L0!ActiveCallStates,
        L0!IsTerminalCall, L0!IsUnusedCall, L0!HasStatus,
        L0!EventKinds, L0!StatusKinds
<1>6. ASSUME NEW chan \in ChannelIds
      PROVE  (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, SMT
\* No backend hint: the goal is a wide conjunction of facts already proved,
\* and pinning one backend on it is what put the step on the edge of its
\* budget - Zenon closes it, but only just.
<1>7. QED
    BY <1>1, <1>3, <1>4, <1>5, <1>6

\* The predicate-level effect of an accepted send: the actor gains one
\* pinned buffer and stays active, everything else reads unchanged.  The
\* preservation case consumes these facts with the predicates hidden, so
\* no obligation ever carries both the action and the invariant bodies.
LEMMA SendMessageTransfers ==
    ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW bs \in BufferIds,
           TypeOK, FfiCallInv, SendMessage(cId, msg, bs)
    PROVE  /\ L0!IsActiveCall(cId)
           /\ ~IsCancelRequested(cId)
           /\ ~IsHandleReleased(cId)
           /\ HostHoldsSomeBuffer(cId)
           /\ \A c \in CallIds :
                  /\ (L0!IsActiveCall(c))' =
                         (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
                  /\ (L0!IsTerminalCall(c))' =
                         (IF c = cId THEN FALSE ELSE L0!IsTerminalCall(c))
                  /\ (L0!IsUnusedCall(c))' =
                         (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
                  /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                  /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                  /\ (IsDeliveryCallbackRunning(c))' =
                         IsDeliveryCallbackRunning(c)
                  /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                  /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                  /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                         HostOwnsAtMostCreditsPlusOne(c)
                  /\ (payloads_consumed_by_host[c])' =
                         payloads_consumed_by_host[c]
                  /\ (events_delivered[c])' = events_delivered[c]
                  /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                  /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                  /\ (IsWriteDoneCallbackRunning(c))' =
                         IsWriteDoneCallbackRunning(c)
                  /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                  /\ Len((submitted[c])') =
                         IF c = cId THEN Len(submitted[c]) + 1
                         ELSE Len(submitted[c])
                  /\ (HasNoSendInFlight(c))' =
                         (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
                  /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                  /\ (c # cId =>
                         /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                         /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
                  /\ (call_channel[c])' = call_channel[c]
           /\ \A chId \in ChannelIds :
                  (IsClosingChannel(chId))' = IsClosingChannel(chId)
<1>1. /\ submitted' = [submitted EXCEPT ![cId] = Append(@, msg)]
      /\ call_state' = [call_state EXCEPT ![cId] = "sending"]
      /\ call_state[cId] \in {"started", "sending"}
      /\ ~IsCancelRequested(cId)
      /\ HostHoldsSomeBuffer(cId)
      /\ buffers_held_by_host' =
             [buffers_held_by_host EXCEPT ![cId] = @ - 1]
      /\ UNCHANGED <<call_channel, channel_state, channel_runtime,
                     events_delivered, sent, received, delivered,
                     send_closed, status_pending, runtime_state>>
      /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                     delivery_callback_running, payloads_consumed_by_host,
                     handle_released, cancel_requested,
                     shutdown_event_emitted, shutdown_callback_running>>
    BY SMT DEF SendMessage, L0!SendMessage,
        L0!RuntimeVars, L0!ChannelVars
<1>2. /\ submitted \in [CallIds -> Seq(Messages)]
      /\ call_state \in [CallIds -> L0!CallStates]
      /\ buffers_held_by_host \in [CallIds -> Nat]
      /\ write_dones_emitted \in [CallIds -> Nat]
      /\ write_done_callback_running \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>3. L0!IsActiveCall(cId)
    BY <1>1, Zenon DEF L0!IsActiveCall, L0!ActiveCallStates
\* A released call is terminal, so an accepted send proves the handle is
\* still live - which is what makes ReleasedCallIsClean vacuous here.
<1>30. ~IsHandleReleased(cId)
    BY <1>3, Zenon DEF FfiCallInv, ReleasedCallIsClean
<1>4. ASSUME NEW c \in CallIds
      PROVE  /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
             /\ (events_delivered[c])' = events_delivered[c]
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (call_channel[c])' = call_channel[c]
             /\ (c # cId =>
                    /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                    /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
    BY <1>1, <1>2, SMTT(120)
    DEF HostHoldsNoBuffer, HostHoldsSomeBuffer
<1>5. ASSUME NEW c \in CallIds
      PROVE  Len((submitted[c])') =
                 IF c = cId THEN Len(submitted[c]) + 1
                 ELSE Len(submitted[c])
  <2>1. CASE c = cId
    <3>1. (submitted[c])' = Append(submitted[cId], msg)
      BY <1>1, <1>2, <2>1, Zenon
    <3>20. submitted[cId] \in Seq(Messages)
      BY <1>2, Zenon
    <3>2. Len(Append(submitted[cId], msg)) = Len(submitted[cId]) + 1
      BY <3>20, AppendProperties
    <3>3. QED BY <2>1, <3>1, <3>2, Zenon
  <2>2. CASE c # cId
    BY <1>1, <1>2, <2>2, Zenon
  <2>3. QED BY <2>1, <2>2, Zenon
\* The count of outstanding buffers does not move: the commit takes one
\* buffer out of the host's hands and puts the send that keeps it into
\* submitted.  The call stops being drained, and that second reading
\* needs the invariant: a call whose acquittals could outrun its sends
\* would look drained right after accepting one.
<1>6. ASSUME NEW c \in CallIds
      PROVE  /\ (HasNoSendInFlight(c))' =
                    (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
  <2>0. /\ write_dones_emitted[c] \in Nat
        /\ Len(submitted[c]) \in Nat
        /\ buffers_held_by_host[c] \in Nat
        /\ write_dones_emitted[c] <= Len(submitted[c])
    <3>1. submitted \in [CallIds -> Seq(Messages)]
      BY <1>2, Zenon
    <3>2. Len(submitted[c]) \in Nat
      BY <3>1, LenProperties, Zenon
    <3>3. QED
      BY <1>2, <3>2, Zenon DEF TypeOK, FfiCallInv, WriteDonesNeverExceedSends
  <2>05. (buffers_held_by_host[c])' =
             IF c = cId THEN buffers_held_by_host[c] - 1
             ELSE buffers_held_by_host[c]
    BY <1>1, <1>2, Zenon
  <2>1. (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
    BY <1>4, <1>5, <2>0, <2>05, SMT
  <2>2. QED
    BY <1>4, <1>5, <2>0, <2>1, SMT
<1>7. ASSUME NEW c \in CallIds
      PROVE  /\ (L0!IsActiveCall(c))' =
                    (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
             /\ (L0!IsTerminalCall(c))' =
                    (IF c = cId THEN FALSE ELSE L0!IsTerminalCall(c))
             /\ (L0!IsUnusedCall(c))' =
                    (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
    BY <1>1, <1>2, SMTT(120) DEF L0!IsActiveCall, L0!ActiveCallStates,
        L0!IsTerminalCall, L0!IsUnusedCall, L0!HasStatus,
        L0!EventKinds, L0!StatusKinds
<1>8. ASSUME NEW chId \in ChannelIds
      PROVE  (IsClosingChannel(chId))' = IsClosingChannel(chId)
    BY <1>1, SMT
<1>9. QED
    BY <1>1, <1>3, <1>30, <1>4, <1>5, <1>6, <1>7, <1>8, Zenon
\* Proved in three groups rather than one: the conjunct list is long
\* enough that a single goal outgrows the solver's budget.
LEMMA FfiFramePreservesFfiCallInv ==
    /\ FfiCallInv
    /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                   write_done_callback_running,
                   delivery_callback_running, payloads_consumed_by_host,
                   handle_released, cancel_requested>>
    /\ UNCHANGED <<call_state, call_channel, channel_state,
                   events_delivered, submitted>>
    => FfiCallInv'
<1>1. ASSUME FfiCallInv,
             UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                         write_done_callback_running,
                         delivery_callback_running,
                         payloads_consumed_by_host,
                         handle_released, cancel_requested>>,
             UNCHANGED <<call_state, call_channel, channel_state,
                         events_delivered, submitted>>
      PROVE  FfiCallInv'
  <2>0. /\ UNCHANGED buffers_held_by_host
        /\ UNCHANGED <<write_dones_emitted, write_done_callback_running,
                       submitted>>
        /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running, submitted>>
        /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
        /\ UNCHANGED delivery_callback_running
        /\ UNCHANGED handle_released
        /\ UNCHANGED cancel_requested
    BY <1>1, SMT
  <2>1. \A c \in CallIds :
            /\ (write_dones_emitted[c])' = write_dones_emitted[c]
            /\ (IsWriteDoneCallbackRunning(c))' =
                   IsWriteDoneCallbackRunning(c)
            /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
            /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
            /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
            /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
            /\ (IsCancelRequested(c))' = IsCancelRequested(c)
            /\ (IsHandleReleased(c))' = IsHandleReleased(c)
            /\ (IsDeliveryCallbackRunning(c))' = IsDeliveryCallbackRunning(c)
            /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
            /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
            /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
            /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                   HostOwnsAtMostCreditsPlusOne(c)
    BY <2>0, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame,
       PayloadReadingsFrame, CancelFlagReadingsFrame,
       HandleFlagReadingsFrame, DeliveryFlagReadingsFrame, Zenon
  <2>2. /\ \A c \in CallIds :
               /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
               /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
               /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
               /\ (L0!HasStatus(c))' = L0!HasStatus(c)
               /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
               /\ (events_delivered[c])' = events_delivered[c]
               /\ Len((submitted[c])') = Len(submitted[c])
               /\ (call_channel[c])' = call_channel[c]
        /\ \A chan \in ChannelIds :
               (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, SMT DEF L0!IsActiveCall, L0!IsTerminalCall, L0!IsUnusedCall,
        L0!HasStatus
  <2>3. (/\ UnusedCallsAreFfiClean
         /\ ReleasedCallIsClean
         /\ ClosingChannelCallsCancelRequested)'
    BY <1>1, <2>1, <2>2, SMT DEF FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, ClosingChannelCallsCancelRequested
  <2>4. (/\ SendsInFlightWithinLimit
         /\ WriteDonesNeverExceedSends
         /\ RunningWriteDoneWasEmitted
         /\ TerminalCallHasNoSendInFlight)'
    BY <1>1, <2>1, <2>2, SMT DEF FfiCallInv, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
  <2>5. (/\ ReleasesNeverExceedDeliveries
         /\ ActiveCallPayloadsWithinCredits
         /\ PayloadsOwnedWithinCreditsPlusOne
         /\ NoDeliveryImpliesNoDebt
         /\ ActiveCallHasNoStatus
         /\ UnusedCallHasNoEvents)'
    BY <1>1, <2>1, <2>2, SMT DEF FfiCallInv,
        ReleasesNeverExceedDeliveries, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
  <2>6. QED
    BY <2>3, <2>4, <2>5, Zenon DEF FfiCallInv
<1>2. QED
    BY <1>1

LEMMA RuntimeOnlyPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeRuntimeOnly => FfiCallInv'
<1>1. QED
    BY FfiFramePreservesFfiCallInv, SMT
    DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!ChannelVars, L0!CallVars,
        ffi_vars

\* The two closing paths - shutdown closes a runtime's channels, a close
\* closes one - differ only in which channels move.  Both latch
\* cancellation on the active calls of those channels and leave every
\* other FFI variable alone, so they share this argument: cancellation
\* only ever goes on, and a call that finds its channel newly closing is
\* one the same step just latched.
LEMMA CancelLatchPreservesFfiCallInv ==
    ASSUME FfiCallInv, TypeOK, NEW chs,
           RequestCancellationOfActiveCalls(chs),
           UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running,
                       delivery_callback_running,
                       payloads_consumed_by_host, handle_released>>,
           UNCHANGED <<call_state, call_channel, events_delivered,
                       submitted>>,
           \A chan \in ChannelIds :
               IsClosingChannel(chan)' =>
                   (IsClosingChannel(chan) \/ chan \in chs)
    PROVE  FfiCallInv'
<1>1. \A c \in CallIds :
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (IsDeliveryCallbackRunning(c))' = IsDeliveryCallbackRunning(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
    BY SendReadingsFrame, BufferReadingsFrame, SendWindowFrame,
       PayloadReadingsFrame, HandleFlagReadingsFrame,
       DeliveryFlagReadingsFrame, Zenon
<1>2. \A c \in CallIds :
          /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
          /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
          /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
          /\ (L0!HasStatus(c))' = L0!HasStatus(c)
          /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
          /\ (events_delivered[c])' = events_delivered[c]
          /\ Len((submitted[c])') = Len(submitted[c])
          /\ (call_channel[c])' = call_channel[c]
    BY SMT DEF L0!IsActiveCall, L0!IsTerminalCall, L0!IsUnusedCall,
        L0!HasStatus
\* Cancellation is latched exactly on the active calls of chs, so it can
\* only turn on, and it is on for every call the closing reaches.
<1>3. \A c \in CallIds :
          /\ (IsCancelRequested(c) => (IsCancelRequested(c))')
          /\ (call_channel[c] \in chs /\ L0!IsActiveCall(c) =>
                  (IsCancelRequested(c))')
    <2>1. cancel_requested \in [CallIds -> BOOLEAN]
      BY Zenon DEF TypeOK
    <2>2. QED
      BY <2>1, SMT DEF RequestCancellationOfActiveCalls, IsCancelRequested
<1>4. \A c \in CallIds : L0!IsUnusedCall(c) => ~L0!IsActiveCall(c)
    BY SMT DEF L0!IsUnusedCall, L0!IsActiveCall, L0!ActiveCallStates
<1>5. \A c \in CallIds :
          L0!IsUnusedCall(c) => ~(IsCancelRequested(c))'
    <2>1. cancel_requested \in [CallIds -> BOOLEAN]
      BY Zenon DEF TypeOK
    <2>2. QED
      BY <1>4, <2>1, SMT
      DEF FfiCallInv, UnusedCallsAreFfiClean,
          RequestCancellationOfActiveCalls, IsCancelRequested
<1>6. (/\ UnusedCallsAreFfiClean
       /\ ReleasedCallIsClean)'
    BY <1>1, <1>2, <1>5, SMT
    DEF FfiCallInv, UnusedCallsAreFfiClean, ReleasedCallIsClean
<1>7. ClosingChannelCallsCancelRequested'
    BY <1>1, <1>2, <1>3, SMT
    DEF FfiCallInv, ClosingChannelCallsCancelRequested
<1>8. (/\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>1, <1>2, SMT DEF FfiCallInv, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>9. (/\ ReleasesNeverExceedDeliveries
       /\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>1, <1>2, SMT DEF FfiCallInv,
        ReleasesNeverExceedDeliveries, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>10. QED
    BY <1>6, <1>7, <1>8, <1>9, Zenon DEF FfiCallInv

LEMMA RuntimeChannelPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeRuntimeChannel => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, NEW rtId \in RuntimeIds,
             RuntimeBeginShutdown(rtId)
      PROVE  FfiCallInv'
  <2>1. /\ RequestCancellationOfActiveCalls(L0!ChannelsOf(rtId))
        /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running,
                       delivery_callback_running,
                       payloads_consumed_by_host, handle_released>>
        /\ UNCHANGED <<call_state, call_channel, events_delivered,
                       submitted>>
    BY <1>1, SMT DEF RuntimeBeginShutdown, L0!RuntimeBeginShutdown,
        L0!CallVars
\* A channel only ever becomes closing when it belongs to the runtime
\* that is stopping, which is exactly the set the same step latches.
  <2>2. \A chan \in ChannelIds :
            IsClosingChannel(chan)' =>
                (IsClosingChannel(chan) \/ chan \in L0!ChannelsOf(rtId))
    BY <1>1, SMT DEF RuntimeBeginShutdown, L0!RuntimeBeginShutdown,
        L0!ChannelsOf, IsClosingChannel
  <2>3. QED
    BY <1>1, <2>1, <2>2, CancelLatchPreservesFfiCallInv
<1>2. QED
    BY <1>1 DEF NextSafeRuntimeChannel

LEMMA ChannelOnlyPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeChannelOnly => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, NextSafeChannelOnly
      PROVE  FfiCallInv'
\* Creating a channel latches nothing, which is the empty case of the
\* same argument: no channel becomes closing, so no call needs latching.
  <2>1. ASSUME NEW chId \in ChannelIds, NEW rtId \in RuntimeIds,
               ChannelCreate(chId, rtId)
        PROVE  FfiCallInv'
    <3>0. channel_state \in [ChannelIds -> L0!ChannelStates]
      BY <1>1, Zenon DEF TypeOK, L0!TypeOK
    <3>1. /\ channel_state' = [channel_state EXCEPT ![chId] = "open"]
          /\ UNCHANGED cancel_requested
          /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                         write_done_callback_running,
                         delivery_callback_running,
                         payloads_consumed_by_host, handle_released>>
          /\ UNCHANGED <<call_state, call_channel, events_delivered,
                         submitted>>
      BY <2>1, SMT DEF ChannelCreate, L0!ChannelCreate, L0!RuntimeVars,
          L0!CallVars, ffi_vars
    <3>2. \A chan \in ChannelIds :
              IsClosingChannel(chan)' =>
                  (IsClosingChannel(chan) \/ chan \in {})
      BY <3>0, <3>1, SMT DEF IsClosingChannel
    <3>3. RequestCancellationOfActiveCalls({})
      <4>1. cancel_requested \in [CallIds -> BOOLEAN]
        BY <1>1, Zenon DEF TypeOK
      <4>2. QED
        BY <3>1, <4>1, SMT
        DEF RequestCancellationOfActiveCalls, IsCancelRequested
    <3>4. QED
      BY <1>1, <3>1, <3>2, <3>3, CancelLatchPreservesFfiCallInv
  <2>2. ASSUME NEW chId \in ChannelIds, ChannelStartClosing(chId)
        PROVE  FfiCallInv'
    <3>0. channel_state \in [ChannelIds -> L0!ChannelStates]
      BY <1>1, Zenon DEF TypeOK, L0!TypeOK
    <3>1. /\ RequestCancellationOfActiveCalls({chId})
          /\ channel_state' = [channel_state EXCEPT ![chId] = "closing"]
          /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                         write_done_callback_running,
                         delivery_callback_running,
                         payloads_consumed_by_host, handle_released>>
          /\ UNCHANGED <<call_state, call_channel, events_delivered,
                         submitted>>
      BY <2>2, SMT DEF ChannelStartClosing, L0!ChannelStartClosing,
          L0!CallVars
    <3>2. \A chan \in ChannelIds :
              IsClosingChannel(chan)' =>
                  (IsClosingChannel(chan) \/ chan \in {chId})
      BY <3>0, <3>1, SMTT(120) DEF IsClosingChannel
    <3>3. QED
      BY <1>1, <3>1, <3>2, CancelLatchPreservesFfiCallInv
  <2>3. QED BY <1>1, <2>1, <2>2 DEF NextSafeChannelOnly, ChannelCreate
<1>2. QED BY <1>1

\* The close completes only once no call of the channel is active, so
\* every level-0 comprehension is the identity and nothing moves.
LEMMA ChannelCallPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeChannelCall => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, NEW chId \in ChannelIds,
             ChannelFinishClosing(chId)
      PROVE  FfiCallInv'
  <2>0. /\ channel_state \in [ChannelIds -> L0!ChannelStates]
        /\ call_state \in [CallIds -> L0!CallStates]
        /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
        /\ cancel_requested \in [CallIds -> BOOLEAN]
    BY <1>1, Zenon DEF TypeOK, L0!TypeOK
  <2>05. \A cId \in L0!CallsOf(chId) : ~L0!IsActiveCall(cId)
    BY <1>1, Zenon DEF ChannelFinishClosing
\* No call of the channel is active, so both comprehensions rewrite every
\* entry to itself and the close moves nothing but the channel.
  <2>1. /\ UNCHANGED <<call_state, call_channel, events_delivered,
                       submitted>>
        /\ UNCHANGED cancel_requested
        /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running,
                       delivery_callback_running,
                       payloads_consumed_by_host, handle_released>>
        /\ channel_state' = [channel_state EXCEPT ![chId] = "closed"]
    BY <1>1, <2>0, <2>05, SMT
    DEF ChannelFinishClosing, L0!ChannelFinishClosing, L0!CallsOf,
        L0!RuntimeVars, ffi_vars
  <2>2. \A chan \in ChannelIds :
            IsClosingChannel(chan)' =>
                (IsClosingChannel(chan) \/ chan \in {})
    BY <2>0, <2>1, SMT DEF IsClosingChannel
  <2>3. RequestCancellationOfActiveCalls({})
    BY <2>0, <2>1, SMT
    DEF RequestCancellationOfActiveCalls, IsCancelRequested
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, CancelLatchPreservesFfiCallInv
<1>2. QED
    BY <1>1 DEF NextSafeChannelCall

\* Every step that needs an active call leaves the released ones alone,
\* and that is all ReleasedCallIsClean asks: the call it touches has a
\* live handle, since a released call is over, and the others do not
\* move.  The four delivery cases and the send path all read it this way
\* rather than expanding the invariant against the action.
LEMMA ActiveCallStepKeepsReleasedClean ==
    ASSUME NEW cId \in CallIds, FfiCallInv, TypeOK,
           L0!IsActiveCall(cId),
           UNCHANGED <<handle_released, buffers_held_by_host>>,
           \A c \in CallIds : c # cId =>
               /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
               /\ (IsDeliveryCallbackRunning(c))' =
                      IsDeliveryCallbackRunning(c)
               /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
    PROVE  ReleasedCallIsClean'
<1>1. \A c \in CallIds :
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
    BY HandleFlagReadingsFrame, BufferReadingsFrame, Zenon
<1>2. ~IsHandleReleased(cId)
    BY Zenon DEF FfiCallInv, ReleasedCallIsClean
<1>3. ASSUME NEW c \in CallIds, (IsHandleReleased(c))'
      PROVE  /\ ~(L0!IsActiveCall(c))'
             /\ (HostOwnsNoPayload(c))'
             /\ (HostHoldsNoBuffer(c))'
  <2>1. c # cId
    BY <1>1, <1>2, <1>3, Zenon
  <2>2. IsHandleReleased(c)
    BY <1>1, <1>3, Zenon
  <2>3. QED
    BY <1>1, <2>1, <2>2, Zenon DEF FfiCallInv, ReleasedCallIsClean
<1>4. QED
    BY <1>3, Zenon DEF ReleasedCallIsClean

LEMMA CallOnlyPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeCallOnly => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, NextSafeCallOnly
      PROVE  FfiCallInv'
  <2>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
    <3> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
             IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
             WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
             IsDeliveryCallbackRunning, IsCancelRequested,
             IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
    <3>1. ASSUME NEW cId \in CallIds, NEW chId \in ChannelIds,
                 CallStart(cId, chId)
          PROVE  FfiCallInv'
      <4>0. /\ L0!IsUnusedCall(cId)
            /\ ~IsClosingChannel(chId)
            /\ (call_channel[cId])' = chId
            /\ \A c \in CallIds :
                   /\ (L0!IsActiveCall(c))' =
                          (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
                   /\ (L0!IsTerminalCall(c))' =
                          (IF c = cId THEN FALSE
                           ELSE L0!IsTerminalCall(c))
                   /\ (L0!IsUnusedCall(c))' =
                          (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
                   /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                   /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                   /\ (IsDeliveryCallbackRunning(c))' =
                          IsDeliveryCallbackRunning(c)
                   /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                   /\ (HostOwnsAtMostCredits(c))' =
                          HostOwnsAtMostCredits(c)
                   /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                          HostOwnsAtMostCreditsPlusOne(c)
                   /\ (payloads_consumed_by_host[c])' =
                          payloads_consumed_by_host[c]
                   /\ (events_delivered[c])' = events_delivered[c]
                   /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                   /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                   /\ (IsWriteDoneCallbackRunning(c))' =
                          IsWriteDoneCallbackRunning(c)
                   /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
                   /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                   /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                   /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                   /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                   /\ Len((submitted[c])') = Len(submitted[c])
                   /\ (call_channel[c])' =
                          IF c = cId THEN chId ELSE call_channel[c]
            /\ \A chan \in ChannelIds :
                   (IsClosingChannel(chan))' = IsClosingChannel(chan)
        BY <1>1, <3>1, CallStartTransfers, Zenon
      <4>10. ~L0!HasStatus(cId)
        BY <1>1, <4>0, NoEventsMeansNoStatus, Zenon
        DEF FfiCallInv, UnusedCallHasNoEvents
\* The started call had no handle, so ReleasedCallIsClean stays vacuous
\* for it even though it has just become active.
      <4>11. ~IsHandleReleased(cId)
        BY <1>1, <4>0, Zenon DEF FfiCallInv, UnusedCallsAreFfiClean
      <4>1. (/\ ReleasedCallIsClean
             /\ ClosingChannelCallsCancelRequested
             /\ NoDeliveryImpliesNoDebt
             /\ ActiveCallHasNoStatus
             /\ UnusedCallHasNoEvents)'
        BY <1>1, <4>0, <4>10, <4>11, Zenon
        DEF FfiCallInv, ReleasedCallIsClean,
            ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
            ActiveCallHasNoStatus, UnusedCallHasNoEvents
      <4>20. HostOwnsAtMostCredits(cId) /\ HostOwnsAtMostCreditsPlusOne(cId)
        BY <1>1, <4>0, DeliveryCreditsArePositive, SMT
        DEF FfiCallInv, UnusedCallsAreFfiClean
      <4>2. (/\ ActiveCallPayloadsWithinCredits
             /\ PayloadsOwnedWithinCreditsPlusOne
             /\ ReleasesNeverExceedDeliveries)'
        BY <1>1, <4>0, <4>20, Zenon
        DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
            PayloadsOwnedWithinCreditsPlusOne,
            ReleasesNeverExceedDeliveries
      <4>30. UnusedCallsAreFfiClean'
        BY <1>1, <4>0, Zenon DEF FfiCallInv, UnusedCallsAreFfiClean
      <4>31. (/\ SendsInFlightWithinLimit
              /\ WriteDonesNeverExceedSends
              /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>0, Zenon
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>32. TerminalCallHasNoSendInFlight'
        BY <1>1, <4>0, Zenon
        DEF FfiCallInv, TerminalCallHasNoSendInFlight
      <4>4. QED
        BY <4>1, <4>2, <4>30, <4>31, <4>32, Zenon DEF FfiCallInv
    <3>2. QED BY <2>1, <3>1
  <2>2. CASE \E cId \in CallIds, msg \in Messages,
                bs \in BufferIds : SendMessage(cId, msg, bs)
    <3> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
             IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
             WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
             IsDeliveryCallbackRunning, IsCancelRequested,
             IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
    <3>1. ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW bs \in BufferIds,
                 SendMessage(cId, msg, bs)
          PROVE  FfiCallInv'
      <4>0. /\ L0!IsActiveCall(cId)
            /\ ~IsCancelRequested(cId)
            /\ ~IsHandleReleased(cId)
            /\ HostHoldsSomeBuffer(cId)
            /\ \A c \in CallIds :
                   /\ (L0!IsActiveCall(c))' =
                          (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
                   /\ (L0!IsTerminalCall(c))' =
                          (IF c = cId THEN FALSE
                           ELSE L0!IsTerminalCall(c))
                   /\ (L0!IsUnusedCall(c))' =
                          (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
                   /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                   /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                   /\ (IsDeliveryCallbackRunning(c))' =
                          IsDeliveryCallbackRunning(c)
                   /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                   /\ (HostOwnsAtMostCredits(c))' =
                          HostOwnsAtMostCredits(c)
                   /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                          HostOwnsAtMostCreditsPlusOne(c)
                   /\ (payloads_consumed_by_host[c])' =
                          payloads_consumed_by_host[c]
                   /\ (events_delivered[c])' = events_delivered[c]
                   /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                   /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                   /\ (IsWriteDoneCallbackRunning(c))' =
                          IsWriteDoneCallbackRunning(c)
                   /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                   /\ Len((submitted[c])') =
                          IF c = cId THEN Len(submitted[c]) + 1
                          ELSE Len(submitted[c])
                   /\ (HasNoSendInFlight(c))' =
                          (IF c = cId THEN FALSE
                           ELSE HasNoSendInFlight(c))
                   /\ (SendWindowOccupancy(c))' =
                          SendWindowOccupancy(c)
                   /\ (c # cId =>
                          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                          /\ (HostHoldsSomeBuffer(c))' =
                                 HostHoldsSomeBuffer(c))
                   /\ (call_channel[c])' = call_channel[c]
            /\ \A chId \in ChannelIds :
                   (IsClosingChannel(chId))' = IsClosingChannel(chId)
        BY <1>1, <3>1, SendMessageTransfers, Zenon
      <4>1. (/\ ReleasedCallIsClean
             /\ ClosingChannelCallsCancelRequested
             /\ NoDeliveryImpliesNoDebt
             /\ ActiveCallHasNoStatus
             /\ UnusedCallHasNoEvents)'
        BY <1>1, <4>0, Zenon
        DEF FfiCallInv, ReleasedCallIsClean,
            ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
            ActiveCallHasNoStatus, UnusedCallHasNoEvents
      <4>2. (/\ ActiveCallPayloadsWithinCredits
             /\ PayloadsOwnedWithinCreditsPlusOne
             /\ ReleasesNeverExceedDeliveries)'
        BY <1>1, <4>0, Zenon
        DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
            PayloadsOwnedWithinCreditsPlusOne,
            ReleasesNeverExceedDeliveries
      <4>30. /\ UnusedCallsAreFfiClean
             /\ SendsInFlightWithinLimit
             /\ WriteDonesNeverExceedSends
             /\ RunningWriteDoneWasEmitted
             /\ TerminalCallHasNoSendInFlight
        BY <1>1, Zenon DEF FfiCallInv
      <4>31. \A c \in CallIds : SendWindowOccupancy(c) \in Int
        BY <1>1, SendWindowCountsAreIntegers, Zenon
      <4>32. (/\ UnusedCallsAreFfiClean
              /\ RunningWriteDoneWasEmitted
              /\ TerminalCallHasNoSendInFlight)'
        BY <4>0, <4>30, Zenon
        DEF UnusedCallsAreFfiClean, RunningWriteDoneWasEmitted,
            TerminalCallHasNoSendInFlight
      <4>33. SendsInFlightWithinLimit'
        BY <4>0, <4>30, Zenon DEF SendsInFlightWithinLimit
      <4>34. /\ write_dones_emitted \in [CallIds -> Nat]
             /\ \A c \in CallIds : Len(submitted[c]) \in Nat
        <5>1. /\ write_dones_emitted \in [CallIds -> Nat]
              /\ submitted \in [CallIds -> Seq(Messages)]
          BY <1>1, Zenon DEF TypeOK, L0!TypeOK
        <5>2. QED BY <5>1, LenProperties, Zenon
      <4>35. WriteDonesNeverExceedSends'
        BY <4>0, <4>30, <4>34, SMT DEF WriteDonesNeverExceedSends
      <4>4. QED
        BY <4>1, <4>2, <4>32, <4>33, <4>35, Zenon DEF FfiCallInv
    <3>2. QED BY <2>2, <3>1
\* Half-closing keeps the call active and touches nothing the FFI
\* invariants read besides the call state, so every conjunct transfers.
  <2>3. CASE \E cId \in CallIds : EndSend(cId)
    <3>1. ASSUME NEW cId \in CallIds, EndSend(cId)
          PROVE  FfiCallInv'
      <4>1. /\ call_state' = [call_state EXCEPT ![cId] = "half_closed"]
            /\ call_state[cId] \in {"started", "sending"}
            /\ UNCHANGED <<call_channel, channel_state, channel_runtime,
                           submitted, sent, received, delivered,
                           events_delivered, status_pending,
                           runtime_state>>
            /\ UNCHANGED ffi_vars
        BY <3>1, SMT DEF EndSend, L0!EndSend, L0!RuntimeVars,
            L0!ChannelVars, ffi_vars
      <4>2. /\ UNCHANGED <<payloads_consumed_by_host, events_delivered>>
            /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                     write_done_callback_running,
                           submitted>>
            /\ UNCHANGED handle_released
            /\ UNCHANGED cancel_requested
            /\ UNCHANGED delivery_callback_running
        BY <4>1, SMT DEF ffi_vars
      <4>3. \A c \in CallIds :
                /\ (L0!IsActiveCall(c))' =
                       (IF c = cId THEN TRUE ELSE L0!IsActiveCall(c))
                /\ (L0!IsTerminalCall(c))' =
                       (IF c = cId THEN FALSE ELSE L0!IsTerminalCall(c))
                /\ (L0!IsUnusedCall(c))' =
                       (IF c = cId THEN FALSE ELSE L0!IsUnusedCall(c))
                /\ (L0!HasStatus(c))' = L0!HasStatus(c)
                /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
                /\ (call_channel[c])' = call_channel[c]
        BY <1>1, <4>1, SMT DEF TypeOK, L0!TypeOK,
            L0!IsActiveCall, L0!ActiveCallStates, L0!IsTerminalCall,
            L0!IsUnusedCall, L0!HasStatus, L0!CallStates
      <4>30. /\ L0!IsActiveCall(cId)
             /\ ~L0!IsUnusedCall(cId)
             /\ ~L0!IsTerminalCall(cId)
        BY <1>1, <4>1, SMT DEF TypeOK, L0!TypeOK,
            L0!IsActiveCall, L0!ActiveCallStates, L0!IsTerminalCall,
            L0!IsUnusedCall, L0!CallStates
      <4>4. \A chan \in ChannelIds :
                (IsClosingChannel(chan))' = IsClosingChannel(chan)
        BY <4>1, SMT
      <4>5. \A c \in CallIds :
                /\ (payloads_consumed_by_host[c])' =
                       payloads_consumed_by_host[c]
                /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
                /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
                /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                       HostOwnsAtMostCreditsPlusOne(c)
        BY <4>2, PayloadReadingsFrame, Zenon
      <4>6. \A c \in CallIds :
                /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                /\ (IsWriteDoneCallbackRunning(c))' =
                       IsWriteDoneCallbackRunning(c)
                /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
        BY <4>2, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, Zenon
      <4>7. \A c \in CallIds :
                /\ (IsCancelRequested(c))' = IsCancelRequested(c)
                /\ (IsHandleReleased(c))' = IsHandleReleased(c)
                /\ (IsDeliveryCallbackRunning(c))' =
                       IsDeliveryCallbackRunning(c)
        BY <4>2, CancelFlagReadingsFrame, HandleFlagReadingsFrame,
           DeliveryFlagReadingsFrame, Zenon
\* The step needs an active call, and a released one is over, so
\* ReleasedCallIsClean stays vacuous for cId.
      <4>75. ~IsHandleReleased(cId)
        BY <1>1, <4>30, Zenon DEF FfiCallInv, ReleasedCallIsClean
      <4>80. (/\ ReleasedCallIsClean
              /\ ClosingChannelCallsCancelRequested
              /\ NoDeliveryImpliesNoDebt
              /\ ActiveCallHasNoStatus
              /\ UnusedCallHasNoEvents)'
        BY <1>1, <4>3, <4>30, <4>4, <4>5, <4>6, <4>7, <4>75, SMT
        DEF FfiCallInv, ReleasedCallIsClean,
            ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
            ActiveCallHasNoStatus, UnusedCallHasNoEvents
      <4>810. ActiveCallPayloadsWithinCredits'
        BY <1>1, <4>3, <4>30, <4>5, SMT
        DEF FfiCallInv, ActiveCallPayloadsWithinCredits
      <4>811. PayloadsOwnedWithinCreditsPlusOne'
        BY <1>1, <4>5, SMT
        DEF FfiCallInv, PayloadsOwnedWithinCreditsPlusOne
      <4>812. ReleasesNeverExceedDeliveries'
        BY <1>1, <4>1, <4>5, SMT
        DEF FfiCallInv, ReleasesNeverExceedDeliveries
      <4>81. (/\ ActiveCallPayloadsWithinCredits
              /\ PayloadsOwnedWithinCreditsPlusOne
              /\ ReleasesNeverExceedDeliveries)'
        BY <4>810, <4>811, <4>812, Zenon
      <4>820. UnusedCallsAreFfiClean'
        BY <1>1, <4>3, <4>5, <4>6, <4>7, SMT
        DEF FfiCallInv, UnusedCallsAreFfiClean
      <4>821. (/\ SendsInFlightWithinLimit
               /\ WriteDonesNeverExceedSends
               /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>1, <4>6, SMT
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>822. TerminalCallHasNoSendInFlight'
        BY <1>1, <4>3, <4>30, <4>6, SMT
        DEF FfiCallInv, TerminalCallHasNoSendInFlight
      <4>82. (/\ UnusedCallsAreFfiClean
              /\ SendsInFlightWithinLimit
              /\ WriteDonesNeverExceedSends
              /\ RunningWriteDoneWasEmitted
              /\ TerminalCallHasNoSendInFlight)'
        BY <4>820, <4>821, <4>822, Zenon
      <4>8. QED
        BY <4>80, <4>81, <4>82, Zenon DEF FfiCallInv
    <3>2. QED BY <2>3, <3>1
  <2>4. CASE \E cId \in CallIds : NetworkSend(cId)
    BY <1>1, <2>4, FfiFramePreservesFfiCallInv, SMT
    DEF NetworkSend, L0!NetworkSend, L0!RuntimeVars, L0!ChannelVars, ffi_vars
  <2>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
    BY <1>1, <2>5, FfiFramePreservesFfiCallInv, SMT
    DEF NetworkReceive, L0!NetworkReceive, L0!RuntimeVars, L0!ChannelVars, ffi_vars
  <2>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
    BY <1>1, <2>6, FfiFramePreservesFfiCallInv, SMT
    DEF ReceiveStatus, L0!ReceiveStatus, L0!RuntimeVars, L0!ChannelVars, ffi_vars
  <2>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
\* Delivering needs an active call and a released one is over, so the
\* implication stays vacuous for the call this step touches.
    <3>0. ReleasedCallIsClean'
      <4>1. ASSUME NEW cId \in CallIds, DeliverInitialMetadata(cId)
            PROVE  ReleasedCallIsClean'
        <5>1. /\ L0!IsActiveCall(cId)
              /\ UNCHANGED <<handle_released, buffers_held_by_host>>
              /\ \A c \in CallIds : c # cId =>
                     /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
                     /\ (IsDeliveryCallbackRunning(c))' =
                            IsDeliveryCallbackRunning(c)
                     /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          BY <1>1, <4>1, SMT
          DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot,
              L0!RuntimeVars, L0!ChannelVars, ffi_vars,
              TypeOK, L0!TypeOK,
              L0!IsActiveCall, L0!ActiveCallStates, L0!CallStates,
              IsDeliveryCallbackRunning, HostOwnsNoPayload, OwedPayloads
        <5>2. QED
          BY <1>1, <5>1, ActiveCallStepKeepsReleasedClean
      <4>2. QED BY <2>7, <4>1
    <3>1. (/\ ClosingChannelCallsCancelRequested
           /\ NoDeliveryImpliesNoDebt
           /\ ActiveCallHasNoStatus
           /\ UnusedCallHasNoEvents)'
      BY <1>1, <2>7, StatusKindsExpansion, SMTT(120)
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ClosingChannelCallsCancelRequested,
          NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus,
          UnusedCallHasNoEvents,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!ChannelStates, L0!CallStates, L0!HasStatus
    <3>2. (/\ ActiveCallPayloadsWithinCredits
           /\ PayloadsOwnedWithinCreditsPlusOne
           /\ ReleasesNeverExceedDeliveries)'
      BY <1>1, <2>7, DeliveryCreditsArePositive, SMTT(120)
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ActiveCallPayloadsWithinCredits, PayloadsOwnedWithinCreditsPlusOne,
          ReleasesNeverExceedDeliveries,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!CallStates, L0!HasStatus,
          L0!EventKinds, L0!StatusKinds
    <3>3. (/\ UnusedCallsAreFfiClean
           /\ SendsInFlightWithinLimit
           /\ WriteDonesNeverExceedSends
           /\ RunningWriteDoneWasEmitted
           /\ TerminalCallHasNoSendInFlight)'
      <4>10. UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                        write_done_callback_running, submitted>>
        BY <2>7, SMT DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, ffi_vars,
            L0!RuntimeVars, L0!ChannelVars
      <4>11. \A c \in CallIds :
                 /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                 /\ (IsWriteDoneCallbackRunning(c))' =
                        IsWriteDoneCallbackRunning(c)
                 /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                 /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                 /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                 /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
        BY <4>10, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, Zenon
      <4>120. (/\ SendsInFlightWithinLimit
               /\ WriteDonesNeverExceedSends
               /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>10, <4>11, Zenon
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>121. (/\ UnusedCallsAreFfiClean
               /\ TerminalCallHasNoSendInFlight)'
        BY <1>1, <2>7, <4>10, <4>11, SMTT(120)
          DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
              TypeOK, L0!TypeOK, FfiCallInv,
              UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
              WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
              TerminalCallHasNoSendInFlight,
              L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
              L0!ActiveCallStates, L0!CallStates, L0!HasStatus
      <4>12. QED
        BY <4>120, <4>121, Zenon DEF FfiCallInv
    <3>4. QED BY <3>0, <3>1, <3>2, <3>3, Zenon DEF FfiCallInv
  <2>8. CASE \E cId \in CallIds : DeliverMessage(cId)
\* Delivering needs an active call and a released one is over, so the
\* implication stays vacuous for the call this step touches.
    <3>0. ReleasedCallIsClean'
      <4>1. ASSUME NEW cId \in CallIds, DeliverMessage(cId)
            PROVE  ReleasedCallIsClean'
        <5>1. /\ L0!IsActiveCall(cId)
              /\ UNCHANGED <<handle_released, buffers_held_by_host>>
              /\ \A c \in CallIds : c # cId =>
                     /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
                     /\ (IsDeliveryCallbackRunning(c))' =
                            IsDeliveryCallbackRunning(c)
                     /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          BY <1>1, <4>1, SMT
          DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot,
              L0!RuntimeVars, L0!ChannelVars, ffi_vars,
              TypeOK, L0!TypeOK,
              L0!IsActiveCall, L0!ActiveCallStates, L0!CallStates,
              IsDeliveryCallbackRunning, HostOwnsNoPayload, OwedPayloads
        <5>2. QED
          BY <1>1, <5>1, ActiveCallStepKeepsReleasedClean
      <4>2. QED BY <2>8, <4>1
    <3>1. (/\ ClosingChannelCallsCancelRequested
           /\ NoDeliveryImpliesNoDebt
           /\ ActiveCallHasNoStatus
           /\ UnusedCallHasNoEvents)'
      BY <1>1, <2>8, StatusKindsExpansion, SMTT(120)
      DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ClosingChannelCallsCancelRequested,
          NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus,
          UnusedCallHasNoEvents,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!ChannelStates, L0!CallStates, L0!HasStatus
    <3>2. (/\ ActiveCallPayloadsWithinCredits
           /\ PayloadsOwnedWithinCreditsPlusOne
           /\ ReleasesNeverExceedDeliveries)'
      BY <1>1, <2>8, DeliveryCreditsArePositive, SMTT(120)
      DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ActiveCallPayloadsWithinCredits, PayloadsOwnedWithinCreditsPlusOne,
          ReleasesNeverExceedDeliveries,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!CallStates, L0!HasStatus,
          L0!EventKinds, L0!StatusKinds
    <3>3. (/\ UnusedCallsAreFfiClean
           /\ SendsInFlightWithinLimit
           /\ WriteDonesNeverExceedSends
           /\ RunningWriteDoneWasEmitted
           /\ TerminalCallHasNoSendInFlight)'
      <4>10. UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                        write_done_callback_running, submitted>>
        BY <2>8, SMT DEF DeliverMessage, L0!DeliverMessage, ffi_vars,
            L0!RuntimeVars, L0!ChannelVars
      <4>11. \A c \in CallIds :
                 /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                 /\ (IsWriteDoneCallbackRunning(c))' =
                        IsWriteDoneCallbackRunning(c)
                 /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                 /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                 /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                 /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
        BY <4>10, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, Zenon
      <4>120. (/\ SendsInFlightWithinLimit
               /\ WriteDonesNeverExceedSends
               /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>10, <4>11, Zenon
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>121. (/\ UnusedCallsAreFfiClean
               /\ TerminalCallHasNoSendInFlight)'
        BY <1>1, <2>8, <4>10, <4>11, SMTT(120)
          DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot, L0!RuntimeVars, L0!ChannelVars,
              TypeOK, L0!TypeOK, FfiCallInv,
              UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
              WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
              TerminalCallHasNoSendInFlight,
              L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
              L0!ActiveCallStates, L0!CallStates, L0!HasStatus
      <4>12. QED
        BY <4>120, <4>121, Zenon DEF FfiCallInv
    <3>4. QED BY <3>0, <3>1, <3>2, <3>3, Zenon DEF FfiCallInv
  <2>9. CASE \E cId \in CallIds : DeliverStatus(cId)
\* Delivering needs an active call and a released one is over, so the
\* implication stays vacuous for the call this step touches.
    <3>0. ReleasedCallIsClean'
      <4>1. ASSUME NEW cId \in CallIds, DeliverStatus(cId)
            PROVE  ReleasedCallIsClean'
        <5>1. /\ L0!IsActiveCall(cId)
              /\ UNCHANGED <<handle_released, buffers_held_by_host>>
              /\ \A c \in CallIds : c # cId =>
                     /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
                     /\ (IsDeliveryCallbackRunning(c))' =
                            IsDeliveryCallbackRunning(c)
                     /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          BY <1>1, <4>1, SMT
          DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
              L0!RuntimeVars, L0!ChannelVars, ffi_vars,
              TypeOK, L0!TypeOK,
              L0!IsActiveCall, L0!ActiveCallStates, L0!CallStates,
              IsDeliveryCallbackRunning, HostOwnsNoPayload, OwedPayloads
        <5>2. QED
          BY <1>1, <5>1, ActiveCallStepKeepsReleasedClean
      <4>2. QED BY <2>9, <4>1
    <3>1. (/\ ClosingChannelCallsCancelRequested
           /\ NoDeliveryImpliesNoDebt
           /\ ActiveCallHasNoStatus
           /\ UnusedCallHasNoEvents)'
      BY <1>1, <2>9, StatusKindsExpansion, SMTT(120)
      DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ClosingChannelCallsCancelRequested,
          NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus,
          UnusedCallHasNoEvents,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!ChannelStates, L0!CallStates, L0!HasStatus
    <3>2. (/\ ActiveCallPayloadsWithinCredits
           /\ PayloadsOwnedWithinCreditsPlusOne
           /\ ReleasesNeverExceedDeliveries)'
      BY <1>1, <2>9, DeliveryCreditsArePositive, SMTT(120)
      DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ActiveCallPayloadsWithinCredits, PayloadsOwnedWithinCreditsPlusOne,
          ReleasesNeverExceedDeliveries,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!CallStates, L0!HasStatus,
          L0!EventKinds, L0!StatusKinds
    <3>3. (/\ UnusedCallsAreFfiClean
           /\ SendsInFlightWithinLimit
           /\ WriteDonesNeverExceedSends
           /\ RunningWriteDoneWasEmitted
           /\ TerminalCallHasNoSendInFlight)'
      <4>10. UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                        write_done_callback_running, submitted>>
        BY <2>9, SMT DEF DeliverStatus, L0!DeliverStatus, ffi_vars,
            L0!RuntimeVars, L0!ChannelVars
      <4>11. \A c \in CallIds :
                 /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                 /\ (IsWriteDoneCallbackRunning(c))' =
                        IsWriteDoneCallbackRunning(c)
                 /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                 /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                 /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                 /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
        BY <4>10, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, Zenon
      <4>120. (/\ SendsInFlightWithinLimit
               /\ WriteDonesNeverExceedSends
               /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>10, <4>11, Zenon
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>121. (/\ UnusedCallsAreFfiClean
               /\ TerminalCallHasNoSendInFlight)'
        BY <1>1, <2>9, <4>10, <4>11, SMTT(120)
          DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
              TypeOK, L0!TypeOK, FfiCallInv,
              UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
              WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
              TerminalCallHasNoSendInFlight,
              L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
              L0!ActiveCallStates, L0!CallStates, L0!HasStatus
      <4>12. QED
        BY <4>120, <4>121, Zenon DEF FfiCallInv
    <3>4. QED BY <3>0, <3>1, <3>2, <3>3, Zenon DEF FfiCallInv
  <2>10. CASE \E cId \in CallIds : DeliverCancelled(cId)
\* Delivering needs an active call and a released one is over, so the
\* implication stays vacuous for the call this step touches.
    <3>0. ReleasedCallIsClean'
      <4>1. ASSUME NEW cId \in CallIds, DeliverCancelled(cId)
            PROVE  ReleasedCallIsClean'
        <5>1. /\ L0!IsActiveCall(cId)
              /\ UNCHANGED <<handle_released, buffers_held_by_host>>
              /\ \A c \in CallIds : c # cId =>
                     /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
                     /\ (IsDeliveryCallbackRunning(c))' =
                            IsDeliveryCallbackRunning(c)
                     /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          BY <1>1, <4>1, SMT
          DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
              L0!RuntimeVars, L0!ChannelVars, ffi_vars,
              TypeOK, L0!TypeOK,
              L0!IsActiveCall, L0!ActiveCallStates, L0!CallStates,
              IsDeliveryCallbackRunning, HostOwnsNoPayload, OwedPayloads
        <5>2. QED
          BY <1>1, <5>1, ActiveCallStepKeepsReleasedClean
      <4>2. QED BY <2>10, <4>1
    <3>1. (/\ ClosingChannelCallsCancelRequested
           /\ NoDeliveryImpliesNoDebt
           /\ ActiveCallHasNoStatus
           /\ UnusedCallHasNoEvents)'
      BY <1>1, <2>10, StatusKindsExpansion, SMTT(120)
      DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
          TypeOK, L0!TypeOK, FfiCallInv,
          ClosingChannelCallsCancelRequested,
          NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus,
          UnusedCallHasNoEvents,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!ChannelStates, L0!CallStates, L0!HasStatus
    <3>2. (/\ ActiveCallPayloadsWithinCredits
           /\ PayloadsOwnedWithinCreditsPlusOne
           /\ ReleasesNeverExceedDeliveries)'
      BY <1>1, <2>10, DeliveryCreditsArePositive, TypeOKSplit, SMTT(600)
      DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
          FfiTypes, L0!TypeOK, FfiCallInv,
          ActiveCallPayloadsWithinCredits, PayloadsOwnedWithinCreditsPlusOne,
          ReleasesNeverExceedDeliveries,
          L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!CallStates, L0!HasStatus,
          L0!EventKinds, L0!StatusKinds
    <3>3. (/\ UnusedCallsAreFfiClean
           /\ SendsInFlightWithinLimit
           /\ WriteDonesNeverExceedSends
           /\ RunningWriteDoneWasEmitted
           /\ TerminalCallHasNoSendInFlight)'
      <4>10. UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                        write_done_callback_running, submitted>>
        BY <2>10, SMT DEF DeliverCancelled, L0!CallCancel, ffi_vars,
            L0!RuntimeVars, L0!ChannelVars
      <4>11. \A c \in CallIds :
                 /\ (write_dones_emitted[c])' = write_dones_emitted[c]
                 /\ (IsWriteDoneCallbackRunning(c))' =
                        IsWriteDoneCallbackRunning(c)
                 /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
                 /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                 /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
                 /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
        BY <4>10, SendReadingsFrame, BufferReadingsFrame, SendWindowFrame, Zenon
      <4>120. (/\ SendsInFlightWithinLimit
               /\ WriteDonesNeverExceedSends
               /\ RunningWriteDoneWasEmitted)'
        BY <1>1, <4>10, <4>11, Zenon
        DEF FfiCallInv, SendsInFlightWithinLimit,
            WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
      <4>121. (/\ UnusedCallsAreFfiClean
               /\ TerminalCallHasNoSendInFlight)'
        BY <1>1, <2>10, <4>10, <4>11, SMTT(120)
          DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
              TypeOK, L0!TypeOK, FfiCallInv,
              UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
              WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
              TerminalCallHasNoSendInFlight,
              L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
              L0!ActiveCallStates, L0!CallStates, L0!HasStatus
      <4>12. QED
        BY <4>120, <4>121, Zenon DEF FfiCallInv
    <3>4. QED BY <3>0, <3>1, <3>2, <3>3, Zenon DEF FfiCallInv
  <2>11. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7,
                 <2>8, <2>9, <2>10 DEF NextSafeCallOnly
<1>2. QED BY <1>1

(***************************************************************************)
(* THE FFI-ONLY STEPS, ONE LEMMA EACH                                      *)
(* Each reads its own delta and the frozen level-0 side as plain facts,    *)
(* with the state predicates hidden: the invariant obligations never carry *)
(* an action body, so none of them grows past what a solver can hold.      *)
(***************************************************************************)

LEMMA CancelRequestKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv,
           RequestCallCancellation(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF RequestCallCancellation
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ ~IsHandleReleased(cId)
      /\ ~L0!IsUnusedCall(cId)
      /\ \A c \in CallIds :
             /\ (IsCancelRequested(c))' =
                    (IF c = cId THEN TRUE ELSE IsCancelRequested(c))
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
    BY CancelRequestTransfers, Zenon
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>4, <1>5, <1>6, Zenon DEF FfiCallInv

LEMMA ReleaseKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv, ReleaseCallHandle(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF ReleaseCallHandle
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
\* The guards are the four conjuncts ReleasedCallIsClean asks of the call
\* that has just been released, and this step disturbs none of them.
<1>3. /\ ~L0!IsUnusedCall(cId)
      /\ ~L0!IsActiveCall(cId)
      /\ HostOwnsNoPayload(cId)
      /\ HostHoldsNoBuffer(cId)
      /\ \A c \in CallIds :
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' =
                    (IF c = cId THEN TRUE ELSE IsHandleReleased(c))
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
    BY ReleaseTransfers, Zenon
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>4, <1>5, <1>6, Zenon DEF FfiCallInv

LEMMA DeliveryReturnKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv,
           DeliveryCallbackReturns(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF DeliveryCallbackReturns
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. \A c \in CallIds :
          /\ (IsDeliveryCallbackRunning(c))' =
                 (IF c = cId THEN FALSE ELSE IsDeliveryCallbackRunning(c))
          /\ (IsCancelRequested(c))' = IsCancelRequested(c)
          /\ (IsHandleReleased(c))' = IsHandleReleased(c)
          /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
          /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
          /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
          /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                 HostOwnsAtMostCreditsPlusOne(c)
          /\ (write_dones_emitted[c])' = write_dones_emitted[c]
          /\ (IsWriteDoneCallbackRunning(c))' =
                 IsWriteDoneCallbackRunning(c)
          /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
          /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
          /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
          /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY DeliveryReturnTransfers, Zenon
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>4, <1>5, <1>6, Zenon DEF FfiCallInv

LEMMA EmitWriteDoneKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv, EmitWriteDone(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF EmitWriteDone
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ write_dones_emitted[cId] < Len(submitted[cId])
      /\ ~HasNoSendInFlight(cId)
      /\ \A c \in CallIds :
             /\ (IsWriteDoneCallbackRunning(c))' =
                    (IF c = cId THEN TRUE
                     ELSE IsWriteDoneCallbackRunning(c))
             /\ (write_dones_emitted[c])' =
                    IF c = cId THEN write_dones_emitted[c] + 1
                    ELSE write_dones_emitted[c]
             /\ (SendWindowOccupancy(c))' =
                    (IF c = cId THEN SendWindowOccupancy(c) - 1
                     ELSE SendWindowOccupancy(c))
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
             /\ (HasNoSendInFlight(c))' =
                    (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
    BY EmitWriteDoneTransfers, Zenon
<1>30. /\ ~L0!IsUnusedCall(cId)
       /\ ~L0!IsTerminalCall(cId)
    BY <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, TerminalCallHasNoSendInFlight
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
\* The window shrinks here, so the bound only gets easier - but that is
\* arithmetic, which Zenon does not do.
<1>60. \A c \in CallIds :
           /\ SendWindowOccupancy(c) \in Int
           /\ SendWindowOccupancy(c) <= MaxSendsInFlight
    BY SendWindowCountsAreIntegers, Zenon
    DEF FfiCallInv, SendsInFlightWithinLimit
<1>605. \A c \in CallIds :
            (SendWindowOccupancy(c))' =
                (IF c = cId THEN SendWindowOccupancy(c) - 1
                 ELSE SendWindowOccupancy(c))
    BY <1>3, Zenon
<1>61. SendsInFlightWithinLimit'
    BY <1>60, <1>605, MaxSendsInFlightIsPositive, SMT
    DEF SendsInFlightWithinLimit
<1>6. (/\ UnusedCallsAreFfiClean
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, <1>30, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean,
        TerminalCallHasNoSendInFlight
<1>70. /\ WriteDonesNeverExceedSends
      /\ RunningWriteDoneWasEmitted
    BY Zenon DEF FfiCallInv
<1>71. /\ write_dones_emitted \in [CallIds -> Nat]
       /\ submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>72. \A c \in CallIds : Len(submitted[c]) \in Nat
    BY <1>71, LenProperties, Zenon
<1>7. (/\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted)'
    BY <1>2, <1>3, <1>70, <1>71, <1>72, SMT
    DEF WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
<1>8. QED
    BY <1>4, <1>5, <1>6, <1>61, <1>7, Zenon DEF FfiCallInv

LEMMA WriteDoneReturnKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv, WriteDoneReturns(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF WriteDoneReturns
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ IsWriteDoneCallbackRunning(cId)
      /\ ~HasNoSendInFlight(cId)
      /\ \A c \in CallIds :
             c # cId => (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
      /\ \A c \in CallIds :
             /\ (IsWriteDoneCallbackRunning(c))' =
                    (IF c = cId THEN FALSE
                     ELSE IsWriteDoneCallbackRunning(c))
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
    BY WriteDoneReturnsTransfers, Zenon
<1>30. /\ ~L0!IsUnusedCall(cId)
       /\ ~L0!IsTerminalCall(cId)
    BY <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, TerminalCallHasNoSendInFlight
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>60. /\ UnusedCallsAreFfiClean
      /\ TerminalCallHasNoSendInFlight
      /\ WriteDonesNeverExceedSends
      /\ RunningWriteDoneWasEmitted
      /\ SendsInFlightWithinLimit
    BY Zenon DEF FfiCallInv
<1>6. (/\ UnusedCallsAreFfiClean
       /\ TerminalCallHasNoSendInFlight
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted)'
    BY <1>2, <1>3, <1>30, <1>60, Zenon
    DEF UnusedCallsAreFfiClean, TerminalCallHasNoSendInFlight,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted
<1>71. /\ write_dones_emitted \in [CallIds -> Nat]
       /\ submitted \in [CallIds -> Seq(Messages)]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>72. \A c \in CallIds : Len(submitted[c]) \in Nat
    BY <1>71, LenProperties, Zenon
<1>73. \A c \in CallIds : SendWindowOccupancy(c) \in Int
    BY SendWindowCountsAreIntegers, Zenon
<1>7. SendsInFlightWithinLimit'
    BY <1>3, <1>60, <1>73, MaxSendsInFlightIsPositive, SMT
    DEF SendsInFlightWithinLimit
<1>8. QED
    BY <1>4, <1>5, <1>6, <1>7, Zenon DEF FfiCallInv

LEMMA ConsumeKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv,
           HostConsumesEvent(cId)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, SendWindowOccupancy, HasFreeSendSlot,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF HostConsumesEvent
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ HostOwnsSomePayload(cId)
      /\ \A c \in CallIds :
             /\ (payloads_consumed_by_host[c])' =
                    IF c = cId THEN payloads_consumed_by_host[c] + 1
                    ELSE payloads_consumed_by_host[c]
             /\ (events_delivered[c])' = events_delivered[c]
             /\ (OwedPayloads(c))' =
                    IF c = cId THEN OwedPayloads(c) - 1 ELSE OwedPayloads(c)
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (SendWindowOccupancy(c))' = SendWindowOccupancy(c)
             /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
             /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c)
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY ConsumeTransfers, Zenon
\* Releasing shrinks the debt by one, so every reading of it can only
\* improve: what was within a line stays within it.
<1>29. /\ payloads_consumed_by_host \in [CallIds -> Nat]
       /\ \A c \in CallIds : Len(events_delivered[c]) \in Nat
    <2>1. /\ payloads_consumed_by_host \in [CallIds -> Nat]
          /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
      BY Zenon DEF TypeOK, L0!TypeOK
    <2>2. QED BY <2>1, LenProperties, Zenon
<1>295. ASSUME NEW c \in CallIds
       PROVE  (OwedPayloads(c))' =
                  IF c = cId THEN OwedPayloads(c) - 1 ELSE OwedPayloads(c)
    BY <1>3, Zenon
\* The consumer owed something, so its debt cannot go below zero: that is
\* what makes the no-debt reading survive the step for every call.
<1>296. HostOwnsSomePayload(cId)
    BY <1>3, Zenon
<1>30. ASSUME NEW c \in CallIds
       PROVE  (OwedPayloads(c))' <= OwedPayloads(c)
    BY <1>295, <1>29, SMT
<1>310. ASSUME NEW c \in CallIds
       PROVE  HostOwnsAtMostCredits(c) => (HostOwnsAtMostCredits(c))'
    BY <1>295, <1>29, DeliveryCreditsArePositive, SMT
<1>311. ASSUME NEW c \in CallIds
       PROVE  HostOwnsAtMostCreditsPlusOne(c) =>
                  (HostOwnsAtMostCreditsPlusOne(c))'
    BY <1>295, <1>29, DeliveryCreditsArePositive, SMT
<1>31. ASSUME NEW c \in CallIds
       PROVE  /\ (HostOwnsAtMostCredits(c) => (HostOwnsAtMostCredits(c))')
              /\ (HostOwnsAtMostCreditsPlusOne(c) =>
                      (HostOwnsAtMostCreditsPlusOne(c))')
    BY <1>310, <1>311, Zenon
<1>32. ASSUME NEW c \in CallIds
       PROVE  HostOwnsNoPayload(c) => (HostOwnsNoPayload(c))'
    BY <1>295, <1>296, <1>29, SMT
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, <1>32, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>40. NoDeliveryImpliesNoDebt'
    BY <1>2, <1>3, <1>32, Zenon
    DEF FfiCallInv, NoDeliveryImpliesNoDebt
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne)'
    BY <1>2, <1>31, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne
<1>50. ReleasesNeverExceedDeliveries'
    BY <1>2, <1>3, <1>29, SMT
    DEF FfiCallInv, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, <1>32, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>4, <1>40, <1>5, <1>50, <1>6, Zenon DEF FfiCallInv

\* Lending takes the one slot the guard proved free, and everything else
\* is frozen.  Both call-scoped invariants are vacuous for the lender: it
\* is a started call, and a released one could not have got here.
LEMMA LendBufferKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, NEW bb \in BufferIds,
           NEW msg \in Messages, NEW ch \in Sizes,
           TypeOK, FfiCallInv, LendSendBuffer(cId, bb, msg, ch)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, HasFreeSendSlot, SendWindowOccupancy,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         IsHandleReleased, HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF LendSendBuffer
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ L0!IsActiveCall(cId)
      /\ ~L0!IsUnusedCall(cId)
      /\ ~IsHandleReleased(cId)
      /\ SendWindowOccupancy(cId) < MaxSendsInFlight
      /\ \A c \in CallIds :
             /\ (SendWindowOccupancy(c))' =
                    (IF c = cId THEN SendWindowOccupancy(c) + 1
                     ELSE SendWindowOccupancy(c))
             /\ (c # cId =>
                    /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                    /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' =
                    payloads_consumed_by_host[c]
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY LendBufferTransfers, Zenon
<1>34. \A c \in CallIds :
           (SendWindowOccupancy(c))' =
               (IF c = cId THEN SendWindowOccupancy(c) + 1
                ELSE SendWindowOccupancy(c))
    BY <1>3, Zenon
<1>35. \A c \in CallIds : SendWindowOccupancy(c) \in Int
    BY SendWindowCountsAreIntegers, Zenon
<1>33. /\ SendWindowOccupancy(cId) < MaxSendsInFlight
       /\ SendsInFlightWithinLimit
    BY <1>3, Zenon DEF FfiCallInv
<1>36. SendsInFlightWithinLimit'
    BY <1>33, <1>34, <1>35, MaxSendsInFlightIsPositive, SMT
    DEF SendsInFlightWithinLimit
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>36, <1>4, <1>5, <1>6, Zenon DEF FfiCallInv

\* Returning gives a slot back, so every count can only improve.  The
\* returner holds a buffer, which by the two call-scoped invariants means
\* the call is started and its handle is live.
LEMMA ReturnBufferKeepsFfiCallInv ==
    ASSUME NEW cId \in CallIds, NEW bb \in BufferIds,
           TypeOK, FfiCallInv, HostReturnsBuffer(cId, bb)
    PROVE  FfiCallInv'
<1> HIDE DEF HasNoSendInFlight, HasFreeSendSlot, SendWindowOccupancy,
         IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
         WriteDonesReturned, HasAcceptedSendAt, IsSendAcquittedAt,
         IsDeliveryCallbackRunning, IsCancelRequested,
         HasNoDeliveredEvents, IsClosingChannel
<1>1. UNCHANGED l0_vars
    BY SMT DEF HostReturnsBuffer
<1>2. /\ \A c \in CallIds :
             /\ (L0!IsActiveCall(c))' = L0!IsActiveCall(c)
             /\ (L0!IsTerminalCall(c))' = L0!IsTerminalCall(c)
             /\ (L0!IsUnusedCall(c))' = L0!IsUnusedCall(c)
             /\ (L0!HasStatus(c))' = L0!HasStatus(c)
             /\ (HasNoDeliveredEvents(c))' = HasNoDeliveredEvents(c)
             /\ (events_delivered[c])' = events_delivered[c]
             /\ Len((submitted[c])') = Len(submitted[c])
             /\ (call_channel[c])' = call_channel[c]
      /\ \A chan \in ChannelIds :
             (IsClosingChannel(chan))' = IsClosingChannel(chan)
    BY <1>1, L0FrameTransfers, Zenon
<1>3. /\ HostHoldsSomeBuffer(cId)
      /\ \A c \in CallIds :
             /\ (SendWindowOccupancy(c))' =
                    (IF c = cId THEN SendWindowOccupancy(c) - 1
                     ELSE SendWindowOccupancy(c))
             /\ (c # cId =>
                    /\ (HostHoldsNoBuffer(c))' = HostHoldsNoBuffer(c)
                    /\ (HostHoldsSomeBuffer(c))' = HostHoldsSomeBuffer(c))
             /\ (IsCancelRequested(c))' = IsCancelRequested(c)
             /\ (IsHandleReleased(c))' = IsHandleReleased(c)
             /\ (IsDeliveryCallbackRunning(c))' =
                    IsDeliveryCallbackRunning(c)
             /\ (HostOwnsNoPayload(c))' = HostOwnsNoPayload(c)
             /\ (HostOwnsAtMostCredits(c))' = HostOwnsAtMostCredits(c)
             /\ (HostOwnsAtMostCreditsPlusOne(c))' =
                    HostOwnsAtMostCreditsPlusOne(c)
             /\ (payloads_consumed_by_host[c])' =
                    payloads_consumed_by_host[c]
             /\ (IsWriteDoneCallbackRunning(c))' =
                    IsWriteDoneCallbackRunning(c)
             /\ (write_dones_emitted[c])' = write_dones_emitted[c]
             /\ (HasNoSendInFlight(c))' = HasNoSendInFlight(c)
    BY ReturnBufferTransfers, Zenon
\* Holding a buffer excludes both cases the call-scoped invariants speak
\* about, so their implications stay vacuous for cId.
<1>31. /\ ~L0!IsUnusedCall(cId)
       /\ ~IsHandleReleased(cId)
    BY <1>3, SMT
    DEF FfiCallInv, UnusedCallsAreFfiClean, ReleasedCallIsClean,
        HostHoldsSomeBuffer, HostHoldsNoBuffer
<1>34. \A c \in CallIds :
           (SendWindowOccupancy(c))' =
               (IF c = cId THEN SendWindowOccupancy(c) - 1
                ELSE SendWindowOccupancy(c))
    BY <1>3, Zenon
<1>35. \A c \in CallIds : SendWindowOccupancy(c) \in Int
    BY SendWindowCountsAreIntegers, Zenon
<1>33. SendsInFlightWithinLimit
    BY Zenon DEF FfiCallInv
<1>36. SendsInFlightWithinLimit'
    BY <1>33, <1>34, <1>35, MaxSendsInFlightIsPositive, SMT
    DEF SendsInFlightWithinLimit
<1>4. (/\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents)'
    BY <1>2, <1>3, <1>31, Zenon
    DEF FfiCallInv, ReleasedCallIsClean,
        ClosingChannelCallsCancelRequested, NoDeliveryImpliesNoDebt,
        ActiveCallHasNoStatus, UnusedCallHasNoEvents, IsHandleReleased
<1>5. (/\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ ReleasesNeverExceedDeliveries)'
    BY <1>2, <1>3, Zenon
    DEF FfiCallInv, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries
<1>6. (/\ UnusedCallsAreFfiClean
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight)'
    BY <1>2, <1>3, <1>31, Zenon
    DEF FfiCallInv, UnusedCallsAreFfiClean,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight
<1>7. QED
    BY <1>36, <1>4, <1>5, <1>6, Zenon DEF FfiCallInv

\* FfiCallInv reads the level-0 state and seven FFI variables, and none of
\* the buffer identities: freeze those and it is preserved.  This is what
\* lets FreeReturnedBuffer be dispatched in one line rather than getting a
\* per-action lemma of its own.
LEMMA UnchangedFfiKeepsFfiCallInv ==
    ASSUME FfiCallInv,
           UNCHANGED l0_vars,
           UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running,
                       delivery_callback_running,
                       payloads_consumed_by_host, handle_released,
                       cancel_requested>>
    PROVE  FfiCallInv'
<1>1. QED
    BY SMT
    DEF FfiCallInv, UnusedCallsAreFfiClean, ReleasedCallIsClean,
        SendsInFlightWithinLimit, WriteDonesNeverExceedSends,
        RunningWriteDoneWasEmitted, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, ReleasesNeverExceedDeliveries,
        NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        HasNoSendInFlight, HostHoldsNoBuffer, IsDeliveryCallbackRunning,
        HostOwnsNoPayload, IsHandleReleased, IsCancelRequested,
        IsWriteDoneCallbackRunning, WriteDonesReturned, SendWindowOccupancy,
        HostOwnsAtMostCredits, HostOwnsAtMostCreditsPlusOne, OwedPayloads,
        HasNoDeliveredEvents, IsClosingChannel,
        L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall, L0!HasStatus,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars

LEMMA FfiOnlyPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ NextSafeFfiOnly => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, NextSafeFfiOnly
      PROVE  FfiCallInv'
  <2>1. CASE NextSafeShutdownFfi
    <3>1. /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                         write_done_callback_running,
                         delivery_callback_running,
                         payloads_consumed_by_host,
                         handle_released, cancel_requested>>
          /\ UNCHANGED <<call_state, call_channel, channel_state,
                         events_delivered, submitted>>
      BY <2>1, SMT
      DEF NextSafeShutdownFfi, EmitShutdownComplete,
          ShutdownCallbackReturns, EmitResourcesReleased,
          ResourcesReleasedCallbackReturns, EmitResourcesReleased,
          ResourcesReleasedCallbackReturns, RuntimeDestroy,
          l0_vars, L0!vars, L0!RuntimeVars,
          L0!ChannelVars, L0!CallVars
    <3>2. QED
      BY <1>1, <3>1, FfiFramePreservesFfiCallInv
  <2>2. CASE NextSafeCallFfi
    <3>1. CASE \E cId \in CallIds : RequestCallCancellation(cId)
      BY <1>1, <3>1, CancelRequestKeepsFfiCallInv
    <3>2. CASE \E cId \in CallIds : ReleaseCallHandle(cId)
      BY <1>1, <3>2, ReleaseKeepsFfiCallInv
    <3>3. CASE \E cId \in CallIds : EmitWriteDone(cId)
      BY <1>1, <3>3, EmitWriteDoneKeepsFfiCallInv
    <3>4. CASE \E cId \in CallIds : WriteDoneReturns(cId)
      BY <1>1, <3>4, WriteDoneReturnKeepsFfiCallInv
    <3>5. CASE \E cId \in CallIds : DeliveryCallbackReturns(cId)
      BY <1>1, <3>5, DeliveryReturnKeepsFfiCallInv
    <3>6. CASE \E cId \in CallIds : HostConsumesEvent(cId)
      BY <1>1, <3>6, ConsumeKeepsFfiCallInv
    <3>60. CASE \E cId \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                   LendSendBuffer(cId, bb, msg, ch)
      BY <1>1, <3>60, LendBufferKeepsFfiCallInv
    <3>61. CASE \E cId \in CallIds, bb \in BufferIds :
                   HostReturnsBuffer(cId, bb)
      BY <1>1, <3>61, ReturnBufferKeepsFfiCallInv
\* Releasing the bytes of a buffer touches nothing FfiCallInv reads:
\* its buffer conjuncts live in BufferStateInv.
    <3>62. CASE \E cId \in CallIds, bb \in BufferIds :
                   FreeReturnedBuffer(cId, bb)
      <4>1. /\ UNCHANGED l0_vars
            /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                              write_done_callback_running,
                              delivery_callback_running,
                              payloads_consumed_by_host, handle_released,
                              cancel_requested>>
        BY <3>62, SMT DEF FreeReturnedBuffer
      <4>2. QED
        BY <1>1, <4>1, UnchangedFfiKeepsFfiCallInv
\* A refused lend writes the status and nothing else, so FfiCallInv reads
\* nothing it touches - the free's case, three actions over.
    <3>63. CASE \E cId \in CallIds, msg \in Messages :
                   RefuseLendTooLarge(cId, msg)
      <4>1. /\ UNCHANGED l0_vars
            /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                              write_done_callback_running,
                              delivery_callback_running,
                              payloads_consumed_by_host, handle_released,
                              cancel_requested>>
        BY <3>63, SMT DEF RefuseLendTooLarge
      <4>2. QED
        BY <1>1, <4>1, UnchangedFfiKeepsFfiCallInv
    <3>64. CASE \E cId \in CallIds, msg \in Messages :
                   RefuseLendForSlot(cId, msg)
      <4>1. /\ UNCHANGED l0_vars
            /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                              write_done_callback_running,
                              delivery_callback_running,
                              payloads_consumed_by_host, handle_released,
                              cancel_requested>>
        BY <3>64, SMT DEF RefuseLendForSlot
      <4>2. QED
        BY <1>1, <4>1, UnchangedFfiKeepsFfiCallInv
    <3>65. CASE \E cId \in CallIds, msg \in Messages, charge \in Sizes :
                   RefuseLendForBudget(cId, msg, charge)
      <4>1. /\ UNCHANGED l0_vars
            /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                              write_done_callback_running,
                              delivery_callback_running,
                              payloads_consumed_by_host, handle_released,
                              cancel_requested>>
        BY <3>65, SMT DEF RefuseLendForBudget
      <4>2. QED
        BY <1>1, <4>1, UnchangedFfiKeepsFfiCallInv
    <3>7. QED BY <1>1, <2>2, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>60,
                  <3>61, <3>62, <3>63, <3>64, <3>65 DEF NextSafeCallFfi
  <2>3. QED BY <1>1, <2>1, <2>2 DEF NextSafeFfiOnly
<1>2. QED BY <1>1

LEMMA NextPreservesFfiCallInv ==
    FfiCallInv /\ TypeOK /\ Next => FfiCallInv'
<1>1. ASSUME FfiCallInv, TypeOK, Next
      PROVE  FfiCallInv'
  <2>1. CASE NextSafeRefining
    <3>1. CASE NextSafeRuntimeOnly
      BY <1>1, <3>1, RuntimeOnlyPreservesFfiCallInv
    <3>2. CASE NextSafeRuntimeChannel
      BY <1>1, <3>2, RuntimeChannelPreservesFfiCallInv
    <3>3. CASE NextSafeChannelOnly
      BY <1>1, <3>3, ChannelOnlyPreservesFfiCallInv
    <3>4. CASE NextSafeChannelCall
      BY <1>1, <3>4, ChannelCallPreservesFfiCallInv
    <3>5. CASE NextSafeCallOnly
      BY <1>1, <3>5, CallOnlyPreservesFfiCallInv
    <3>6. QED BY <2>1, <3>1, <3>2, <3>3, <3>4, <3>5 DEF NextSafeRefining
  <2>2. CASE NextSafeFfiOnly
    BY <1>1, <2>2, FfiOnlyPreservesFfiCallInv
  <2>3. CASE NextFail
    BY <1>1, <2>3, FfiFramePreservesFfiCallInv, SMT
    DEF NextFail, RuntimeFail, L0!RuntimeFail,
        L0!ChannelVars, L0!CallVars, ffi_vars
  <2>4. CASE NextExplicitStutter
    BY <1>1, <2>4, FfiFramePreservesFfiCallInv, SMT
    DEF NextExplicitStutter, RemainFailed, RemainReleased,
        L0!RemainFailed, L0!RemainReleased, L0!vars, ffi_vars
  <2>5. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, NextDecomposition
        DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

(***************************************************************************)
(* FAILURE BOOKKEEPING                                                     *)
(***************************************************************************)

\* A failed runtime never recovers: every runtime action writes a slot
\* whose source state is not FAILED_UNQUIESCED.
LEMMA FailedPersists ==
    TypeOK /\ ~L0!NotFailed /\ Next => ~L0!NotFailed'
<1>1. ASSUME TypeOK, ~L0!NotFailed, Next
      PROVE  ~L0!NotFailed'
  <2>1. CASE NextSafeRefining
    BY <1>1, <2>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, DeliverCancelled,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!RuntimeBeginShutdown,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
        L0!NotFailed, TypeOK, L0!TypeOK, L0!RuntimeStates
  <2>2. CASE NextSafeFfiOnly
    BY <1>1, <2>2, FfiOnlyStutters, SMT DEF l0_vars, L0!vars, L0!NotFailed
  <2>3. CASE NextFail
    BY <1>1, <2>3, SMT
    DEF NextFail, RuntimeFail, L0!RuntimeFail, L0!NotFailed,
        TypeOK, L0!TypeOK
  <2>4. CASE NextExplicitStutter
    BY <1>1, <2>4, StutterProjects, SMT
    DEF L0!NextExplicitStutter, L0!RemainFailed, L0!RemainReleased,
        L0!vars, L0!NotFailed
  <2>5. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, NextDecomposition
        DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

\* No safe action fails a runtime.
LEMMA SafeKeepsNotFailed ==
    TypeOK /\ L0!NotFailed /\ [NextSafe]_vars => L0!NotFailed'
<1>1. ASSUME TypeOK, L0!NotFailed, [NextSafe]_vars
      PROVE  L0!NotFailed'
  <2>1. CASE NextSafeRefining
    BY <1>1, <2>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
        ReceiveStatus, DeliverInitialMetadata, DeliverMessage,
        DeliverStatus, DeliverCancelled,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!RuntimeBeginShutdown,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
        L0!NotFailed, TypeOK, L0!TypeOK, L0!RuntimeStates
  <2>2. CASE NextSafeFfiOnly
    BY <1>1, <2>2, FfiOnlyStutters, SMT DEF l0_vars, L0!vars, L0!NotFailed
  <2>3. CASE UNCHANGED vars
    BY <1>1, <2>3, SMT DEF vars, l0_vars, L0!vars, ffi_vars, L0!NotFailed
  <2>4. QED BY <1>1, <2>1, <2>2, <2>3 DEF NextSafe
<1>2. QED BY <1>1

(***************************************************************************)
(* SHUTDOWN SIGNAL DISCIPLINE PRESERVED                                    *)
(* Needs the full StrongInv: an active call lives on an active channel,    *)
(* so it cannot belong to a quiesced runtime and no delivery or send can   *)
(* break an established IsRuntimeDrained.                                          *)
(***************************************************************************)

\* An empty ledger survives a stutter.  Isolated because the ledger is a
\* difference of a sequence length and a counter, quantified over the calls:
\* the equalities have to reach the arithmetic as ground facts, which is not
\* something a call carrying the whole invariant can arrange.
LEMMA StutterKeepsReclaimable ==
    ASSUME NEW rtId \in RuntimeIds, UNCHANGED vars,
           NoHostDebt(rtId)
    PROVE  (NoHostDebt(rtId))'
<1>1. /\ call_channel' = call_channel
      /\ channel_runtime' = channel_runtime
      /\ events_delivered' = events_delivered
      /\ payloads_consumed_by_host' = payloads_consumed_by_host
      /\ buffers_held_by_host' = buffers_held_by_host
    BY SMT DEF vars, l0_vars, L0!vars, ffi_vars
<1>2. SUFFICES ASSUME NEW cId \in CallIds,
                      (call_channel[cId])' \in (L0!ChannelsOf(rtId))'
               PROVE  (HostOwnsNoPayload(cId))' /\ (HostHoldsNoBuffer(cId))'
    BY Zenon DEF NoHostDebt
<1>3. call_channel[cId] \in L0!ChannelsOf(rtId)
    BY <1>1, <1>2, Zenon DEF L0!ChannelsOf
<1>4. HostOwnsNoPayload(cId) /\ HostHoldsNoBuffer(cId)
    BY <1>3, Zenon DEF NoHostDebt
\* Pointwise before arithmetic: the last call should have no function
\* extensionality left to do.
<1>45. /\ (events_delivered[cId])' = events_delivered[cId]
       /\ (payloads_consumed_by_host[cId])' = payloads_consumed_by_host[cId]
       /\ (buffers_held_by_host[cId])' = buffers_held_by_host[cId]
    BY <1>1
<1>5. QED BY <1>4, <1>45, SMT DEF HostHoldsNoBuffer

LEMMA StutterPreservesShutdownSignal ==
    ShutdownSignalInv /\ UNCHANGED vars => ShutdownSignalInv'
\* One step per conjunct of the goal.  The monolithic call carried this while
\* the invariant had one quantified conjunct; with two it is over budget, and
\* splitting the goal is cheaper than teaching one backend both jobs.
<1>1. SUFFICES ASSUME ShutdownSignalInv, UNCHANGED vars,
                      NEW rtId \in RuntimeIds
               PROVE  /\ (IsShutdownCallbackRunning(rtId) =>
                             IsShutdownEventEmitted(rtId))'
                      /\ (IsShutdownEventEmitted(rtId) =>
                             (IsStoppingRuntime(rtId) \/
                                  IsReleasedRuntime(rtId)))'
                      /\ (IsReleasedRuntime(rtId) =>
                             /\ IsShutdownEventEmitted(rtId)
                             /\ ~IsShutdownCallbackRunning(rtId))'
                      /\ (IsShutdownEventEmitted(rtId) =>
                             IsRuntimeDrained(rtId))'
                      /\ (IsResourcesReleasedCallbackRunning(rtId) =>
                             IsResourcesReleasedEmitted(rtId))'
                      /\ (IsResourcesReleasedEmitted(rtId) =>
                             /\ IsShutdownEventEmitted(rtId)
                             /\ SecondEventOwed(rtId))'
                      /\ (IsShutdownEventEmitted(rtId) /\
                             ~SecondEventOwed(rtId) =>
                                 NoHostDebt(rtId))'
                      /\ (IsResourcesReleasedEmitted(rtId) =>
                             /\ NoHostDebt(rtId)
                             /\ RuntimeHoldsNoReturnedBytes(rtId))'
    BY Zenon DEF ShutdownSignalInv, ShutdownSignalCore, ReleaseSignalInv
<1>2. /\ (IsShutdownCallbackRunning(rtId) => IsShutdownEventEmitted(rtId))'
      /\ (IsShutdownEventEmitted(rtId) =>
             (IsStoppingRuntime(rtId) \/ IsReleasedRuntime(rtId)))'
      /\ (IsReleasedRuntime(rtId) =>
             /\ IsShutdownEventEmitted(rtId)
             /\ ~IsShutdownCallbackRunning(rtId))'
      /\ (IsResourcesReleasedCallbackRunning(rtId) =>
             IsResourcesReleasedEmitted(rtId))'
      /\ (IsResourcesReleasedEmitted(rtId) =>
             /\ IsShutdownEventEmitted(rtId)
             /\ SecondEventOwed(rtId))'
    BY <1>1, SMT DEF vars, l0_vars, L0!vars, ffi_vars, ShutdownSignalInv,
        ShutdownSignalCore, ReleaseSignalInv
<1>3. (IsShutdownEventEmitted(rtId) => IsRuntimeDrained(rtId))'
    BY <1>1, SMT
    DEF vars, l0_vars, L0!vars, ffi_vars, ShutdownSignalInv,
        ShutdownSignalCore, ReleaseSignalInv,
        IsRuntimeDrained, L0!ChannelsOf
\* The instance first, as a ground fact: carrying the whole invariant into a
\* call that also expands the ledger is what put this one over budget.
<1>35. IsShutdownEventEmitted(rtId) /\ ~SecondEventOwed(rtId)
           => NoHostDebt(rtId)
    BY <1>1, Zenon DEF ShutdownSignalInv, ShutdownSignalCore,
        ReleaseSignalInv
<1>4. (IsShutdownEventEmitted(rtId) /\ ~SecondEventOwed(rtId) =>
           NoHostDebt(rtId))'
    BY <1>1, <1>35, StutterKeepsReclaimable, SMT
    DEF vars, l0_vars, L0!vars, ffi_vars
\* The fourth release conjunct is frozen twice over: nothing the stutter leaves
\* alone is what it reads.
<1>45. IsResourcesReleasedEmitted(rtId) =>
           /\ NoHostDebt(rtId)
           /\ RuntimeHoldsNoReturnedBytes(rtId)
    BY <1>1, Zenon DEF ShutdownSignalInv, ShutdownSignalCore,
        ReleaseSignalInv
<1>46. (IsResourcesReleasedEmitted(rtId) =>
            /\ NoHostDebt(rtId)
            /\ RuntimeHoldsNoReturnedBytes(rtId))'
    BY <1>1, <1>45, StutterKeepsReclaimable, SMT
    DEF vars, l0_vars, L0!vars, ffi_vars, RuntimeHoldsNoReturnedBytes,
        IsReturnedBuffer, L0!ChannelsOf, L0!RuntimeVars, L0!ChannelVars,
        L0!CallVars
<1>5. QED BY <1>2, <1>3, <1>4, <1>46

\* A released runtime cannot be handed a debt back.  Its calls are all
\* terminal (L0!ReleasedNoCalls), and the two steps that can raise what
\* the host owes - a delivery and a lend - both need an active call.  The
\* two that lower it are always welcome.  Nothing can join the runtime
\* either: starting a call needs an open channel on a RUNNING runtime,
\* and creating a channel needs the same, so neither the channel set nor
\* the call-to-channel map moves for a released runtime.
LEMMA EveryStepEitherDeliversOrKeepsEvents ==
    ASSUME TypeOK, [Next]_vars
    PROVE  \/ \E c \in CallIds : DeliverInitialMetadata(c)
           \/ \E c \in CallIds : DeliverMessage(c)
           \/ \E c \in CallIds : DeliverStatus(c)
           \/ \E c \in CallIds : DeliverCancelled(c)
           \/ UNCHANGED events_delivered
<1>1. CASE NextSafeRefining
  <2>1. CASE NextSafeCallOnly
    BY <2>1, SMT
    DEF NextSafeCallOnly, CallStart, SendMessage, EndSend, NetworkSend,
        NetworkReceive, ReceiveStatus, L0!CallStart, L0!SendMessage,
        L0!EndSend, L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
        L0!RuntimeVars, L0!ChannelVars
  <2>20. CASE NextSafeRuntimeOnly
    BY <2>20, SMT
    DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!ChannelVars, L0!CallVars
  <2>21. CASE NextSafeRuntimeChannel
    BY <2>21, SMT
    DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
        RequestCancellationOfActiveCalls, L0!RuntimeBeginShutdown,
        L0!ChannelVars, L0!CallVars, L0!ChannelsOf
  <2>22. CASE NextSafeChannelOnly
    BY <2>22, SMT
    DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
        RequestCancellationOfActiveCalls, L0!ChannelCreate,
        L0!ChannelStartClosing, L0!RuntimeVars, L0!CallVars, L0!CallsOf
\* Closing a channel cancels its active calls at level 0, which would
\* append terminals - but level 1 only lets it fire once none is active,
\* so the mass cancellation is a no-op and the stream stands still.
  <2>23. CASE NextSafeChannelCall
    <3>1. PICK chId \in ChannelIds : ChannelFinishClosing(chId)
      BY <2>23, Zenon DEF NextSafeChannelCall
    <3>2. \A c \in CallIds :
              ~(call_channel[c] = chId /\ L0!IsActiveCall(c))
      BY <3>1, Zenon DEF ChannelFinishClosing, L0!CallsOf
    <3>3. events_delivered' =
              [c \in CallIds |-> events_delivered[c]]
      BY <3>1, <3>2, Zenon
      DEF ChannelFinishClosing, L0!ChannelFinishClosing
    <3>4. events_delivered = [c \in CallIds |-> events_delivered[c]]
      BY Zenon DEF TypeOK, L0!TypeOK
    <3>5. QED BY <3>3, <3>4, Zenon
  <2>3. QED
    BY <1>1, <2>1, <2>20, <2>21, <2>22, <2>23 DEF NextSafeRefining
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        IsRuntimeDrained, l0_vars, L0!vars
<1>3. CASE NextFail
    BY <1>3, SMT
    DEF NextFail, RuntimeFail, L0!RuntimeFail, L0!ChannelVars, L0!CallVars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT
    DEF NextExplicitStutter, RemainFailed, RemainReleased,
        L0!RemainFailed, L0!RemainReleased, L0!vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, l0_vars, L0!vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

LEMMA EveryStepEitherConsumesOrKeepsReleases ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds : HostConsumesEvent(c)
           \/ UNCHANGED payloads_consumed_by_host
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* The send-side footprints, same shape as the delivery one: only these
\* actions touch these variables.  The frames below are built from them
\* rather than from a case analysis over Next, which is what keeps their
\* obligations small enough to discharge.
LEMMA EveryStepEitherEmitsOrKeepsWriteDones ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds : EmitWriteDone(c)
           \/ UNCHANGED write_dones_emitted
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget,
        HostReturnsBuffer, FreeReturnedBuffer, WriteDoneReturns, DeliveryCallbackReturns,
        HostConsumesEvent
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

LEMMA EveryStepEitherAcquitsOrKeepsWriteDoneFlag ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds : EmitWriteDone(c)
           \/ \E c \in CallIds : WriteDoneReturns(c)
           \/ UNCHANGED write_done_callback_running
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, SendMessage, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget,
        HostReturnsBuffer, FreeReturnedBuffer, DeliveryCallbackReturns, HostConsumesEvent
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

LEMMA EveryStepEitherSubmitsOrKeepsSubmitted ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds, m \in Messages,
                 bs \in BufferIds : SendMessage(c, m, bs)
           \/ UNCHANGED submitted
<1>1. CASE NextSafeRefining
    BY <1>1, SMTT(120)
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, EndSend,
        L0!CallStart, L0!EndSend, NetworkSend, L0!NetworkSend,
        NetworkReceive, L0!NetworkReceive, ReceiveStatus, L0!ReceiveStatus,
        DeliverInitialMetadata, L0!DeliverInitialMetadata,
        DeliverMessage, L0!DeliverMessage, DeliverStatus, L0!DeliverStatus,
        DeliverCancelled, L0!CallCancel, HandPayloadToHost,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget,
        HostReturnsBuffer, FreeReturnedBuffer, EmitWriteDone, WriteDoneReturns,
        DeliveryCallbackReturns, HostConsumesEvent, l0_vars, L0!vars
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, L0!RuntimeFail,
        L0!ChannelVars, L0!CallVars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        L0!RemainFailed, L0!RemainReleased, L0!vars, l0_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, l0_vars, L0!vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* A footprint lemma for the send buffers, the mirror of the delivery one:
\* only a lend raises what the host holds.
LEMMA EveryStepEitherLendsOrKeepsBuffers ==
    ASSUME TypeOK, [Next]_vars
    PROVE  \/ \E c \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                 LendSendBuffer(c, bb, msg, ch)
           \/ \E c \in CallIds, bb \in BufferIds :
                      HostReturnsBuffer(c, bb)
           \/ \E c \in CallIds, m \in Messages,
                 bs \in BufferIds : SendMessage(c, m, bs)
           \/ UNCHANGED buffers_held_by_host
<1>1. CASE NextSafeRefining
    BY <1>1, SMT
    DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        RuntimeCreate, RuntimeBeginShutdown, RuntimeRelease,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        RequestCancellationOfActiveCalls, CallStart, EndSend,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, ffi_vars
<1>2. CASE NextSafeFfiOnly
    BY <1>2, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent
<1>3. CASE NextFail
    BY <1>3, SMT DEF NextFail, RuntimeFail, ffi_vars
<1>4. CASE NextExplicitStutter
    BY <1>4, SMT DEF NextExplicitStutter, RemainFailed, RemainReleased,
        ffi_vars
<1>5. CASE UNCHANGED vars
    BY <1>5, SMT DEF vars, ffi_vars
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, NextDecomposition
    DEF NextByFootprint, NextSafe

\* The reclamation argument itself, stated on the two facts it actually
\* needs: this runtime's calls are not active, and no call joins it over the
\* step.  Both a released runtime and one that has only emitted its shutdown
\* event satisfy them, by different routes, and neither route belongs here.
LEMMA QuietRuntimeStaysReclaimable ==
    ASSUME TypeOK, Next, NEW rtId \in RuntimeIds,
           NoHostDebt(rtId),
           \A c \in CallIds :
               call_channel[c] \in L0!ChannelsOf(rtId) =>
                   ~L0!IsActiveCall(c),
           \A c \in CallIds :
               (call_channel[c])' \in (L0!ChannelsOf(rtId))' =>
                   call_channel[c] \in L0!ChannelsOf(rtId)
    PROVE  (NoHostDebt(rtId))'
<1>0. TypeOK
    OBVIOUS
<1>01. \A c \in CallIds :
           call_channel[c] \in L0!ChannelsOf(rtId) => ~L0!IsActiveCall(c)
    OBVIOUS
<1>02. \A c \in CallIds :
           call_channel[c] \in L0!ChannelsOf(rtId) =>
               HostOwnsNoPayload(c) /\ HostHoldsNoBuffer(c)
    BY Zenon DEF NoHostDebt
<1>1. \A c \in CallIds :
          (call_channel[c])' \in (L0!ChannelsOf(rtId))' =>
              call_channel[c] \in L0!ChannelsOf(rtId)
    OBVIOUS
\* The debt cannot come back: a delivery needs an active call, and so
\* does a lend, so neither can name a call of this runtime.
<1>2. ASSUME NEW c \in CallIds,
             call_channel[c] \in L0!ChannelsOf(rtId)
      PROVE  (HostOwnsNoPayload(c))' /\ (HostHoldsNoBuffer(c))'
  <2>0. /\ ~L0!IsActiveCall(c)
        /\ HostOwnsNoPayload(c)
        /\ HostHoldsNoBuffer(c)
    BY <1>01, <1>02, <1>2, Zenon
\* A delivery names an active call, and this one is not, so whichever call
\* it names it is not this one and this one's events do not move.
  <2>1. (events_delivered[c])' = events_delivered[c]
    <3>1. CASE \E d \in CallIds : DeliverInitialMetadata(d)
      BY <1>0, <2>0, <3>1, SMT
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
          L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
    <3>2. CASE \E d \in CallIds : DeliverMessage(d)
      BY <1>0, <2>0, <3>2, SMT
      DEF DeliverMessage, L0!DeliverMessage,
          L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
    <3>3. CASE \E d \in CallIds : DeliverStatus(d)
      BY <1>0, <2>0, <3>3, SMT
      DEF DeliverStatus, L0!DeliverStatus,
          L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
    <3>4. CASE \E d \in CallIds : DeliverCancelled(d)
      BY <1>0, <2>0, <3>4, SMT
      DEF DeliverCancelled, L0!CallCancel,
          L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
    <3>5. QED
      BY <1>0, <3>1, <3>2, <3>3, <3>4,
         EveryStepEitherDeliversOrKeepsEvents, Zenon
\* Same shape on the send side: lending needs an active call, and both
\* ways of giving a buffer back need one to be held.
  <2>2. (buffers_held_by_host[c])' = buffers_held_by_host[c]
    <3>1. CASE \E d \in CallIds, bq \in BufferIds, msg \in Messages, ch \in Sizes :
                  LendSendBuffer(d, bq, msg, ch)
      BY <1>0, <2>0, <3>1, SMT
      DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, L0!IsActiveCall, L0!ActiveCallStates,
          TypeOK, L0!TypeOK
    <3>2. CASE \E d \in CallIds, bq \in BufferIds :
                  HostReturnsBuffer(d, bq)
      BY <1>0, <2>0, <3>2, SMT
      DEF HostReturnsBuffer, FreeReturnedBuffer, HostHoldsNoBuffer, HostHoldsSomeBuffer,
          TypeOK
    <3>3. CASE \E d \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(d, m, bs)
      BY <1>0, <2>0, <3>3, SMT
      DEF SendMessage, HostHoldsNoBuffer, HostHoldsSomeBuffer, TypeOK
    <3>4. QED
      BY <1>0, <3>1, <3>2, <3>3,
         EveryStepEitherLendsOrKeepsBuffers, Zenon
\* Owing nothing, there is nothing to consume either, so the release
\* counter is frozen alongside the event count.
  <2>3. (HostOwnsNoPayload(c))'
    <3>1. (payloads_consumed_by_host[c])' = payloads_consumed_by_host[c]
      <4>1. CASE \E d \in CallIds : HostConsumesEvent(d)
        BY <1>0, <2>0, <4>1, SMT
        DEF HostConsumesEvent, HostOwnsSomePayload, HostOwnsNoPayload,
            OwedPayloads, TypeOK
      <4>2. QED
        BY <4>1, EveryStepEitherConsumesOrKeepsReleases, Zenon
    <3>2. QED
      BY <2>0, <2>1, <3>1, SMT DEF HostOwnsNoPayload, OwedPayloads
  <2>4. QED
    BY <2>0, <2>2, <2>3, Zenon DEF HostHoldsNoBuffer

<1>3. QED
    BY <1>1, <1>2, Zenon DEF NoHostDebt

\* A released runtime cannot be handed a debt back.  Its calls are all
\* terminal - an active call needs an open or closing channel, and a
\* released runtime has neither - and the only two steps that raise what
\* the host owes, a delivery and a lend, both need an active call.  The
\* steps that lower it are always welcome.  Nothing joins the runtime
\* either: starting a call needs an open channel on a RUNNING runtime.
LEMMA ReleasedRuntimeStaysClean ==
    ASSUME StrongInv, Next, NEW rtId \in RuntimeIds,
           IsReleasedRuntime(rtId), NoHostDebt(rtId)
    PROVE  (IsReleasedRuntime(rtId))' /\ (NoHostDebt(rtId))'
<1>0. TypeOK
    BY Zenon DEF StrongInv
\* An active call hangs off an active channel, and an active channel hangs
\* off a runtime that is not released.
<1>01. \A c \in CallIds :
           call_channel[c] \in L0!ChannelsOf(rtId) => ~L0!IsActiveCall(c)
    BY SMT
    DEF StrongInv, L0!StrongInv, L0!StructuralInv, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!UsedChannels, L0!UsedCalls,
        L0!ActiveChannels, L0!ActiveChannelStates, L0!ChannelsOf,
        L0!IsActiveCall, L0!ActiveCallStates, L0!IsUnusedCall,
        IsReleasedRuntime, TypeOK, L0!TypeOK, L0!ChannelStates,
        L0!CallStates, L0!RuntimeStates
<1>02. \A c \in CallIds :
           call_channel[c] \in L0!ChannelsOf(rtId) =>
               HostOwnsNoPayload(c) /\ HostHoldsNoBuffer(c)
    BY Zenon DEF NoHostDebt
\* RELEASED is absorbing: every step that writes runtime_state demands a
\* different prior state, and none of them creates a channel here either.
<1>1. /\ (IsReleasedRuntime(rtId))'
      /\ \A c \in CallIds :
             (call_channel[c])' \in L0!ChannelsOf(rtId)' =>
                 call_channel[c] \in L0!ChannelsOf(rtId)
    BY <1>0, SMT
    DEF Next, NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        NextFail, NextExplicitStutter,
        RuntimeCreate, RuntimeBeginShutdown, EmitShutdownComplete,
        ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeRelease, RuntimeDestroy,
        RuntimeFail, RemainFailed, RemainReleased,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, RequestCallCancellation, ReleaseCallHandle,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        SendMessage, EndSend, EmitWriteDone, WriteDoneReturns,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, DeliveryCallbackReturns, HostConsumesEvent,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        L0!RuntimeFail, L0!RemainFailed, L0!RemainReleased,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
        vars, ffi_vars, RequestCancellationOfActiveCalls,
        HandPayloadToHost, HasFreeDeliverySlot,
        HasFreeDeliverySlotForTerminal, IsRuntimeDrained,
        L0!ChannelsOf, IsReleasedRuntime, TypeOK, L0!TypeOK
<1>2. QED
    BY <1>0, <1>01, <1>1, QuietRuntimeStaysReclaimable, Zenon

\* A call never returns to unused - no action writes call_state back to
\* "none" - so the only way to break the invariant is to allocate on a
\* call that is still unused, and lending needs an active one.
LEMMA NextPreservesFreshBuffers ==
    TypeOK /\ UnusedCallsHaveFreshBuffers /\ Next =>
        UnusedCallsHaveFreshBuffers'
<1>1. SUFFICES ASSUME TypeOK, UnusedCallsHaveFreshBuffers, Next,
                      NEW c \in CallIds, NEW b \in BufferIds,
                      (L0!IsUnusedCall(c))'
               PROVE  (IsFreshBuffer(c, b))'
    BY Zenon DEF UnusedCallsHaveFreshBuffers
\* Unused is where a call starts and never comes back to, so a call
\* unused after the step was unused before it.
<1>2. L0!IsUnusedCall(c)
  <2>1. CASE \/ NextSafeRuntimeOnly
             \/ NextSafeRuntimeChannel
             \/ NextSafeChannelOnly
    <3>1. UNCHANGED L0!CallVars
      BY <2>1, RuntimeAndChannelStepsKeepCalls
    <3>2. QED BY <1>1, <3>1, SMT DEF L0!CallVars, L0!IsUnusedCall, L0!CallStates, TypeOK, L0!TypeOK
  <2>2. CASE NextSafeChannelCall
    BY <1>1, <2>2, SMTT(45)
    DEF NextSafeChannelCall, ChannelFinishClosing, L0!ChannelFinishClosing,
        L0!ChannelsOf, L0!CallsOf, L0!IsActiveCall, L0!ActiveCallStates,
        L0!ChannelStates, L0!CallVars, L0!IsUnusedCall, L0!CallStates, TypeOK, L0!TypeOK
  <2>3. CASE NextSafeCallOnly
    BY <1>1, <2>3, SMTT(45)
    DEF NextSafeCallOnly, CallStart, SendMessage, EndSend, NetworkSend,
        NetworkReceive, ReceiveStatus, DeliverInitialMetadata,
        DeliverMessage, DeliverStatus, DeliverCancelled,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        HandPayloadToHost, HasFreeDeliverySlot,
        HasFreeDeliverySlotForTerminal, L0!IsActiveCall,
        L0!ActiveCallStates, L0!CallVars, L0!IsUnusedCall, L0!CallStates, TypeOK, L0!TypeOK
  <2>4. CASE NextSafeFfiOnly
    <3>1. UNCHANGED l0_vars
      BY <2>4, FfiOnlyStepsKeepL0
    <3>2. QED
      BY <1>1, <3>1, SMT DEF l0_vars, L0!vars, L0!CallVars, L0!IsUnusedCall, L0!CallStates, TypeOK, L0!TypeOK
  <2>5. CASE NextFail \/ NextExplicitStutter
    <3>1. UNCHANGED L0!CallVars
      BY <2>5, FailAndStutterStepsKeepCalls
    <3>2. QED BY <1>1, <3>1, SMT DEF L0!CallVars, L0!IsUnusedCall, L0!CallStates, TypeOK, L0!TypeOK
  <2>6. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, NextDecomposition
    DEF NextByFootprint, NextSafe, NextSafeRefining
<1>3. IsFreshBuffer(c, b)
    BY <1>1, <1>2, Zenon DEF UnusedCallsHaveFreshBuffers
\* Lending is the only step that writes a state onto a fresh buffer, and
\* it needs an active call.
<1>4. CASE \E c0 \in CallIds, b0 \in BufferIds, msg \in Messages, ch \in Sizes : 
              LendSendBuffer(c0, b0, msg, ch)
  <2>1. PICK d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>4
  <2>2. d # c
    BY <1>2, <2>1, SMT
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, L0!IsActiveCall, L0!ActiveCallStates,
        L0!IsUnusedCall
  <2>3. QED
    BY <1>1, <1>3, <2>1, <2>2, SMT
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer, TypeOK, L0!TypeOK
\* The other three writers move a buffer that is not fresh, so they
\* cannot be the one that named this buffer either.
<1>5. ASSUME \E d \in CallIds, e \in BufferIds,
                 s \in BufferStates :
                 /\ buffer_state[d][e] # "none"
                 /\ buffer_state' = [buffer_state EXCEPT ![d][e] = s]
      PROVE  (IsFreshBuffer(c, b))'
  <2>1. PICK d \in CallIds, e \in BufferIds, s \in BufferStates :
            /\ buffer_state[d][e] # "none"
            /\ buffer_state' = [buffer_state EXCEPT ![d][e] = s]
    BY <1>5
  <2>2. QED
    BY <1>1, <1>3, <2>1, SMT DEF IsFreshBuffer, TypeOK, L0!TypeOK
<1>51. CASE \E d0 \in CallIds, e0 \in BufferIds : 
               HostReturnsBuffer(d0, e0)
  <2>1. \E d \in CallIds, e \in BufferIds :
            /\ buffer_state[d][e] # "none"
            /\ \E s \in BufferStates :
                  buffer_state' = [buffer_state EXCEPT ![d][e] = s]
    BY <1>51, Zenon DEF HostReturnsBuffer, IsLentBuffer, BufferStates
  <2>2. QED BY <1>5, <2>1
<1>52. CASE \E d0 \in CallIds, e0 \in BufferIds : 
               FreeReturnedBuffer(d0, e0)
  <2>1. \E d \in CallIds, e \in BufferIds :
            /\ buffer_state[d][e] # "none"
            /\ \E s \in BufferStates :
                  buffer_state' = [buffer_state EXCEPT ![d][e] = s]
    BY <1>52, Zenon DEF FreeReturnedBuffer, IsReturnedBuffer, BufferStates
  <2>2. QED BY <1>5, <2>1
<1>53. CASE \E d0 \in CallIds, m0 \in Messages,
               bs \in BufferIds :
           SendMessage(d0, m0, bs)
  <2>1. \E d \in CallIds, e \in BufferIds :
            /\ buffer_state[d][e] # "none"
            /\ \E s \in BufferStates :
                  buffer_state' = [buffer_state EXCEPT ![d][e] = s]
    BY <1>53, Zenon DEF SendMessage, IsLentBuffer, BufferStates
  <2>2. QED BY <1>5, <2>1
\* Lending writes a fresh entry, and a fresh entry carries no send.
<1>6. CASE UNCHANGED buffer_state
    BY <1>3, <1>6, Zenon DEF IsFreshBuffer
<1>7. QED
    BY <1>1, <1>4, <1>51, <1>52, <1>53, <1>6,
       OnlyBufferStepsWriteBufferStates, Zenon

\* The send-to-buffer link, preserved by each of its writers.  Every case
\* is one citation of the writer lemma above plus arithmetic: reading one
\* entry of a nested function back after a write is done there, outside
\* this context, which is what makes these steps go through at all.
\* The last conjunct is where the new guard pays: FreeReturnedBuffer may
\* only fire on a buffer whose send is acquitted, so it can never be the
\* step that releases bytes the transport still needs.
\* Each accepted send lives in exactly one allocation, preserved.  Only a
\* commit touches either side, and it records the index the sequence is
\* about to reach: above every index already recorded, so it collides with
\* none, and it is the witness for the one index the sequence gains.
LEMMA NextPreservesSendBufferMatch ==
    TypeOK /\ BufferStateInv /\ Next =>
        (EverySendHasItsBuffer /\ SendsLiveInOneBuffer)'
<1>1. SUFFICES ASSUME TypeOK, BufferStateInv, Next
               PROVE  (EverySendHasItsBuffer /\ SendsLiveInOneBuffer)'
    OBVIOUS
<1>2. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY <1>1, TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
\* Every recorded index is already within the sequence, which is what makes
\* the new one collision-free.
<1>3. \A x \in CallIds, y \in BufferIds :
          buffer_send[x][y] <= Len(submitted[x])
    BY <1>1, Zenon DEF BufferStateInv, BufferSendIndicesExist
\* A lent buffer carries no send, so the one being committed is not already
\* the witness of an earlier index.
<1>4. \A x \in CallIds, y \in BufferIds :
          IsLentBuffer(x, y) => buffer_send[x][y] = 0
    BY <1>1, <1>2, SMT
    DEF BufferStateInv, CommittedBuffersAreGivenBack, IsLentBuffer
<1>5. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
               SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>5
  <2>2. IsLentBuffer(d, e)
    BY <2>1, Zenon DEF SendMessage
  <2>3. \A x \in CallIds, y \in BufferIds :
            /\ (buffer_send[x][y])' =
                   IF x = d /\ y = e THEN Len(submitted[d]) + 1
                                     ELSE buffer_send[x][y]
            /\ Len((submitted[x])') =
                   IF x = d THEN Len(submitted[d]) + 1 ELSE Len(submitted[x])
    BY <1>2, <2>1, SendMovesOneEntry
  <2>4. Len(submitted[d]) \in Nat
    BY <1>2, LenProperties
  <2>5. (EverySendHasItsBuffer)'
    <3>1. SUFFICES ASSUME NEW x \in CallIds,
                          NEW k \in 1..Len((submitted[x])')
                   PROVE  \E b \in BufferIds : (buffer_send[x][b])' = k
      BY Zenon DEF EverySendHasItsBuffer
    <3>2. CASE x = d /\ k = Len(submitted[d]) + 1
      BY <2>3, <3>2, Zenon
    <3>3. CASE ~(x = d /\ k = Len(submitted[d]) + 1)
      <4>1. k \in 1..Len(submitted[x])
        BY <2>3, <2>4, <3>1, <3>3, SMT
      <4>2. PICK b \in BufferIds : buffer_send[x][b] = k
        BY <1>1, <4>1, Zenon DEF BufferStateInv, EverySendHasItsBuffer
      <4>3. ~(x = d /\ b = e)
        BY <1>4, <2>2, <4>1, <4>2, SMT
      <4>4. QED BY <2>3, <4>2, <4>3, Zenon
    <3>4. QED BY <3>2, <3>3
  <2>6. (SendsLiveInOneBuffer)'
    <3>1. SUFFICES ASSUME NEW x \in CallIds, NEW b1 \in BufferIds,
                          NEW b2 \in BufferIds,
                          (buffer_send[x][b1])' # 0,
                          (buffer_send[x][b1])' = (buffer_send[x][b2])'
                   PROVE  b1 = b2
      BY Zenon DEF SendsLiveInOneBuffer
\* Another call is untouched, so its own instance of the invariant answers.
    <3>2. CASE x # d
      BY <1>1, <2>3, <3>1, <3>2
      DEF BufferStateInv, SendsLiveInOneBuffer
    <3>3. CASE x = d
\* Substituted once, so the arithmetic below reads one call's entries.
      <4>0. /\ (buffer_send[d][b1])' # 0
            /\ (buffer_send[d][b1])' = (buffer_send[d][b2])'
        BY <3>1, <3>3
      <4>1. \A y \in BufferIds :
                y # e => (buffer_send[d][y])' = buffer_send[d][y]
        BY <2>3, Zenon
      <4>2. (buffer_send[d][e])' = Len(submitted[d]) + 1
        BY <2>3, Zenon
\* The new index is above every index this call already records, so it
\* cannot equal one of them.
      <4>3. \A y \in BufferIds : buffer_send[d][y] <= Len(submitted[d])
        BY <1>3
      <4>4. CASE b1 = e /\ b2 # e
        <5>1. (buffer_send[d][b2])' = buffer_send[d][b2]
          BY <4>1, <4>4
        <5>2. buffer_send[d][b2] <= Len(submitted[d])
          BY <4>3
        <5>3. QED BY <2>4, <4>0, <4>2, <4>4, <5>1, <5>2, SMT
      <4>5. CASE b1 # e /\ b2 = e
        <5>1. (buffer_send[d][b1])' = buffer_send[d][b1]
          BY <4>1, <4>5
        <5>2. buffer_send[d][b1] <= Len(submitted[d])
          BY <4>3
        <5>3. QED BY <2>4, <4>0, <4>2, <4>5, <5>1, <5>2, SMT
      <4>6. CASE b1 # e /\ b2 # e
        <5>1. /\ (buffer_send[d][b1])' = buffer_send[d][b1]
              /\ (buffer_send[d][b2])' = buffer_send[d][b2]
          BY <4>1, <4>6
        <5>2. (/\ buffer_send[d][b1] # 0
               /\ buffer_send[d][b1] = buffer_send[d][b2]) => b1 = b2
          BY <1>1, Zenon DEF BufferStateInv, SendsLiveInOneBuffer
        <5>3. QED BY <4>0, <5>1, <5>2
      <4>7. QED BY <4>4, <4>5, <4>6
    <3>4. QED BY <3>2, <3>3
  <2>7. QED BY <2>5, <2>6
<1>6. CASE UNCHANGED buffer_send /\ UNCHANGED submitted
    BY <1>1, <1>6, Zenon
    DEF BufferStateInv, EverySendHasItsBuffer, SendsLiveInOneBuffer
<1>7. QED
    BY <1>1, <1>5, <1>6, OnlySendMessageWritesBufferSend,
       EveryStepEitherSubmitsOrKeepsSubmitted, Zenon

LEMMA NextPreservesBufferSendLink ==
    TypeOK /\ FfiCallInv /\ BufferStateInv /\ Next =>
        (/\ BufferSendIndicesExist
         /\ CommittedBuffersAreGivenBack
         /\ UnacquittedSendKeepsItsBytes)'
<1>1. SUFFICES ASSUME TypeOK, FfiCallInv, BufferStateInv, Next,
                      NEW c \in CallIds, NEW b \in BufferIds
               PROVE  /\ (buffer_send[c][b])' <= Len((submitted[c])')
                      /\ ((buffer_send[c][b])' # 0 =>
                            (buffer_state[c][b])' \in {"returned", "freed"})
                      /\ ((buffer_send[c][b])' > (write_dones_emitted[c])'
                            => (buffer_state[c][b])' = "returned")
    BY Zenon DEF BufferSendIndicesExist, CommittedBuffersAreGivenBack,
        UnacquittedSendKeepsItsBytes
<1>2. \A x \in CallIds, y \in BufferIds :
          /\ buffer_send[x][y] <= Len(submitted[x])
          /\ (buffer_send[x][y] # 0 => buffer_state[x][y] \in {"returned", "freed"})
          /\ (buffer_send[x][y] > write_dones_emitted[x] =>
                buffer_state[x][y] = "returned")
    BY <1>1, Zenon DEF BufferStateInv, BufferSendIndicesExist, CommittedBuffersAreGivenBack,
        UnacquittedSendKeepsItsBytes
<1>3. write_dones_emitted[c] <= Len(submitted[c])
    BY <1>1, Zenon DEF FfiCallInv, WriteDonesNeverExceedSends
<1>4. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY <1>1, TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
\* Lending writes a fresh entry, and a fresh entry carries no send, so it
\* cannot break either implication.
<1>5. CASE \E c0 \in CallIds, b0 \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(c0, b0, msg, ch)
  <2>1. PICK d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>5
  <2>2. /\ (buffer_state[c][b])' =
              IF c = d /\ b = e THEN "lent"
                                  ELSE buffer_state[c][b]
        /\ (buffer_send[c][b])' = buffer_send[c][b]
        /\ (submitted[c])' = submitted[c]
        /\ (write_dones_emitted[c])' = write_dones_emitted[c]
        /\ buffer_state[d][e] = "none"
    BY <1>4, <2>1, LendMovesOneEntry
  <2>25. /\ buffer_send[c][b] <= Len(submitted[c])
         /\ (buffer_send[c][b] # 0 =>
               buffer_state[c][b] \in {"returned", "freed"})
         /\ (buffer_send[c][b] > write_dones_emitted[c] =>
               buffer_state[c][b] = "returned")
    BY <1>1, <1>2
\* The entry being lent needs the invariant at its own index, not at the one
\* under proof: a fresh entry is neither returned nor freed, so it carries no
\* send, and that is what makes both implications survive the write.
  <2>26. buffer_send[d][e] # 0 =>
             buffer_state[d][e] \in {"returned", "freed"}
    BY <1>1, <1>2
  <2>3. buffer_send[d][e] = 0
    BY <2>2, <2>26, SMT
  <2>4. (buffer_send[c][b])' <= Len((submitted[c])')
    BY <2>2, <2>25
  <2>5. (buffer_send[c][b])' # 0 =>
            (buffer_state[c][b])' \in {"returned", "freed"}
    <3>1. CASE c = d /\ b = e
      BY <2>2, <2>3, <3>1, SMT
    <3>2. CASE ~(c = d /\ b = e)
      BY <2>2, <2>25, <3>2, SMT
    <3>3. QED BY <3>1, <3>2
  <2>6. (buffer_send[c][b])' > (write_dones_emitted[c])' =>
            (buffer_state[c][b])' = "returned"
    <3>1. CASE c = d /\ b = e
      BY <1>4, <2>2, <2>3, <3>1, SMT
    <3>2. CASE ~(c = d /\ b = e)
      BY <2>2, <2>25, <3>2, SMT
    <3>3. QED BY <3>1, <3>2
  <2>7. QED BY <2>4, <2>5, <2>6
\* Giving a buffer back moves the entry to returned, which satisfies both
\* implications whatever the index.
<1>6. CASE \E c0 \in CallIds, b0 \in BufferIds : HostReturnsBuffer(c0, b0)
  <2>1. PICK d \in CallIds, e \in BufferIds :
            HostReturnsBuffer(d, e)
    BY <1>6
  <2>2. /\ (buffer_state[c][b])' =
              IF c = d /\ b = e THEN "returned"
                                  ELSE buffer_state[c][b]
        /\ (buffer_send[c][b])' = buffer_send[c][b]
        /\ (submitted[c])' = submitted[c]
        /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>4, <2>1, ReturnMovesOneEntry
  <2>25. /\ buffer_send[c][b] <= Len(submitted[c])
         /\ (buffer_send[c][b] # 0 =>
               buffer_state[c][b] \in {"returned", "freed"})
         /\ (buffer_send[c][b] > write_dones_emitted[c] =>
               buffer_state[c][b] = "returned")
    BY <1>1, <1>2
  <2>3. QED
    BY <2>25, <2>2, SMT
\* Releasing the bytes: the guard says the send is acquitted, so the last
\* antecedent is false for the entry it touches.
<1>7. CASE \E c0 \in CallIds, b0 \in BufferIds : FreeReturnedBuffer(c0, b0)
  <2>1. PICK d \in CallIds, e \in BufferIds :
            FreeReturnedBuffer(d, e)
    BY <1>7
  <2>2. /\ (buffer_state[c][b])' =
              IF c = d /\ b = e THEN "freed"
                                  ELSE buffer_state[c][b]
        /\ (buffer_send[c][b])' = buffer_send[c][b]
        /\ (submitted[c])' = submitted[c]
        /\ (write_dones_emitted[c])' = write_dones_emitted[c]
        /\ buffer_send[d][e] <= write_dones_emitted[d]
    BY <1>4, <2>1, FreeMovesOneEntry
  <2>25. /\ buffer_send[c][b] <= Len(submitted[c])
         /\ (buffer_send[c][b] # 0 =>
               buffer_state[c][b] \in {"returned", "freed"})
         /\ (buffer_send[c][b] > write_dones_emitted[c] =>
               buffer_state[c][b] = "returned")
    BY <1>1, <1>2
\* One conjunct per step: asking one obligation for all three, over terms
\* that are conditionals, is what put this out of the solver's reach.
  <2>3. (buffer_send[c][b])' <= Len((submitted[c])')
    BY <2>2, <2>25
  <2>4. (buffer_send[c][b])' # 0 =>
            (buffer_state[c][b])' \in {"returned", "freed"}
    BY <2>2, <2>25, SMT
  <2>5. (buffer_send[c][b])' > (write_dones_emitted[c])' =>
            (buffer_state[c][b])' = "returned"
\* On the entry it touches the antecedent is false, because the guard says
\* that send is acquitted; anywhere else the entry has not moved.
    <3>0. buffer_send[d][e] \in Nat /\ write_dones_emitted[d] \in Nat
      BY <1>4, <2>1
    <3>1. CASE c = d /\ b = e
      BY <2>2, <3>0, <3>1, SMT
    <3>2. CASE ~(c = d /\ b = e)
      BY <2>2, <2>25, <3>2, SMT
    <3>3. QED BY <3>1, <3>2
  <2>6. QED BY <2>3, <2>4, <2>5
\* Committing records an index equal to the new length, and the entry it
\* records is returned by the same step.
<1>8. CASE \E d \in CallIds, m \in Messages,
               e \in BufferIds : SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>8
  <2>2. /\ (buffer_state[c][b])' =
              IF c = d /\ b = e THEN "returned"
                                  ELSE buffer_state[c][b]
        /\ (buffer_send[c][b])' =
              IF c = d /\ b = e THEN Len(submitted[d]) + 1
                                  ELSE buffer_send[c][b]
        /\ Len((submitted[c])') =
              IF c = d THEN Len(submitted[d]) + 1 ELSE Len(submitted[c])
        /\ (write_dones_emitted[c])' = write_dones_emitted[c]
    BY <1>4, <2>1, SendMovesOneEntry
  <2>25. /\ buffer_send[c][b] <= Len(submitted[c])
         /\ (buffer_send[c][b] # 0 =>
               buffer_state[c][b] \in {"returned", "freed"})
         /\ (buffer_send[c][b] > write_dones_emitted[c] =>
               buffer_state[c][b] = "returned")
    BY <1>1, <1>2
  <2>26. /\ Len(submitted[c]) \in Nat
         /\ Len(submitted[d]) \in Nat
         /\ buffer_send[c][b] \in Nat
    BY <1>1, <1>4, <2>1, LenProperties
  <2>3. (buffer_send[c][b])' <= Len((submitted[c])')
\* The recorded index is the new length, so the bound is tight on the entry
\* just committed and slack everywhere else.
    <3>1. CASE c = d
      BY <2>2, <2>25, <2>26, <3>1, SMT
    <3>2. CASE c # d
      BY <2>2, <2>25, <3>2, SMT
    <3>3. QED BY <3>1, <3>2
  <2>4. (buffer_send[c][b])' # 0 =>
            (buffer_state[c][b])' \in {"returned", "freed"}
    BY <2>2, <2>25, SMT
  <2>5. (buffer_send[c][b])' > (write_dones_emitted[c])' =>
            (buffer_state[c][b])' = "returned"
    BY <2>2, <2>25, SMT
  <2>6. QED BY <2>3, <2>4, <2>5
\* Emitting an acquittal only raises the count, which can only make the
\* last antecedent false.
<1>9. CASE \E d \in CallIds : EmitWriteDone(d)
  <2>1. PICK d \in CallIds : EmitWriteDone(d)
    BY <1>9
  <2>2. /\ (buffer_state[c][b])' = buffer_state[c][b]
        /\ (buffer_send[c][b])' = buffer_send[c][b]
        /\ (submitted[c])' = submitted[c]
        /\ (write_dones_emitted[c])' \in Nat
        /\ (write_dones_emitted[c])' >= write_dones_emitted[c]
    BY <1>4, <2>1, EmitMovesOneCount
  <2>25. /\ buffer_send[c][b] <= Len(submitted[c])
         /\ (buffer_send[c][b] # 0 =>
               buffer_state[c][b] \in {"returned", "freed"})
         /\ (buffer_send[c][b] > write_dones_emitted[c] =>
               buffer_state[c][b] = "returned")
    BY <1>1, <1>2
  <2>26. write_dones_emitted[c] \in Nat /\ buffer_send[c][b] \in Nat
    BY <1>1, <1>4
  <2>3. (buffer_send[c][b])' <= Len((submitted[c])')
    BY <2>2, <2>25
  <2>4. (buffer_send[c][b])' # 0 =>
            (buffer_state[c][b])' \in {"returned", "freed"}
    BY <2>2, <2>25
  <2>5. (buffer_send[c][b])' > (write_dones_emitted[c])' =>
            (buffer_state[c][b])' = "returned"
    BY <2>2, <2>25, <2>26, SMT
  <2>6. QED BY <2>3, <2>4, <2>5
<1>10. CASE UNCHANGED <<buffer_send, buffer_state, submitted,
                        write_dones_emitted>>
    BY <1>2, <1>10, Zenon
<1>11. QED
    BY <1>1, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10,
       OnlyBufferStepsWriteBufferStates, OnlySendMessageWritesBufferSend,
       EveryStepEitherEmitsOrKeepsWriteDones,
       EveryStepEitherSubmitsOrKeepsSubmitted, Zenon

LEMMA NextPreservesLentCountBridge ==
    TypeOK /\ LentCountMatchesBufferStates /\ Next =>
        LentCountMatchesBufferStates'
<1>1. ASSUME TypeOK, LentCountMatchesBufferStates, Next
      PROVE  LentCountMatchesBufferStates'
  <2>0. buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
    BY <1>1, TypeOKSplit DEF BufferTypes
  <2>00. \A c \in CallIds : IsFiniteSet({x \in BufferIds : buffer_state[c][x] = "lent"})
    BY BufferIdsAreAFiniteNonemptySet, FS_Subset
  <2>01. \A c \in CallIds :
             buffers_held_by_host[c] = Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"})
    BY <1>1, Zenon DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>02. buffers_held_by_host \in [CallIds -> Nat]
    BY <1>1, Zenon DEF TypeOK, L0!TypeOK
  <2>1. CASE \E c0 \in CallIds, b0 \in BufferIds, msg \in Messages, ch \in Sizes : 
                LendSendBuffer(c0, b0, msg, ch)
    <3>1. PICK c \in CallIds, b \in BufferIds :
              /\ buffer_state' = [buffer_state EXCEPT ![c][b] = "lent"]
              /\ buffer_state[c][b] # "lent"
              /\ buffers_held_by_host' =
                     [buffers_held_by_host EXCEPT ![c] =
                          buffers_held_by_host[c] + 1]
      BY <2>1, ZenonT(120) DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer
    <3>2. /\ {x \in BufferIds : buffer_state'[c][x] = "lent"} = {x \in BufferIds : buffer_state[c][x] = "lent"} \cup {b}
          /\ b \notin {x \in BufferIds : buffer_state[c][x] = "lent"}
          /\ \A d \in CallIds : d # c =>
                 {x \in BufferIds : buffer_state'[d][x] = "lent"} = {x \in BufferIds : buffer_state[d][x] = "lent"}
      BY <2>0, <3>1, LentSetGainsTheLentOne DEF BufferStates
    <3>3. IsFiniteSet({x \in BufferIds : buffer_state[c][x] = "lent"})
      BY <2>00
    <3>4. Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"} \cup {b}) =
              Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"}) + 1
      BY <3>2, <3>3, FS_AddElement
    <3>5. ASSUME NEW d \in CallIds
          PROVE  buffers_held_by_host'[d] = Cardinality({x \in BufferIds : buffer_state'[d][x] = "lent"})
      <4>1. CASE d = c
        <5>1. buffers_held_by_host'[c] = buffers_held_by_host[c] + 1
          BY <2>02, <3>1, Zenon
        <5>2. buffers_held_by_host[c] = Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"})
          BY <2>01
        <5>25. {x \in BufferIds : buffer_state'[d][x] = "lent"} =
                   {x \in BufferIds : buffer_state'[c][x] = "lent"}
          BY <4>1
        <5>3. QED
          BY <3>2, <3>4, <4>1, <5>1, <5>2, <5>25, Zenon
      <4>2. CASE d # c
        <5>1. buffers_held_by_host'[d] = buffers_held_by_host[d]
          BY <2>02, <3>1, <4>2, Zenon
        <5>2. QED
          BY <2>01, <3>2, <4>2, <5>1, Zenon
      <4>3. QED BY <4>1, <4>2
    <3>6. QED
      BY <3>5, Zenon DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>2. CASE \E c0 \in CallIds, b0 \in BufferIds : 
                HostReturnsBuffer(c0, b0)
    <3>1. PICK c \in CallIds, b \in BufferIds :
              /\ buffer_state' = [buffer_state EXCEPT ![c][b] = "returned"]
              /\ buffer_state[c][b] = "lent"
              /\ buffers_held_by_host' =
                     [buffers_held_by_host EXCEPT ![c] =
                          buffers_held_by_host[c] - 1]
      BY <2>2, Zenon DEF HostReturnsBuffer, IsLentBuffer
    <3>2. /\ {x \in BufferIds : buffer_state'[c][x] = "lent"} = {x \in BufferIds : buffer_state[c][x] = "lent"} \ {b}
          /\ b \in {x \in BufferIds : buffer_state[c][x] = "lent"}
          /\ \A d \in CallIds : d # c =>
                 {x \in BufferIds : buffer_state'[d][x] = "lent"} = {x \in BufferIds : buffer_state[d][x] = "lent"}
      BY <2>0, <3>1, LentSetLosesTheGivenBack DEF BufferStates
    <3>3. IsFiniteSet({x \in BufferIds : buffer_state[c][x] = "lent"})
      BY <2>00
    <3>4. Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"} \ {b}) =
              Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"}) - 1
      BY <3>2, <3>3, FS_RemoveElement
    <3>5. ASSUME NEW d \in CallIds
          PROVE  buffers_held_by_host'[d] = Cardinality({x \in BufferIds : buffer_state'[d][x] = "lent"})
      <4>1. CASE d = c
        <5>1. buffers_held_by_host'[c] = buffers_held_by_host[c] - 1
          BY <2>02, <3>1, Zenon
        <5>2. buffers_held_by_host[c] = Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"})
          BY <2>01
        <5>25. {x \in BufferIds : buffer_state'[d][x] = "lent"} =
                   {x \in BufferIds : buffer_state'[c][x] = "lent"}
          BY <4>1
        <5>3. QED
          BY <3>2, <3>4, <4>1, <5>1, <5>2, <5>25, Zenon
      <4>2. CASE d # c
        <5>1. buffers_held_by_host'[d] = buffers_held_by_host[d]
          BY <2>02, <3>1, <4>2, Zenon
        <5>2. QED
          BY <2>01, <3>2, <4>2, <5>1, Zenon
      <4>3. QED BY <4>1, <4>2
    <3>6. QED
      BY <3>5, Zenon DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>3. CASE \E c0 \in CallIds, m0 \in Messages,
                bs \in BufferIds :
             SendMessage(c0, m0, bs)
    <3>1. PICK c \in CallIds, b \in BufferIds :
              /\ buffer_state' = [buffer_state EXCEPT ![c][b] = "returned"]
              /\ buffer_state[c][b] = "lent"
              /\ buffers_held_by_host' =
                     [buffers_held_by_host EXCEPT ![c] =
                          buffers_held_by_host[c] - 1]
      BY <2>3, Zenon DEF SendMessage, IsLentBuffer
    <3>2. /\ {x \in BufferIds : buffer_state'[c][x] = "lent"} = {x \in BufferIds : buffer_state[c][x] = "lent"} \ {b}
          /\ b \in {x \in BufferIds : buffer_state[c][x] = "lent"}
          /\ \A d \in CallIds : d # c =>
                 {x \in BufferIds : buffer_state'[d][x] = "lent"} = {x \in BufferIds : buffer_state[d][x] = "lent"}
      BY <2>0, <3>1, LentSetLosesTheGivenBack DEF BufferStates
    <3>3. IsFiniteSet({x \in BufferIds : buffer_state[c][x] = "lent"})
      BY <2>00
    <3>4. Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"} \ {b}) =
              Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"}) - 1
      BY <3>2, <3>3, FS_RemoveElement
    <3>5. ASSUME NEW d \in CallIds
          PROVE  buffers_held_by_host'[d] = Cardinality({x \in BufferIds : buffer_state'[d][x] = "lent"})
      <4>1. CASE d = c
        <5>1. buffers_held_by_host'[c] = buffers_held_by_host[c] - 1
          BY <2>02, <3>1, Zenon
        <5>2. buffers_held_by_host[c] = Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"})
          BY <2>01
        <5>25. {x \in BufferIds : buffer_state'[d][x] = "lent"} =
                   {x \in BufferIds : buffer_state'[c][x] = "lent"}
          BY <4>1
        <5>3. QED
          BY <3>2, <3>4, <4>1, <5>1, <5>2, <5>25, Zenon
      <4>2. CASE d # c
        <5>1. buffers_held_by_host'[d] = buffers_held_by_host[d]
          BY <2>02, <3>1, <4>2, Zenon
        <5>2. QED
          BY <2>01, <3>2, <4>2, <5>1, Zenon
      <4>3. QED BY <4>1, <4>2
    <3>6. QED
      BY <3>5, Zenon DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>4. CASE \E c0 \in CallIds, b0 \in BufferIds : 
                FreeReturnedBuffer(c0, b0)
    <3>1. PICK c \in CallIds, b \in BufferIds :
              /\ buffer_state' = [buffer_state EXCEPT ![c][b] = "freed"]
              /\ buffer_state[c][b] = "returned"
              /\ UNCHANGED buffers_held_by_host
      BY <2>4, Zenon DEF FreeReturnedBuffer, IsReturnedBuffer
    <3>2. \A d \in CallIds :
              {x \in BufferIds : buffer_state'[d][x] = "lent"} = {x \in BufferIds : buffer_state[d][x] = "lent"}
      BY <2>0, <3>1, LentSetsUnmovedOffTheLentState DEF BufferStates
    <3>3. QED
      BY <2>01, <3>1, <3>2, Zenon
      DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>5. CASE UNCHANGED buffer_state /\ UNCHANGED buffers_held_by_host
    BY <1>1, <2>5, Zenon DEF LentCountMatchesBufferStates, IsLentBuffer
  <2>6. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5,
       OnlyBufferStepsWriteBufferStates, EveryStepEitherLendsOrKeepsBuffers,
       Zenon
<1>2. QED BY <1>1




\* A call on a channel of a drained runtime is not running: the level-0
\* invariant puts an active call on an open or closing channel, and a drained
\* runtime has neither.
LEMMA DrainedRuntimeCallNotActive ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds,
           TypeOK, L0!StrongInv, IsRuntimeDrained(rtId),
           call_channel[cId] \in L0!ChannelsOf(rtId)
    PROVE  ~L0!IsActiveCall(cId)
<1>1. SUFFICES ASSUME L0!IsActiveCall(cId)
               PROVE  FALSE
    OBVIOUS
\* The invariant quantifies over the used calls, so membership comes first.
<1>2. cId \in L0!UsedCalls
    BY <1>1, SMT
    DEF L0!UsedCalls, L0!IsUnusedCall, L0!IsActiveCall,
        L0!ActiveCallStates, TypeOK, L0!TypeOK, L0!CallStates
<1>3. call_channel[cId] \in L0!ActiveChannels
    BY <1>1, <1>2, Zenon
    DEF L0!StrongInv, L0!StructuralInv, L0!CallLifecycleInv
<1>4. IsClosedChannel(call_channel[cId])
    BY Zenon DEF IsRuntimeDrained
<1>5. QED
    BY <1>3, <1>4, SMT
    DEF L0!ActiveChannels, L0!ActiveChannelStates, IsClosedChannel,
        TypeOK, L0!TypeOK

\* Only starting a call names a channel.  The refining family is split by
\* footprint: five groups, and only the one CallStart lives in can write a
\* call's channel.
LEMMA OnlyCallStartWritesCallChannel ==
    ASSUME [Next]_vars
    PROVE  \/ \E c \in CallIds, ch \in ChannelIds :
                 CallStart(c, ch)
           \/ UNCHANGED call_channel
<1>1. CASE NextSafeRuntimeOnly \/ NextSafeRuntimeChannel
    BY <1>1, SMT
    DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
        L0!RuntimeCreate, L0!RuntimeRelease, L0!RuntimeBeginShutdown,
        RequestCancellationOfActiveCalls,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>2. CASE NextSafeChannelOnly \/ NextSafeChannelCall
    BY <1>2, SMT
    DEF NextSafeChannelOnly, NextSafeChannelCall,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        RequestCancellationOfActiveCalls,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>3. CASE NextSafeCallOnly
    BY <1>3, SMT
    DEF NextSafeCallOnly, CallStart, SendMessage, EndSend, NetworkSend,
        NetworkReceive, ReceiveStatus, DeliverInitialMetadata,
        DeliverMessage, DeliverStatus, DeliverCancelled, HandPayloadToHost,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>4. CASE NextSafeFfiOnly
    BY <1>4, FfiOnlyStutters, SMT DEF l0_vars, L0!vars, L0!CallVars
<1>5. CASE NextFail \/ NextExplicitStutter
    BY <1>5, SMT
    DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
        RemainReleased, L0!RuntimeFail, L0!RemainFailed, L0!RemainReleased,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
<1>6. CASE UNCHANGED vars
    BY <1>6, SMT DEF vars, l0_vars, L0!vars, L0!CallVars
<1>7. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, NextDecomposition
    DEF NextByFootprint, NextSafe, NextSafeRefining

\* Released is where a runtime ends.  Read at level 0 for the same reason as
\* the terminal call: the level-0 machinery is the only writer of a runtime
\* state, and none of its writers names RELEASED as the state it leaves.
LEMMA ReleasedRuntimeStaysReleased ==
    ASSUME NEW rtId \in RuntimeIds, TypeOK, [Next]_vars,
           IsReleasedRuntime(rtId)
    PROVE  (IsReleasedRuntime(rtId))'
<1>0. runtime_state \in [RuntimeIds -> L0!RuntimeStates]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>1. [L0!Next]_l0_vars
    BY RefinesNext
<1>2. CASE L0!Next
    BY <1>0, <1>2, SMTT(60)
    DEF L0!Next, L0!RuntimeCreate, L0!RuntimeBeginShutdown,
        L0!RuntimeRelease, L0!RuntimeFail, L0!RemainFailed,
        L0!RemainReleased, L0!ChannelCreate, L0!ChannelStartClosing,
        L0!ChannelFinishClosing, L0!CallStart, L0!SendMessage, L0!EndSend,
        L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
        L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
        L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        IsReleasedRuntime, L0!RuntimeStates
<1>3. CASE UNCHANGED l0_vars
    BY <1>0, <1>3, SMT
    DEF l0_vars, L0!vars, L0!RuntimeVars, IsReleasedRuntime
<1>4. QED BY <1>1, <1>2, <1>3

\* The tag is frozen once the event is out: only EmitShutdownComplete writes
\* it, and that step demands an event not yet emitted.
LEMMA EmittedRuntimeTagFrozen ==
    ASSUME NEW rtId \in RuntimeIds, TypeOK, [Next]_vars,
           IsShutdownEventEmitted(rtId)
    PROVE  /\ (IsShutdownEventEmitted(rtId))'
           /\ (SecondEventOwed(rtId))' =
                  SecondEventOwed(rtId)
<1>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
    BY <1>1, SMT DEF EmitShutdownComplete, TypeOK, L0!TypeOK
<1>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
    BY <1>2, SMT DEF ShutdownCallbackReturns, TypeOK, L0!TypeOK
<1>3. CASE UNCHANGED <<shutdown_event_emitted, shutdown_callback_running,
                       second_event_owed>>
    BY <1>3, SMT
<1>4. QED
    BY <1>1, <1>2, <1>3, OnlyShutdownStepsWriteShutdownFlags, Zenon

\* The second event is a latch: only its own step writes the flag, and that
\* step raises it.
LEMMA ResourcesEmittedStable ==
    ASSUME NEW rtId \in RuntimeIds, TypeOK, [Next]_vars,
           IsResourcesReleasedEmitted(rtId)
    PROVE  (IsResourcesReleasedEmitted(rtId))'
<1>1. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    BY <1>1, SMT DEF EmitResourcesReleased, TypeOK, L0!TypeOK
<1>2. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    BY <1>2, SMT DEF ResourcesReleasedCallbackReturns, TypeOK, L0!TypeOK
<1>3. CASE UNCHANGED <<resources_released_emitted,
                       resources_released_callback_running>>
    BY <1>3, SMT
<1>4. QED
    BY <1>1, <1>2, <1>3, OnlyReleaseStepsWriteReleaseFlags, Zenon

\* Once the shutdown event is out, the runtime's ledger can only fall.  The
\* event is emitted from a drained runtime, whose calls are all terminal, and
\* the two steps that raise what the host owes - a delivery and a lend - both
\* need an active call.  Nothing joins the runtime either: naming a channel
\* needs its runtime RUNNING, and this one is past that.
LEMMA EmittedRuntimeStaysReclaimable ==
    ASSUME StrongInv, Next, NEW rtId \in RuntimeIds,
           IsShutdownEventEmitted(rtId), NoHostDebt(rtId)
    PROVE  (NoHostDebt(rtId))'
<1>1. TypeOK /\ L0!StrongInv
    BY Zenon DEF StrongInv
<1>2. IsRuntimeDrained(rtId)
    BY Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
<1>3. \A c \in CallIds :
          call_channel[c] \in L0!ChannelsOf(rtId) => ~L0!IsActiveCall(c)
    BY <1>1, <1>2, DrainedRuntimeCallNotActive, Zenon
<1>4. runtime_state[rtId] # "RUNNING"
    BY Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore,
        IsStoppingRuntime, IsReleasedRuntime
\* Ownership is written once, at creation, and creating a channel here would
\* need this runtime RUNNING, so the runtime's channel set cannot grow.
<1>5. (L0!ChannelsOf(rtId))' \subseteq L0!ChannelsOf(rtId)
  <2>1. CASE UNCHANGED channel_runtime
    BY <2>1, Zenon DEF L0!ChannelsOf
  <2>2. CASE \E ch \in ChannelIds, rt \in RuntimeIds : ChannelCreate(ch, rt)
    BY <1>1, <1>4, <2>2, SMT
    DEF ChannelCreate, L0!ChannelCreate, L0!ChannelsOf, TypeOK, L0!TypeOK
  <2>3. QED
    BY <2>1, <2>2, OnlyChannelCreateWritesOwnership, Zenon
\* And starting a call cannot name one of them, for the same reason.
<1>6. \A c \in CallIds :
          (call_channel[c])' \in (L0!ChannelsOf(rtId))' =>
              call_channel[c] \in L0!ChannelsOf(rtId)
  <2>1. CASE UNCHANGED call_channel
    BY <1>5, <2>1, Zenon
  <2>2. CASE \E c2 \in CallIds, ch \in ChannelIds : CallStart(c2, ch)
    BY <1>1, <1>4, <1>5, <2>2, SMT
    DEF CallStart, L0!CallStart, L0!ChannelsOf, TypeOK, L0!TypeOK
  <2>3. QED
    BY <2>1, <2>2, OnlyCallStartWritesCallChannel, Zenon
<1>7. QED
    BY <1>1, <1>3, <1>6, QuietRuntimeStaysReclaimable, Zenon

\* The counter reads the states in the direction NoLentMeansNoneHeld does not:
\* a zero count is an empty set of lent buffers.
LEMMA NoneHeldMeansNoLent ==
    ASSUME TypeOK, LentCountMatchesBufferStates, NEW cId \in CallIds,
           HostHoldsNoBuffer(cId), NEW b \in BufferIds
    PROVE  ~IsLentBuffer(cId, b)
<1>1. Cardinality({e \in BufferIds : IsLentBuffer(cId, e)}) = 0
    BY Zenon DEF LentCountMatchesBufferStates, HostHoldsNoBuffer
<1>2. IsFiniteSet({e \in BufferIds : IsLentBuffer(cId, e)})
    BY BufferIdsAreAFiniteNonemptySet, FS_Subset, Zenon
    DEF BufferIdsAreAFiniteNonemptySet
<1>3. {e \in BufferIds : IsLentBuffer(cId, e)} = {}
    BY <1>1, <1>2, FS_EmptySet, Zenon
<1>4. QED
    BY <1>3, Zenon

\* And the runtime acquires no new returned buffer either.  The three steps that
\* write "returned" are a lend it cannot make, a give-back of a buffer it has not
\* lent, and a commit that needs an active call - so the state a runtime owes
\* itself only shrinks once its calls are quiet.
LEMMA QuietRuntimeKeepsNoReturnedBytes ==
    ASSUME TypeOK, LentCountMatchesBufferStates, Next,
           NEW rtId \in RuntimeIds,
           NoHostDebt(rtId), RuntimeHoldsNoReturnedBytes(rtId),
           \A c \in CallIds :
               call_channel[c] \in L0!ChannelsOf(rtId) => ~L0!IsActiveCall(c),
           \A c \in CallIds :
               (call_channel[c])' \in (L0!ChannelsOf(rtId))' =>
                   call_channel[c] \in L0!ChannelsOf(rtId)
    PROVE  (RuntimeHoldsNoReturnedBytes(rtId))'
<1>1. SUFFICES ASSUME NEW c \in CallIds, NEW b \in BufferIds,
                      (call_channel[c])' \in (L0!ChannelsOf(rtId))'
               PROVE  ~(IsReturnedBuffer(c, b))'
    BY Zenon DEF RuntimeHoldsNoReturnedBytes
<1>2. /\ call_channel[c] \in L0!ChannelsOf(rtId)
      /\ ~L0!IsActiveCall(c)
      /\ HostHoldsNoBuffer(c)
      /\ ~IsReturnedBuffer(c, b)
    BY <1>1, Zenon DEF NoHostDebt, RuntimeHoldsNoReturnedBytes
<1>3. \A e \in BufferIds : ~IsLentBuffer(c, e)
    BY <1>2, NoneHeldMeansNoLent, Zenon
<1>4. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>2, <1>4, SMT
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsReturnedBuffer, L0!IsActiveCall,
        L0!ActiveCallStates, TypeOK, L0!TypeOK
<1>5. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <1>2, <1>3, <1>5, SMT
    DEF HostReturnsBuffer, IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
<1>6. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
              SendMessage(d, m, e)
    BY <1>2, <1>3, <1>6, SMT
    DEF SendMessage, L0!SendMessage, IsReturnedBuffer, IsLentBuffer,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
<1>7. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <1>2, <1>7, SMT
    DEF FreeReturnedBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
<1>8. CASE UNCHANGED buffer_state
    BY <1>2, <1>8, Zenon DEF IsReturnedBuffer
<1>9. QED
    BY <1>4, <1>5, <1>6, <1>7, <1>8, OnlyBufferStepsWriteBufferStates, Zenon

\* Both halves of what an emitted runtime owes nobody, in one citable fact.
LEMMA EmittedRuntimeStaysDebtFree ==
    ASSUME StrongInv, Next, NEW rtId \in RuntimeIds,
           IsShutdownEventEmitted(rtId),
           NoHostDebt(rtId), RuntimeHoldsNoReturnedBytes(rtId)
    PROVE  /\ (NoHostDebt(rtId))'
           /\ (RuntimeHoldsNoReturnedBytes(rtId))'
<1>1. TypeOK /\ L0!StrongInv /\ LentCountMatchesBufferStates
    BY Zenon DEF StrongInv, BufferStateInv
<1>2. (NoHostDebt(rtId))'
    BY EmittedRuntimeStaysReclaimable
<1>3. IsRuntimeDrained(rtId)
    BY Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
<1>4. \A c \in CallIds :
          call_channel[c] \in L0!ChannelsOf(rtId) => ~L0!IsActiveCall(c)
    BY <1>1, <1>3, DrainedRuntimeCallNotActive, Zenon
<1>5. runtime_state[rtId] # "RUNNING"
    BY Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore,
        IsStoppingRuntime, IsReleasedRuntime
<1>6. (L0!ChannelsOf(rtId))' \subseteq L0!ChannelsOf(rtId)
  <2>1. CASE UNCHANGED channel_runtime
    BY <2>1, Zenon DEF L0!ChannelsOf
  <2>2. CASE \E ch \in ChannelIds, rt \in RuntimeIds : ChannelCreate(ch, rt)
    BY <1>1, <1>5, <2>2, SMT
    DEF ChannelCreate, L0!ChannelCreate, L0!ChannelsOf, TypeOK, L0!TypeOK
  <2>3. QED
    BY <1>1, <2>1, <2>2, OnlyChannelCreateWritesOwnership, Zenon
<1>7. \A c \in CallIds :
          (call_channel[c])' \in (L0!ChannelsOf(rtId))' =>
              call_channel[c] \in L0!ChannelsOf(rtId)
  <2>1. CASE UNCHANGED call_channel
    BY <1>6, <2>1, Zenon
  <2>2. CASE \E c2 \in CallIds, ch \in ChannelIds : CallStart(c2, ch)
    BY <1>1, <1>5, <1>6, <2>2, SMT
    DEF CallStart, L0!CallStart, L0!ChannelsOf, TypeOK, L0!TypeOK
  <2>3. QED
    BY <1>1, <2>1, <2>2, OnlyCallStartWritesCallChannel, Zenon
<1>8. QED
    BY <1>1, <1>2, <1>4, <1>7, QuietRuntimeKeepsNoReturnedBytes, Zenon

\* The release signal, conjunct by conjunct.  The first two are framing: only
\* the two release steps write those flags, and the step that raises the
\* emitted flag demands the shutdown tag it points at.  The third is the tag's
\* accuracy, and it is the one that needs the ledger not to grow.
LEMMA NextPreservesReleaseSignal ==
    ASSUME StrongInv, Next
    PROVE  ReleaseSignalInv'
<1>1. SUFFICES ASSUME NEW rtId \in RuntimeIds
               PROVE  /\ (IsResourcesReleasedCallbackRunning(rtId))' =>
                          (IsResourcesReleasedEmitted(rtId))'
                      /\ (IsResourcesReleasedEmitted(rtId))' =>
                          /\ (IsShutdownEventEmitted(rtId))'
                          /\ (SecondEventOwed(rtId))'
                      /\ ((IsShutdownEventEmitted(rtId))' /\
                          ~(SecondEventOwed(rtId))') =>
                              (NoHostDebt(rtId))'
                      /\ (IsResourcesReleasedEmitted(rtId))' =>
                          /\ (NoHostDebt(rtId))'
                          /\ (RuntimeHoldsNoReturnedBytes(rtId))'
    BY Zenon DEF ReleaseSignalInv
<1>2. TypeOK
    BY Zenon DEF StrongInv
\* The running flag never outlives the emitted one: the step that raises it
\* raises both, and the step that lowers it leaves the other alone.
<1>3. (IsResourcesReleasedCallbackRunning(rtId))' =>
          (IsResourcesReleasedEmitted(rtId))'
  <2>0. IsResourcesReleasedCallbackRunning(rtId) =>
            IsResourcesReleasedEmitted(rtId)
    BY Zenon DEF StrongInv, ShutdownSignalInv, ReleaseSignalInv
  <2>1. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    BY <1>2, <2>0, <2>1, SMT
    DEF EmitResourcesReleased, IsResourcesReleasedEmitted,
        IsResourcesReleasedCallbackRunning, TypeOK, L0!TypeOK
  <2>2. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    BY <1>2, <2>0, <2>2, SMT
    DEF ResourcesReleasedCallbackReturns, IsResourcesReleasedEmitted,
        IsResourcesReleasedCallbackRunning, TypeOK, L0!TypeOK
  <2>3. CASE UNCHANGED <<resources_released_emitted,
                         resources_released_callback_running>>
    BY <2>0, <2>3, SMT
    DEF IsResourcesReleasedEmitted, IsResourcesReleasedCallbackRunning
  <2>4. QED
    BY <2>1, <2>2, <2>3, OnlyReleaseStepsWriteReleaseFlags, Zenon
\* The second event is owed before it is sent: its step demands both shutdown
\* flags, and once the event is out neither can move again.
<1>4. (IsResourcesReleasedEmitted(rtId))' =>
          /\ (IsShutdownEventEmitted(rtId))'
          /\ (SecondEventOwed(rtId))'
  <2>0. SUFFICES ASSUME (IsResourcesReleasedEmitted(rtId))'
                 PROVE  /\ (IsShutdownEventEmitted(rtId))'
                        /\ (SecondEventOwed(rtId))'
    OBVIOUS
  <2>1. ASSUME EmitResourcesReleased(rtId)
        PROVE  /\ (IsShutdownEventEmitted(rtId))'
               /\ (SecondEventOwed(rtId))'
    BY <1>2, <2>1, SMT
    DEF EmitResourcesReleased, IsShutdownEventEmitted,
        SecondEventOwed, TypeOK, L0!TypeOK
\* Not this step's flag, so the event was already out - and an emitted runtime
\* cannot emit again, which is what freezes its tag.
  <2>2. ASSUME IsResourcesReleasedEmitted(rtId)
        PROVE  /\ (IsShutdownEventEmitted(rtId))'
               /\ (SecondEventOwed(rtId))'
    <3>0. IsShutdownEventEmitted(rtId) /\ SecondEventOwed(rtId)
      BY <2>2, Zenon
      DEF StrongInv, ShutdownSignalInv, ReleaseSignalInv
    <3>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
      BY <1>2, <3>0, <3>1, SMT
      DEF EmitShutdownComplete, IsShutdownEventEmitted,
          SecondEventOwed, TypeOK, L0!TypeOK
    <3>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
      BY <1>2, <3>0, <3>2, SMT
      DEF ShutdownCallbackReturns, IsShutdownEventEmitted,
          SecondEventOwed, TypeOK, L0!TypeOK
    <3>3. CASE UNCHANGED <<shutdown_event_emitted,
                           shutdown_callback_running,
                           second_event_owed>>
      BY <3>0, <3>3, SMT
      DEF IsShutdownEventEmitted, SecondEventOwed
    <3>4. QED
      BY <3>1, <3>2, <3>3, OnlyShutdownStepsWriteShutdownFlags, Zenon
  <2>3. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    <3>1. PICK rt \in RuntimeIds : EmitResourcesReleased(rt)
      BY <2>3
    <3>2. CASE rt = rtId
      BY <2>1, <3>1, <3>2
    <3>3. CASE rt # rtId
      <4>1. resources_released_emitted'[rtId] =
                resources_released_emitted[rtId]
        BY <1>2, <3>1, <3>3, SMT
        DEF EmitResourcesReleased, TypeOK, L0!TypeOK
      <4>2. IsResourcesReleasedEmitted(rtId)
        BY <2>0, <4>1, Zenon DEF IsResourcesReleasedEmitted
      <4>3. QED
        BY <2>2, <4>2
    <3>4. QED
      BY <3>2, <3>3
  <2>4. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    <3>1. resources_released_emitted' = resources_released_emitted
      BY <2>4, SMT DEF ResourcesReleasedCallbackReturns
    <3>2. IsResourcesReleasedEmitted(rtId)
      BY <2>0, <3>1, Zenon DEF IsResourcesReleasedEmitted
    <3>3. QED
      BY <2>2, <3>2
  <2>5. CASE UNCHANGED <<resources_released_emitted,
                         resources_released_callback_running>>
    <3>1. resources_released_emitted' = resources_released_emitted
      BY <2>5, SMT
    <3>2. IsResourcesReleasedEmitted(rtId)
      BY <2>0, <3>1, Zenon DEF IsResourcesReleasedEmitted
    <3>3. QED
      BY <2>2, <3>2
  <2>6. QED
    BY <2>3, <2>4, <2>5, OnlyReleaseStepsWriteReleaseFlags, Zenon
\* The tag is accurate.  Where the step sets it, it reads the ledger it reports
\* and moves nothing the ledger is made of; everywhere else the tag is frozen,
\* so the invariant already says the ledger was empty, and an emitted runtime
\* cannot be handed a debt back.
<1>5. ((IsShutdownEventEmitted(rtId))' /\
       ~(SecondEventOwed(rtId))') => (NoHostDebt(rtId))'
  <2>0. SUFFICES ASSUME (IsShutdownEventEmitted(rtId))',
                        ~(SecondEventOwed(rtId))'
                 PROVE  (NoHostDebt(rtId))'
    OBVIOUS
  <2>1. ASSUME EmitShutdownComplete(rtId)
        PROVE  (NoHostDebt(rtId))'
    <3>1. NoHostDebt(rtId)
      BY <1>2, <2>0, <2>1, SMT
      DEF EmitShutdownComplete, SecondEventOwed, TypeOK, L0!TypeOK
    <3>2. QED
      BY <1>2, <2>1, <3>1, SMT
      DEF EmitShutdownComplete, NoHostDebt, HostOwnsNoPayload,
          HostHoldsNoBuffer, OwedPayloads, L0!ChannelsOf, l0_vars, L0!vars,
          L0!RuntimeVars, L0!ChannelVars, L0!CallVars, TypeOK, L0!TypeOK
\* Anything else leaves this runtime's two flags where they were, so the
\* invariant hands over an empty ledger and the runtime keeps it empty.
  <2>2. ASSUME shutdown_event_emitted'[rtId] = shutdown_event_emitted[rtId],
               second_event_owed'[rtId] =
                   second_event_owed[rtId]
        PROVE  (NoHostDebt(rtId))'
    <3>1. IsShutdownEventEmitted(rtId) /\ ~SecondEventOwed(rtId)
      BY <2>0, <2>2, Zenon
      DEF IsShutdownEventEmitted, SecondEventOwed
    <3>2. NoHostDebt(rtId)
      BY <3>1, Zenon
      DEF StrongInv, ShutdownSignalInv, ReleaseSignalInv
    <3>3. QED
      BY <3>1, <3>2, EmittedRuntimeStaysReclaimable, Zenon
  <2>3. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
    <3>1. PICK rt \in RuntimeIds : EmitShutdownComplete(rt)
      BY <2>3
    <3>2. CASE rt = rtId
      BY <2>1, <3>1, <3>2
\* Another runtime's event moves only that runtime's entries.
    <3>3. CASE rt # rtId
      <4>1. /\ shutdown_event_emitted'[rtId] = shutdown_event_emitted[rtId]
            /\ second_event_owed'[rtId] =
                   second_event_owed[rtId]
        BY <1>2, <3>1, <3>3, SMT
        DEF EmitShutdownComplete, TypeOK, L0!TypeOK
      <4>2. QED
        BY <2>2, <4>1
    <3>4. QED
      BY <3>2, <3>3
  <2>4. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
    <3>1. /\ shutdown_event_emitted'[rtId] = shutdown_event_emitted[rtId]
          /\ second_event_owed'[rtId] =
                 second_event_owed[rtId]
      BY <2>4, SMT DEF ShutdownCallbackReturns
    <3>2. QED
      BY <2>2, <3>1
  <2>5. CASE UNCHANGED <<shutdown_event_emitted, shutdown_callback_running,
                         second_event_owed>>
    <3>1. /\ shutdown_event_emitted'[rtId] = shutdown_event_emitted[rtId]
          /\ second_event_owed'[rtId] =
                 second_event_owed[rtId]
      BY <2>5, SMT
    <3>2. QED
      BY <2>2, <3>1
  <2>6. QED
    BY <2>3, <2>4, <2>5, OnlyShutdownStepsWriteShutdownFlags, Zenon
\* And what the event announces holds while it is out: the step reads both
\* ledgers in its guard and moves neither, and an emitted runtime acquires no
\* new debt of either kind afterwards.
<1>6. (IsResourcesReleasedEmitted(rtId))' =>
          /\ (NoHostDebt(rtId))'
          /\ (RuntimeHoldsNoReturnedBytes(rtId))'
  <2>0. SUFFICES ASSUME (IsResourcesReleasedEmitted(rtId))'
                 PROVE  /\ (NoHostDebt(rtId))'
                        /\ (RuntimeHoldsNoReturnedBytes(rtId))'
    OBVIOUS
  <2>1. ASSUME EmitResourcesReleased(rtId)
        PROVE  /\ (NoHostDebt(rtId))'
               /\ (RuntimeHoldsNoReturnedBytes(rtId))'
    BY <1>2, <2>1, SMT
    DEF EmitResourcesReleased, NoHostDebt, RuntimeHoldsNoReturnedBytes,
        HostHoldsNoBuffer, IsReturnedBuffer, L0!ChannelsOf, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, TypeOK, L0!TypeOK
  <2>2. ASSUME IsResourcesReleasedEmitted(rtId)
        PROVE  /\ (NoHostDebt(rtId))'
               /\ (RuntimeHoldsNoReturnedBytes(rtId))'
    <3>1. /\ IsShutdownEventEmitted(rtId)
          /\ NoHostDebt(rtId)
          /\ RuntimeHoldsNoReturnedBytes(rtId)
      BY <2>2, Zenon
      DEF StrongInv, ShutdownSignalInv, ReleaseSignalInv
    <3>2. QED
      BY <3>1, EmittedRuntimeStaysDebtFree, Zenon
  <2>3. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    <3>1. PICK rt \in RuntimeIds : EmitResourcesReleased(rt)
      BY <2>3
    <3>2. CASE rt = rtId
      BY <2>1, <3>1, <3>2
    <3>3. CASE rt # rtId
      <4>1. resources_released_emitted'[rtId] =
                resources_released_emitted[rtId]
        BY <1>2, <3>1, <3>3, SMT
        DEF EmitResourcesReleased, TypeOK, L0!TypeOK
      <4>2. IsResourcesReleasedEmitted(rtId)
        BY <2>0, <4>1, Zenon
      <4>3. QED
        BY <2>2, <4>2
    <3>4. QED
      BY <3>2, <3>3
  <2>4. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    <3>1. resources_released_emitted' = resources_released_emitted
      BY <2>4, SMT DEF ResourcesReleasedCallbackReturns
    <3>2. IsResourcesReleasedEmitted(rtId)
      BY <2>0, <3>1, Zenon
    <3>3. QED
      BY <2>2, <3>2
  <2>5. CASE UNCHANGED <<resources_released_emitted,
                         resources_released_callback_running>>
    <3>1. resources_released_emitted' = resources_released_emitted
      BY <2>5, SMT
    <3>2. IsResourcesReleasedEmitted(rtId)
      BY <2>0, <3>1, Zenon
    <3>3. QED
      BY <2>2, <3>2
  <2>6. QED
    BY <2>3, <2>4, <2>5, OnlyReleaseStepsWriteReleaseFlags, Zenon
<1>7. QED
    BY <1>3, <1>4, <1>5, <1>6

\* Both cases end the same way.  Either the runtime was already destroyed,
\* and the invariant hands over what it needs; or it is being destroyed
\* now, and the guard does.  From there the step is any step, and the
\* previous lemma carries it.
\* Quiescence is absorbing.  Each of its six conjuncts is either an absorbing
\* level-0 state, a callback whose only re-entry the conjunct itself forbids, a
\* latch, or a ledger an emitted runtime cannot refill.  This is what lets the
\* destroy flag carry the whole gate rather than half of it.
LEMMA QuiescentRuntimeStaysQuiescent ==
    ASSUME StrongInv, Next, NEW rtId \in RuntimeIds,
           IsRuntimeQuiescent(rtId)
    PROVE  (IsRuntimeQuiescent(rtId))'
<1>0. TypeOK
    BY Zenon DEF StrongInv
<1>01. /\ IsReleasedRuntime(rtId)
       /\ ~IsShutdownCallbackRunning(rtId)
       /\ ~IsResourcesReleasedCallbackRunning(rtId)
       /\ (SecondEventOwed(rtId) => IsResourcesReleasedEmitted(rtId))
       /\ NoHostDebt(rtId)
       /\ RuntimeHoldsNoReturnedBytes(rtId)
    BY Zenon DEF IsRuntimeQuiescent
\* Released implies the event went out, which is what freezes the flags below.
<1>02. IsShutdownEventEmitted(rtId)
    BY <1>01, Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
<1>1. (IsReleasedRuntime(rtId))'
    BY <1>0, <1>01, ReleasedRuntimeStaysReleased
\* The shutdown callback cannot be re-entered: its only writer demands an event
\* not yet emitted, and this one is.
<1>2. (~IsShutdownCallbackRunning(rtId))'
  <2>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
    BY <1>0, <1>01, <1>02, <2>1, SMT
    DEF EmitShutdownComplete, TypeOK, L0!TypeOK
  <2>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
    BY <1>0, <1>01, <2>2, SMT
    DEF ShutdownCallbackReturns, TypeOK, L0!TypeOK
  <2>3. CASE UNCHANGED <<shutdown_event_emitted, shutdown_callback_running,
                         second_event_owed>>
    BY <1>01, <2>3, SMT
  <2>4. QED
    BY <2>1, <2>2, <2>3, OnlyShutdownStepsWriteShutdownFlags, Zenon
\* And the release callback cannot either: it needs the tag set and the event
\* not yet out, and quiescence says those two cannot both hold.
<1>3. (~IsResourcesReleasedCallbackRunning(rtId))'
  <2>1. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    BY <1>0, <1>01, <2>1, SMT
    DEF EmitResourcesReleased, TypeOK, L0!TypeOK
  <2>2. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    BY <1>0, <1>01, <2>2, SMT
    DEF ResourcesReleasedCallbackReturns, TypeOK, L0!TypeOK
  <2>3. CASE UNCHANGED <<resources_released_emitted,
                         resources_released_callback_running>>
    BY <1>01, <2>3, SMT
  <2>4. QED
    BY <2>1, <2>2, <2>3, OnlyReleaseStepsWriteReleaseFlags, Zenon
\* The tag is frozen and the event is a latch, so the implication between them
\* cannot become false.
<1>4. ((SecondEventOwed(rtId) => IsResourcesReleasedEmitted(rtId)))'
  <2>1. (SecondEventOwed(rtId))' = SecondEventOwed(rtId)
    BY <1>0, <1>02, EmittedRuntimeTagFrozen, Zenon
  <2>2. CASE SecondEventOwed(rtId)
    <3>1. IsResourcesReleasedEmitted(rtId)
      BY <1>01, <2>2, Zenon
    <3>2. QED
      BY <1>0, <3>1, ResourcesEmittedStable, Zenon
  <2>3. CASE ~SecondEventOwed(rtId)
    BY <2>1, <2>3, Zenon
  <2>4. QED
    BY <2>2, <2>3
<1>5. (NoHostDebt(rtId))' /\ (RuntimeHoldsNoReturnedBytes(rtId))'
    BY <1>01, <1>02, EmittedRuntimeStaysDebtFree
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, Zenon DEF IsRuntimeQuiescent

LEMMA NextPreservesDestroyedClean ==
    StrongInv /\ Next => DestroyedRuntimeIsClean'
<1>1. SUFFICES ASSUME StrongInv, Next, NEW rtId \in RuntimeIds,
                      (IsRuntimeDestroyed(rtId))'
               PROVE  (IsRuntimeQuiescent(rtId))'
    BY Zenon DEF DestroyedRuntimeIsClean
<1>2. CASE IsRuntimeDestroyed(rtId)
  <2>1. IsRuntimeQuiescent(rtId)
    BY <1>1, <1>2, Zenon DEF StrongInv, DestroyedRuntimeIsClean
  <2>2. QED
    BY <1>1, <2>1, QuiescentRuntimeStaysQuiescent
\* Only ak_runtime_destroy sets the flag, and only for its own runtime.
<1>3. CASE ~IsRuntimeDestroyed(rtId)
  <2>0. runtime_destroyed \in [RuntimeIds -> BOOLEAN]
    BY <1>1, Zenon DEF StrongInv, TypeOK
  <2>1. RuntimeDestroy(rtId)
    BY <1>1, <1>3, <2>0, SMT
    DEF Next, NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        NextFail, NextExplicitStutter,
        RuntimeCreate, RuntimeBeginShutdown, EmitShutdownComplete,
        ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeRelease, RuntimeDestroy,
        RuntimeFail, RemainFailed, RemainReleased,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, RequestCallCancellation, ReleaseCallHandle,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        SendMessage, EndSend, EmitWriteDone, WriteDoneReturns,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, DeliveryCallbackReturns, HostConsumesEvent,
        L0!RuntimeCreate, L0!RuntimeBeginShutdown, L0!RuntimeRelease,
        L0!RuntimeFail, L0!RemainFailed, L0!RemainReleased,
        L0!ChannelCreate, L0!ChannelStartClosing, L0!ChannelFinishClosing,
        L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
        L0!NetworkReceive, L0!ReceiveStatus, L0!DeliverInitialMetadata,
        L0!DeliverMessage, L0!DeliverStatus, L0!CallCancel,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
        vars, ffi_vars, RequestCancellationOfActiveCalls,
        HandPayloadToHost, HasFreeDeliverySlot,
        HasFreeDeliverySlotForTerminal, IsRuntimeDrained,
        IsRuntimeDestroyed, TypeOK, L0!TypeOK
  <2>2. IsRuntimeQuiescent(rtId)
    BY <2>1, Zenon DEF RuntimeDestroy
  <2>3. QED
    BY <1>1, <2>2, QuiescentRuntimeStaysQuiescent
<1>4. QED
    BY <1>2, <1>3

LEMMA StutterPreservesDestroyedClean ==
    DestroyedRuntimeIsClean /\ UNCHANGED vars => DestroyedRuntimeIsClean'
<1>1. SUFFICES ASSUME DestroyedRuntimeIsClean, UNCHANGED vars,
                      NEW rtId \in RuntimeIds, (IsRuntimeDestroyed(rtId))'
               PROVE  (IsRuntimeQuiescent(rtId))'
    BY Zenon DEF DestroyedRuntimeIsClean
<1>2. IsRuntimeQuiescent(rtId)
    BY <1>1, SMT
    DEF vars, l0_vars, L0!vars, ffi_vars, DestroyedRuntimeIsClean,
        IsRuntimeDestroyed
<1>3. QED
    BY <1>1, <1>2, SMT
    DEF vars, l0_vars, L0!vars, ffi_vars, IsRuntimeQuiescent, IsReleasedRuntime,
        NoHostDebt, RuntimeHoldsNoReturnedBytes, HostHoldsNoBuffer,
        IsReturnedBuffer, L0!ChannelsOf, L0!RuntimeVars, L0!ChannelVars,
        L0!CallVars

LEMMA NextPreservesShutdownSignal ==
    StrongInv /\ Next /\ L0!NotFailed' => ShutdownSignalCore'
<1>1. ASSUME StrongInv, Next, L0!NotFailed'
      PROVE  ShutdownSignalCore'
  <2>1. CASE NextSafeRuntimeOnly
    BY <1>1, <2>1, SMT
    DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
        L0!RuntimeCreate, L0!RuntimeRelease,
        L0!ChannelVars, L0!CallVars, ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>2. CASE NextSafeRuntimeChannel
    BY <1>1, <2>2, SMT
    DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
        L0!RuntimeBeginShutdown, RequestCancellationOfActiveCalls, L0!CallVars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>3. CASE NextSafeChannelOnly
    BY <1>1, <2>3, SMT
    DEF NextSafeChannelOnly, ChannelCreate, ChannelStartClosing,
        L0!ChannelCreate, L0!ChannelStartClosing, RequestCancellationOfActiveCalls,
        L0!RuntimeVars, L0!CallVars, ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>4. CASE NextSafeChannelCall
    BY <1>1, <2>4, SMT
    DEF NextSafeChannelCall, ChannelFinishClosing,
        L0!ChannelFinishClosing, L0!CallsOf, L0!RuntimeVars, ffi_vars,
        L0!EventKinds, L0!StatusKinds, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>5. CASE NextSafeCallOnly
    <3>1. CASE \E cId \in CallIds, chId \in ChannelIds : CallStart(cId, chId)
      BY <1>1, <3>1, SMTT(120)
      DEF CallStart, L0!CallStart, L0!RuntimeVars, L0!ChannelVars,
          ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>2. CASE \E cId \in CallIds, msg \in Messages,
                  bs \in BufferIds : SendMessage(cId, msg, bs)
      <4>1. ASSUME NEW cId \in CallIds, NEW msg \in Messages, NEW bs \in BufferIds,
                   SendMessage(cId, msg, bs)
            PROVE  ShutdownSignalCore'
        <5>1. \A rtId \in RuntimeIds :
                  IsShutdownEventEmitted(rtId) =>
                      ~(call_channel[cId] \in L0!ChannelsOf(rtId))
          BY <1>1, <4>1, SMTT(120)
          DEF SendMessage, L0!SendMessage,
              ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
              StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
              L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
              L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
              L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
              L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
              L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
              TypeOK, L0!TypeOK,
              L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
              L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
              L0!CallStates
        \* Only the quiescence conjunct can move: a send touches no
        \* runtime state, and it happens on a call whose channel belongs
        \* to no runtime that has already signalled.
        <5>20. /\ UNCHANGED <<runtime_state, shutdown_event_emitted,
                              shutdown_callback_running, channel_state,
                              channel_runtime, call_channel,
                              events_delivered>>
               /\ UNCHANGED <<delivery_callback_running,
                              write_dones_emitted,
                              write_done_callback_running>>
          BY <4>1, SMT DEF SendMessage, L0!SendMessage, L0!RuntimeVars,
              L0!ChannelVars, ffi_vars
        <5>21. \A rtId \in RuntimeIds :
                   /\ (IsShutdownCallbackRunning(rtId))' =
                          IsShutdownCallbackRunning(rtId)
                   /\ (IsShutdownEventEmitted(rtId))' =
                          IsShutdownEventEmitted(rtId)
                   /\ (IsStoppingRuntime(rtId))' = IsStoppingRuntime(rtId)
                   /\ (IsReleasedRuntime(rtId))' = IsReleasedRuntime(rtId)
                   /\ (L0!ChannelsOf(rtId))' = L0!ChannelsOf(rtId)
          BY <5>20, SMT DEF L0!ChannelsOf
        <5>22. \A chId \in ChannelIds :
                   (IsClosedChannel(chId))' = IsClosedChannel(chId)
          BY <5>20, SMT
        <5>23. \A c \in CallIds :
                   /\ (IsDeliveryCallbackRunning(c))' =
                          IsDeliveryCallbackRunning(c)
                   /\ (call_channel[c])' = call_channel[c]
          BY <5>20, SMT
        <5>24. \A c \in CallIds :
                   (HasNoSendInFlight(c))' =
                       (IF c = cId THEN FALSE ELSE HasNoSendInFlight(c))
          BY <1>1, <4>1, SendMessageTransfers, Zenon
              DEF StrongInv
        <5>25. \A rtId \in RuntimeIds :
                   /\ ((IsShutdownCallbackRunning(rtId) =>
                            IsShutdownEventEmitted(rtId)))'
                   /\ ((IsShutdownEventEmitted(rtId) =>
                            (IsStoppingRuntime(rtId) \/
                             IsReleasedRuntime(rtId))))'
                   /\ ((IsReleasedRuntime(rtId) =>
                            /\ IsShutdownEventEmitted(rtId)
                            /\ ~IsShutdownCallbackRunning(rtId)))'
          BY <1>1, <5>21, SMT DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
        \* Everything here is carried as an implication: an ASSUME
        \* hypothesis placed after a NEW is silently dropped by this
        \* prover, and the resulting obligation is unprovable.
        <5>26. ASSUME NEW rtId \in RuntimeIds
               PROVE  IsShutdownEventEmitted(rtId) =>
                          (IsRuntimeDrained(rtId))'
          <6>20. ShutdownSignalInv
            BY <1>1, Zenon DEF StrongInv
          <6>2. IsShutdownEventEmitted(rtId) => IsRuntimeDrained(rtId)
            BY <6>20, Zenon DEF ShutdownSignalInv, ShutdownSignalCore
          <6>3. IsShutdownEventEmitted(rtId) =>
                    /\ (\A chId \in L0!ChannelsOf(rtId) :
                            IsClosedChannel(chId))
                    /\ (\A c \in CallIds :
                            call_channel[c] \in L0!ChannelsOf(rtId) =>
                                /\ ~IsDeliveryCallbackRunning(c)
                                /\ HasNoSendInFlight(c))
            BY <6>2, Zenon DEF IsRuntimeDrained
          <6>5. IsShutdownEventEmitted(rtId) =>
                    ~(call_channel[cId] \in L0!ChannelsOf(rtId))
            BY <5>1, Zenon
          <6>6. IsShutdownEventEmitted(rtId) =>
                    (\A chId \in L0!ChannelsOf(rtId) :
                         IsClosedChannel(chId))'
            BY <5>20, <6>3, SMT DEF L0!ChannelsOf
          <6>7. IsShutdownEventEmitted(rtId) =>
                    (\A c \in CallIds :
                         call_channel[c] \in L0!ChannelsOf(rtId) =>
                             /\ ~IsDeliveryCallbackRunning(c)
                             /\ HasNoSendInFlight(c))'
            BY <5>20, <5>24, <6>3, <6>5, SMT DEF L0!ChannelsOf
          <6>8. QED
            BY <6>6, <6>7, Zenon DEF IsRuntimeDrained
        <5>27. \A rtId \in RuntimeIds :
                   ((IsShutdownEventEmitted(rtId) =>
                         IsRuntimeDrained(rtId)))'
          BY <5>21, <5>26, Zenon
        <5>2. QED
          BY <5>25, <5>27, Zenon DEF ShutdownSignalInv, ShutdownSignalCore
      <4>2. QED BY <3>2, <4>1
    <3>3. CASE \E cId \in CallIds : EndSend(cId)
      BY <1>1, <3>3, SMTT(120)
      DEF EndSend, L0!EndSend, L0!RuntimeVars, L0!ChannelVars, ffi_vars,
          ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>4. CASE \E cId \in CallIds : NetworkSend(cId)
      BY <1>1, <3>4, SMTT(120)
      DEF NetworkSend, L0!NetworkSend, L0!RuntimeVars, L0!ChannelVars,
          ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>5. CASE \E cId \in CallIds, msg \in Messages : NetworkReceive(cId, msg)
      BY <1>1, <3>5, SMTT(120)
      DEF NetworkReceive, L0!NetworkReceive, L0!RuntimeVars,
          L0!ChannelVars, ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>6. CASE \E cId \in CallIds : ReceiveStatus(cId)
      BY <1>1, <3>6, SMTT(120)
      DEF ReceiveStatus, L0!ReceiveStatus, L0!RuntimeVars, L0!ChannelVars,
          ffi_vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>7. CASE \E cId \in CallIds : DeliverInitialMetadata(cId)
      BY <1>1, <3>7, SMTT(120)
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot,
          L0!RuntimeVars, L0!ChannelVars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>8. CASE \E cId \in CallIds : DeliverMessage(cId)
      BY <1>1, <3>8, SMTT(120)
      DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot,
          L0!RuntimeVars, L0!ChannelVars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>9. CASE \E cId \in CallIds : DeliverStatus(cId)
      BY <1>1, <3>9, SMTT(120)
      DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
          L0!RuntimeVars, L0!ChannelVars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>10. CASE \E cId \in CallIds : DeliverCancelled(cId)
      BY <1>1, <3>10, SMTT(120)
      DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
          L0!RuntimeVars, L0!ChannelVars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
    <3>11. QED BY <1>1, <2>5, <3>1, <3>2, <3>3, <3>4, <3>5, <3>6, <3>7,
                   <3>8, <3>9, <3>10 DEF NextSafeCallOnly
  <2>6. CASE NextSafeShutdownFfi
    BY <1>1, <2>6, SMT
    DEF NextSafeShutdownFfi, EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
    ResourcesReleasedCallbackReturns, EmitResourcesReleased,
    ResourcesReleasedCallbackReturns, RuntimeDestroy,
        l0_vars, L0!vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>7. CASE NextSafeCallFfi
    BY <1>1, <2>7, SMT
    DEF NextSafeCallFfi, RequestCallCancellation, ReleaseCallHandle, EmitWriteDone,
        WriteDoneReturns, DeliveryCallbackReturns, HostConsumesEvent,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        l0_vars, L0!vars, ShutdownSignalInv, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf,
        StrongInv, L0!StrongInv, L0!StructuralInv, L0!SingleRuntime,
        L0!ChannelSentinelEquivalence, L0!CallSentinelEquivalence,
        L0!UnusedCallsAreEmpty, L0!ChannelLifecycleInv,
        L0!CallLifecycleInv, L0!TerminalStatusEquivalence,
        L0!UsedChannels, L0!UsedCalls, L0!ActiveChannels,
        L0!ActiveChannelStates, L0!NotFailed, L0!HasStatus,
        TypeOK, L0!TypeOK, FfiCallInv, UnusedCallsAreFfiClean,
        ReleasedCallIsClean, TerminalCallHasNoSendInFlight,
        ClosingChannelCallsCancelRequested, ActiveCallPayloadsWithinCredits,
        PayloadsOwnedWithinCreditsPlusOne, NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus, UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!IsActiveCall,
        L0!ActiveCallStates, L0!RuntimeStates, L0!ChannelStates,
        L0!CallStates
  <2>8. CASE NextFail
    BY <1>1, <2>8, SMT
    DEF NextFail, RuntimeFail, L0!RuntimeFail, L0!NotFailed,
        TypeOK, L0!TypeOK, StrongInv
  <2>9. CASE NextExplicitStutter
    \* The two halves apart: the goal is a nested tuple, and proving it
    \* in one go asks the solver to reason under two levels of tupling.
    <3>10. UNCHANGED l0_vars
      BY <2>9, StutterProjects, SMT
      DEF L0!NextExplicitStutter, L0!RemainFailed, L0!RemainReleased,
          L0!vars, l0_vars
    <3>11. UNCHANGED ffi_vars
      BY <2>9, SMT
      DEF NextExplicitStutter, RemainFailed, RemainReleased
    <3>1. UNCHANGED vars
      BY <3>10, <3>11, SMT DEF vars
    <3>2. QED BY <1>1, <3>1, StutterPreservesShutdownSignal DEF StrongInv, ShutdownSignalInv
  <2>10. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8,
                 <2>9, NextDecomposition
         DEF NextByFootprint, NextSafe, NextSafeRefining, NextSafeFfiOnly
<1>2. QED BY <1>1

(***************************************************************************)
(* INDUCTIVE INVARIANT - established, preserved, projected                 *)
(***************************************************************************)

LEMMA StutterPreservesFfi ==
    TypeOK /\ FfiCallInv /\ UNCHANGED vars => (FfiTypes /\ FfiCallInv)'
<1>1. ASSUME TypeOK, FfiCallInv, UNCHANGED vars
      PROVE  (FfiTypes /\ FfiCallInv)'
  <2>1. /\ UNCHANGED ffi_vars
        /\ UNCHANGED <<buffers_held_by_host, write_dones_emitted,
                       write_done_callback_running,
                       delivery_callback_running,
                       payloads_consumed_by_host,
                       handle_released, cancel_requested>>
        /\ UNCHANGED <<call_state, call_channel, channel_state,
                       events_delivered, submitted>>
    BY <1>1, SMT DEF vars, l0_vars, L0!vars, L0!RuntimeVars,
        L0!ChannelVars, L0!CallVars, ffi_vars
  <2>2. FfiTypes'
    BY <1>1, <2>1, TypeOKSplit, UnchangedFfiPreservesFfiTypes
  <2>3. FfiCallInv'
    BY <1>1, <2>1, FfiFramePreservesFfiCallInv
  <2>4. QED
    BY <2>2, <2>3
<1>2. QED
    BY <1>1

THEOREM InitEstablishesIndInv == Init => IndInv
<1>0. SUFFICES ASSUME Init PROVE IndInv
    OBVIOUS
<1>1. L0!IndInv
    BY <1>0, L0!InitEstablishesIndInv,
       L0Assumptions, Zenon DEF Init
<1>2. FfiTypes
    BY <1>0, FS_EmptySet, SMT DEF Init, FfiTypes, LendStatuses
<1>25. BufferTypes
    BY <1>0, SMT DEF Init, BufferTypes, BufferStates
<1>3. TypeOK
    BY <1>1, <1>2, <1>25, TypeOKSplit DEF L0!IndInv
\* Nothing is lent yet, so every lent set is empty and its cardinality is
\* zero - the one place the bridge needs the empty-set fact.
\* Nothing is out and the counter is zero, so the accounting is the empty
\* sum and the ceiling bound is trivial.
<1>26. MemoryAccountingExact /\ MemoryWithinCeiling
  <2>1. OutstandingPairs = {}
    BY <1>0, SMT
    DEF Init, OutstandingPairs, BufferOutstanding, IsLentBuffer,
        IsReturnedBuffer
  <2>2. BytesOutstanding = 0
    BY <2>1, SumFunctionOnSetEmpty, Zenon DEF BytesOutstanding
  <2>3. QED
    BY <1>0, <2>2, CeilingIsPositive, SMT
    DEF Init, MemoryAccountingExact, MemoryWithinCeiling, CeilingIsPositive
<1>28. BufferStateInv
  <2>1. \A c \in CallIds : {x \in BufferIds : buffer_state[c][x] = "lent"} = {}
    BY <1>0, Zenon DEF Init
  <2>2. \A c \in CallIds : Cardinality({x \in BufferIds : buffer_state[c][x] = "lent"}) = 0
    BY <2>1, FS_EmptySet, Zenon
\* The three link conjuncts are vacuous here: every entry is zero, so no
\* buffer carries a send and none can be unacquitted.
  <2>3. \A c \in CallIds : Len(submitted[c]) = 0
    BY <1>0, Zenon DEF Init, L0!Init
  <2>4. QED
    BY <1>0, <2>2, <2>3, SMT
    DEF Init, BufferStateInv, LentCountMatchesBufferStates,
        UnusedCallsHaveFreshBuffers, BufferSendIndicesExist,
        CommittedBuffersAreGivenBack, UnacquittedSendKeepsItsBytes,
        EverySendHasItsBuffer, SendsLiveInOneBuffer,
        IsLentBuffer, IsFreshBuffer
\* Proved in three groups: every call is unused and every counter zero,
\* but the conjunct list is long enough that one goal outgrows the budget.
<1>38. /\ \A c \in CallIds : Len(submitted[c]) = 0
       /\ \A c \in CallIds : Len(events_delivered[c]) = 0
       /\ \A c \in CallIds : ~L0!IsActiveCall(c)
       /\ \A c \in CallIds : ~IsHandleReleased(c)
       /\ \A c \in CallIds : ~IsCancelRequested(c)
    BY <1>0, SMT
    DEF Init, L0!Init, L0!IsActiveCall, L0!ActiveCallStates,
        IsHandleReleased, IsCancelRequested
<1>39. UnusedCallsAreFfiClean
    BY <1>0, <1>38, SMT
    DEF Init, UnusedCallsAreFfiClean, HasNoSendInFlight,
        HostHoldsNoBuffer, HostOwnsNoPayload, OwedPayloads,
        IsDeliveryCallbackRunning, IsHandleReleased, IsCancelRequested
<1>40. /\ ReleasedCallIsClean
       /\ ClosingChannelCallsCancelRequested
    BY <1>38, Zenon
    DEF ReleasedCallIsClean, ClosingChannelCallsCancelRequested
<1>41. /\ SendsInFlightWithinLimit
       /\ WriteDonesNeverExceedSends
       /\ RunningWriteDoneWasEmitted
       /\ TerminalCallHasNoSendInFlight
    BY <1>0, MaxSendsInFlightIsPositive, SMT
    DEF Init, L0!Init, SendsInFlightWithinLimit,
        WriteDonesNeverExceedSends, RunningWriteDoneWasEmitted,
        TerminalCallHasNoSendInFlight, L0!IsTerminalCall
<1>42. /\ ReleasesNeverExceedDeliveries
       /\ ActiveCallPayloadsWithinCredits
       /\ PayloadsOwnedWithinCreditsPlusOne
       /\ NoDeliveryImpliesNoDebt
       /\ ActiveCallHasNoStatus
       /\ UnusedCallHasNoEvents
    BY <1>0, DeliveryCreditsArePositive, SMT
    DEF Init, L0!Init, ReleasesNeverExceedDeliveries,
        ActiveCallPayloadsWithinCredits, PayloadsOwnedWithinCreditsPlusOne,
        NoDeliveryImpliesNoDebt, ActiveCallHasNoStatus,
        UnusedCallHasNoEvents,
        L0!IsUnusedCall, L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus
<1>4. FfiCallInv
    BY <1>39, <1>40, <1>41, <1>42, Zenon DEF FfiCallInv
<1>5. ShutdownSignalInv
\* One conjunct at a time: the core reads the level-0 initial state, the
\* release half reads only the three flags, and one call over both is what
\* went over budget.
  <2>1. ShutdownSignalCore
    BY <1>0, SMT
    DEF Init, L0!Init, ShutdownSignalCore, IsRuntimeDrained, L0!ChannelsOf
  <2>2. ReleaseSignalInv
    BY <1>0, SMT DEF Init, ReleaseSignalInv
  <2>3. QED
    BY <2>1, <2>2, Zenon DEF ShutdownSignalInv
<1>50. DestroyedRuntimeIsClean
    BY <1>0, SMT DEF Init, DestroyedRuntimeIsClean, IsRuntimeDestroyed
<1>6. QED
    BY <1>0, <1>1, <1>26, <1>28, <1>3, <1>4, <1>5, <1>50, Zenon
    DEF IndInv, StrongInv, L0!IndInv

THEOREM IndInvPreserved == IndInv /\ [Next]_vars => IndInv'
<1>0. SUFFICES ASSUME IndInv, [Next]_vars PROVE IndInv'
    OBVIOUS
<1>1. L0!IndInv'
    BY <1>0, IndInvProjects, RefinesNext, L0!IndInvPreserved,
       L0Assumptions, Zenon
    DEF l0_vars
<1>2. (FfiTypes /\ FfiCallInv)'
  <2>1. CASE Next
    BY <1>0, <2>1, NextPreservesFfiTypes, NextPreservesFfiCallInv,
       TypeOKSplit DEF IndInv
  <2>2. CASE UNCHANGED vars
    BY <1>0, <2>2, StutterPreservesFfi DEF IndInv
  <2>3. QED BY <1>0, <2>1, <2>2
<1>25. (BufferTypes /\ BufferStateInv)'
  <2>0. TypeOK /\ FfiCallInv /\ BufferStateInv
    BY <1>0, Zenon DEF IndInv
  <2>1. CASE Next
    BY <1>0, <2>0, <2>1, NextPreservesBufferTypes,
       NextPreservesLentCountBridge, NextPreservesFreshBuffers,
       NextPreservesBufferSendLink, NextPreservesSendBufferMatch
       DEF IndInv, BufferStateInv
  <2>2. CASE UNCHANGED vars
    <3>1. UNCHANGED <<buffer_state, buffers_held_by_host, call_state,
                      buffer_send, submitted, write_dones_emitted>>
      BY <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, L0!CallVars
\* Split because the send-link conjuncts carry arithmetic and the other two
\* carry a cardinality: no single backend is good at both.
    <3>2. (BufferTypes /\ LentCountMatchesBufferStates
              /\ UnusedCallsHaveFreshBuffers)'
      BY <1>0, <3>1, Zenon
      DEF IndInv, BufferStateInv, BufferTypes,
          LentCountMatchesBufferStates, UnusedCallsHaveFreshBuffers,
          IsLentBuffer, IsFreshBuffer, L0!IsUnusedCall, TypeOK
    <3>3. (BufferSendIndicesExist /\ CommittedBuffersAreGivenBack
              /\ UnacquittedSendKeepsItsBytes)'
      BY <1>0, <3>1, Zenon
      DEF IndInv, BufferStateInv, BufferSendIndicesExist,
          CommittedBuffersAreGivenBack, UnacquittedSendKeepsItsBytes
\* The matching pair goes on its own: one half is an existential over the
\* identity space, which Zenon will not instantiate.
    <3>35. (EverySendHasItsBuffer /\ SendsLiveInOneBuffer)'
      BY <1>0, <3>1
      DEF IndInv, BufferStateInv, EverySendHasItsBuffer,
          SendsLiveInOneBuffer
    <3>4. QED BY <3>2, <3>3, <3>35, Zenon DEF BufferStateInv
  <2>3. QED BY <1>0, <2>1, <2>2
<1>26. MemoryAccountingExact' /\ MemoryWithinCeiling'
  <2>1. TypeOK /\ MemoryAccountingExact /\ MemoryWithinCeiling
    BY <1>0, Zenon DEF IndInv
  <2>2. QED
    BY <1>0, <2>1, NextPreservesAccounting, NextPreservesCeiling
<1>3. TypeOK'
    BY <1>1, <1>2, <1>25, SMT DEF TypeOK, FfiTypes, BufferTypes, LendStatuses, L0!IndInv
<1>4. ASSUME L0!NotFailed' PROVE StrongInv'
  <2>1. L0!NotFailed
    <3>1. CASE Next
      BY <1>0, <1>4, <3>1, FailedPersists DEF IndInv
    <3>2. CASE UNCHANGED vars
      BY <1>0, <1>4, <3>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, L0!NotFailed
    <3>3. QED BY <1>0, <3>1, <3>2
  <2>2. StrongInv
    BY <1>0, <2>1 DEF IndInv
  <2>3. L0!StrongInv'
    BY <1>1, <1>4 DEF L0!IndInv
  <2>4. ShutdownSignalInv'
    <3>1. CASE Next
      BY <2>2, <1>4, <3>1, NextPreservesShutdownSignal,
         NextPreservesReleaseSignal DEF ShutdownSignalInv
    <3>2. CASE UNCHANGED vars
      BY <2>2, <3>2, StutterPreservesShutdownSignal DEF StrongInv
    <3>3. QED BY <1>0, <3>1, <3>2
  <2>40. DestroyedRuntimeIsClean'
    <3>1. CASE Next
      BY <2>2, <3>1, NextPreservesDestroyedClean
    <3>2. CASE UNCHANGED vars
      BY <2>2, <3>2, StutterPreservesDestroyedClean DEF StrongInv
    <3>3. QED BY <1>0, <3>1, <3>2
  <2>5. QED
    BY <1>2, <1>25, <1>26, <1>3, <2>3, <2>4, <2>40 DEF StrongInv
<1>5. QED
    BY <1>1, <1>2, <1>25, <1>26, <1>3, <1>4
    DEF IndInv, L0!IndInv, StrongInv

THEOREM IndInvImpliesSafetyInvariant == IndInv => SafetyInvariant
<1>1. IndInv => L0!SafetyInvariant
    BY IndInvProjects, L0!IndInvImpliesSafetyInvariant,
       L0Assumptions, Zenon
<1>2. QED
    BY <1>1 DEF IndInv, StrongInv, SafetyInvariant

THEOREM BehaviorEstablishesIndInv ==
    Init /\ [][Next]_vars => []IndInv
<1>1. QED
    BY InitEstablishesIndInv, IndInvPreserved, PTL

THEOREM SafetyTheorem == Spec => []SafetyInvariant
<1>1. Spec => []IndInv
    BY BehaviorEstablishesIndInv, PTL DEF Spec
<1>2. QED
    BY <1>1, IndInvImpliesSafetyInvariant, PTL

(***************************************************************************)
(* THE SAFE WORLD - []StrongInv under [][NextSafe]_vars                    *)
(***************************************************************************)

\* The simultaneous induction step: the healthy strengthening survives a
\* safe step.  The level-0 half rides on L0!IndInvPreserved, which also
\* absorbs the stutter.
LEMMA SafeStepPreservesHealthyStrongInv ==
    StrongInv /\ L0!NotFailed /\ [NextSafe]_vars =>
        StrongInv' /\ L0!NotFailed'
<1>0. SUFFICES ASSUME StrongInv, L0!NotFailed, [NextSafe]_vars
               PROVE  StrongInv' /\ L0!NotFailed'
    OBVIOUS
<1>1. TypeOK
    BY <1>0 DEF StrongInv
<1>2. L0!NotFailed'
    BY <1>0, <1>1, SafeKeepsNotFailed
<1>3. L0!IndInv
    BY <1>0, SMT
    DEF StrongInv, L0!IndInv, TypeOK, L0!StrongInv, L0!StructuralInv
<1>4. [L0!Next]_l0_vars
    BY <1>0, RefinesSafeNext, L0SafeIsNext DEF NextSafe
<1>5. L0!StrongInv'
    BY <1>2, <1>3, <1>4, L0!IndInvPreserved,
       L0Assumptions, Zenon
    DEF l0_vars, L0!IndInv
<1>6. (FfiTypes /\ FfiCallInv)'
  <2>1. CASE NextSafeRefining
    BY <1>0, <1>1, <2>1, RefiningPreservesFfiTypes,
       RuntimeOnlyPreservesFfiCallInv, RuntimeChannelPreservesFfiCallInv,
       ChannelOnlyPreservesFfiCallInv, ChannelCallPreservesFfiCallInv,
       CallOnlyPreservesFfiCallInv, TypeOKSplit
    DEF StrongInv, NextSafeRefining
  <2>2. CASE NextSafeFfiOnly
    BY <1>0, <1>1, <2>2, FfiOnlyPreservesFfiTypes,
       FfiOnlyPreservesFfiCallInv, TypeOKSplit DEF StrongInv
  <2>3. CASE UNCHANGED vars
    BY <1>0, <1>1, <2>3, StutterPreservesFfi DEF StrongInv
  <2>4. QED BY <1>0, <2>1, <2>2, <2>3 DEF NextSafe
<1>7. ShutdownSignalInv'
  <2>1. CASE NextSafe
    <3>1. Next
      BY <2>1, NextDecomposition DEF NextByFootprint
    <3>2. QED BY <1>0, <1>2, <3>1, NextPreservesShutdownSignal,
                 NextPreservesReleaseSignal DEF ShutdownSignalInv
  <2>2. CASE UNCHANGED vars
    BY <1>0, <2>2, StutterPreservesShutdownSignal DEF StrongInv
  <2>3. QED BY <1>0, <2>1, <2>2 DEF NextSafe
<1>70. DestroyedRuntimeIsClean'
  <2>1. CASE NextSafe
    <3>1. Next
      BY <2>1, NextDecomposition DEF NextByFootprint
    <3>2. QED BY <1>0, <3>1, NextPreservesDestroyedClean
  <2>2. CASE UNCHANGED vars
    BY <1>0, <2>2, StutterPreservesDestroyedClean DEF StrongInv
  <2>3. QED BY <1>0, <2>1, <2>2 DEF NextSafe
<1>75. (BufferTypes /\ BufferStateInv)'
  <2>0. TypeOK /\ FfiCallInv /\ BufferStateInv
    BY <1>0, Zenon DEF StrongInv
  <2>1. CASE NextSafe
    <3>1. Next
      BY <2>1, NextDecomposition DEF NextByFootprint
    <3>2. QED
      BY <2>0, <3>1, NextPreservesBufferTypes,
         NextPreservesLentCountBridge, NextPreservesFreshBuffers,
         NextPreservesBufferSendLink, NextPreservesSendBufferMatch
         DEF BufferStateInv
  <2>2. CASE UNCHANGED vars
    <3>1. UNCHANGED <<buffer_state, buffers_held_by_host, call_state,
                      buffer_send, submitted, write_dones_emitted>>
      BY <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, L0!CallVars
\* Same split as in IndInvPreserved: the send-link conjuncts carry
\* arithmetic and the other two a cardinality.
    <3>2. (BufferTypes /\ LentCountMatchesBufferStates
              /\ UnusedCallsHaveFreshBuffers)'
      BY <2>0, <3>1, Zenon
      DEF BufferStateInv, BufferTypes, LentCountMatchesBufferStates,
          UnusedCallsHaveFreshBuffers, IsLentBuffer, IsFreshBuffer,
          L0!IsUnusedCall, TypeOK
    <3>3. (BufferSendIndicesExist /\ CommittedBuffersAreGivenBack
              /\ UnacquittedSendKeepsItsBytes)'
      BY <2>0, <3>1, Zenon
      DEF BufferStateInv, BufferSendIndicesExist,
          CommittedBuffersAreGivenBack, UnacquittedSendKeepsItsBytes
\* The matching pair goes on its own: one half is an existential over the
\* identity space, which Zenon will not instantiate.
    <3>35. (EverySendHasItsBuffer /\ SendsLiveInOneBuffer)'
      BY <2>0, <3>1
      DEF BufferStateInv, EverySendHasItsBuffer, SendsLiveInOneBuffer
    <3>4. QED BY <3>2, <3>3, <3>35, Zenon DEF BufferStateInv
  <2>3. QED BY <1>0, <2>1, <2>2
<1>8. TypeOK'
    BY <1>5, <1>6, <1>75, SMT
    DEF TypeOK, FfiTypes, BufferTypes, LendStatuses, L0!StrongInv, L0!StructuralInv
<1>85. MemoryAccountingExact' /\ MemoryWithinCeiling'
  <2>1. MemoryAccountingExact /\ MemoryWithinCeiling
    BY <1>0, Zenon DEF StrongInv
\* SafeIsNext is declared further down; the decomposition is in scope here.
  <2>2. [Next]_vars
    BY <1>0, NextDecomposition, Zenon DEF NextByFootprint
  <2>3. QED
    BY <1>1, <2>1, <2>2, NextPreservesAccounting, NextPreservesCeiling
<1>9. QED
    BY <1>2, <1>5, <1>6, <1>7, <1>70, <1>75, <1>8, <1>85 DEF StrongInv

LEMMA InitHealthy == Init => StrongInv /\ L0!NotFailed
<1>1. Init => IndInv
    BY InitEstablishesIndInv
<1>2. Init => L0!NotFailed
    BY SMT DEF Init, L0!Init, L0!NotFailed
<1>3. QED
    BY <1>1, <1>2 DEF IndInv

THEOREM NominalBehaviorEstablishesStrongInv ==
    Init /\ [][NextSafe]_vars => []StrongInv
<1>1. Init /\ [][NextSafe]_vars => [](StrongInv /\ L0!NotFailed)
    BY InitHealthy, SafeStepPreservesHealthyStrongInv, PTL
<1>2. QED
    BY <1>1, PTL

(***************************************************************************)
(* ENABLING BRIDGES                                                        *)
(* State-level facts: whenever the level-0 action is enabled and the FFI   *)
(* conditions hold, the level-1 action is enabled.  Slim hypotheses only:  *)
(* ENABLED expansion drowns under a full StrongInv.                        *)
(***************************************************************************)

\* A level-1 step of a refining action is a level-0 step of the refined
\* action: the level-0 conjunct changes the level-0 state on its own.
LEMMA NetworkSendStepProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<NetworkSend(cId)>>_vars =>
               <<L0!NetworkSend(cId)>>_l0_vars
<1>1. QED
    BY SMT DEF NetworkSend, L0!NetworkSend, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

LEMMA ReceiveStatusStepProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<ReceiveStatus(cId)>>_vars =>
               <<L0!ReceiveStatus(cId)>>_l0_vars
<1>1. QED
    BY SMT DEF ReceiveStatus, L0!ReceiveStatus, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

LEMMA MetadataStepProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverInitialMetadata(cId)>>_vars =>
               <<L0!DeliverInitialMetadata(cId)>>_l0_vars
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
        HandPayloadToHost, HasFreeDeliverySlot, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

\* Enabling bridges at fixed state.  The witness of the level-1 ENABLED
\* extends the level-0 witness by the explicit FFI assignments.
LEMMA NetworkSendEnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ ENABLED <<L0!NetworkSend(cId)>>_l0_vars =>
               ENABLED <<NetworkSend(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF NetworkSend, L0!NetworkSend, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

LEMMA ReceiveStatusEnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ ENABLED <<L0!ReceiveStatus(cId)>>_l0_vars =>
               ENABLED <<ReceiveStatus(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF ReceiveStatus, L0!ReceiveStatus, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

\* The metadata slot is free on its own: no event delivered yet means no
\* debt (NoDeliveryImpliesNoDebt), so the level-1 guard follows from the level-0
\* one and the invariant.
LEMMA MetadataEnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ NoDeliveryImpliesNoDebt /\
               ENABLED <<L0!DeliverInitialMetadata(cId)>>_l0_vars =>
               ENABLED <<DeliverInitialMetadata(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, FS_EmptySet, DeliveryCreditsArePositive, SMT
    DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot,
        NoDeliveryImpliesNoDebt, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates

(***************************************************************************)
(* THE THREE DIRECT LIFTS                                                  *)
(* Shape: the enabling bridge turns persistent level-0 enabledness into    *)
(* persistent level-1 enabledness, the level-1 WF fires, and the step      *)
(* projection turns the level-1 step into the level-0 step.                *)
(***************************************************************************)

THEOREM NetworkSendLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(NetworkSend(cId))
           => WF_l0_vars(L0!NetworkSend(cId))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(NetworkSend(cId))
      PROVE  WF_l0_vars(L0!NetworkSend(cId))
  <2>1. [](TypeOK /\ <<NetworkSend(cId)>>_vars =>
               <<L0!NetworkSend(cId)>>_l0_vars)
    BY NetworkSendStepProjects, PTL
  <2>2. [](TypeOK /\ ENABLED <<L0!NetworkSend(cId)>>_l0_vars =>
               ENABLED <<NetworkSend(cId)>>_vars)
    BY NetworkSendEnabledBridge, PTL
  <2>3. []TypeOK
    BY <1>1, PTL DEF IndInv
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, PTL
<1>2. QED BY <1>1, PTL

THEOREM ReceiveStatusLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(ReceiveStatus(cId))
           => WF_l0_vars(L0!ReceiveStatus(cId))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(ReceiveStatus(cId))
      PROVE  WF_l0_vars(L0!ReceiveStatus(cId))
  <2>1. [](TypeOK /\ <<ReceiveStatus(cId)>>_vars =>
               <<L0!ReceiveStatus(cId)>>_l0_vars)
    BY ReceiveStatusStepProjects, PTL
  <2>2. [](TypeOK /\ ENABLED <<L0!ReceiveStatus(cId)>>_l0_vars =>
               ENABLED <<ReceiveStatus(cId)>>_vars)
    BY ReceiveStatusEnabledBridge, PTL
  <2>3. []TypeOK
    BY <1>1, PTL DEF IndInv
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, PTL
<1>2. QED BY <1>1, PTL

THEOREM MetadataDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverInitialMetadata(cId))
           => WF_l0_vars(L0!DeliverInitialMetadata(cId))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliverInitialMetadata(cId))
      PROVE  WF_l0_vars(L0!DeliverInitialMetadata(cId))
  <2>1. [](TypeOK /\ <<DeliverInitialMetadata(cId)>>_vars =>
               <<L0!DeliverInitialMetadata(cId)>>_l0_vars)
    BY MetadataStepProjects, PTL
  <2>2. [](TypeOK /\ NoDeliveryImpliesNoDebt /\
               ENABLED <<L0!DeliverInitialMetadata(cId)>>_l0_vars =>
               ENABLED <<DeliverInitialMetadata(cId)>>_vars)
    BY MetadataEnabledBridge, PTL
  <2>3. [](TypeOK /\ NoDeliveryImpliesNoDebt)
    BY <1>1, PTL DEF IndInv, FfiCallInv
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* SHARED DRAIN BRIDGES                                                    *)
(* State and action facts consumed by the four remaining lifts and by the  *)
(* new liveness guarantees.  All stated on the full Next: [][NextSafe]     *)
(* implies [][Next] disjunct-wise, so the safe world reuses them.          *)
(***************************************************************************)

\* Every action a frame is subscripted on necessarily changes the state,
\* so <<A>>_vars and A are the same thing.  Saying so once collapses the
\* subscript, and with it the reasoning about inequality of a
\* twenty-two-variable tuple that every one of those obligations was
\* otherwise carrying.
LEMMA SubscriptCollapses ==
    ASSUME TypeOK, NEW cId \in CallIds
    PROVE  /\ <<EmitWriteDone(cId)>>_vars <=> EmitWriteDone(cId)
           /\ <<WriteDoneReturns(cId)>>_vars <=> WriteDoneReturns(cId)
           /\ <<DeliveryCallbackReturns(cId)>>_vars <=>
                  DeliveryCallbackReturns(cId)
           /\ <<HostConsumesEvent(cId)>>_vars <=> HostConsumesEvent(cId)
<1>1. QED
    BY SMT DEF EmitWriteDone, WriteDoneReturns, DeliveryCallbackReturns,
        HostConsumesEvent, IsWriteDoneCallbackRunning,
        IsDeliveryCallbackRunning, IsAwaitingWriteDone,
        HostOwnsSomePayload, OwedPayloads, vars, ffi_vars, TypeOK

\* The same, for the actions whose change is on the level-0 side.
LEMMA DeliverySubscriptCollapses ==
    ASSUME TypeOK, NEW cId \in CallIds
    PROVE  /\ <<DeliverInitialMetadata(cId)>>_vars <=>
                  DeliverInitialMetadata(cId)
           /\ <<DeliverMessage(cId)>>_vars <=> DeliverMessage(cId)
           /\ <<DeliverStatus(cId)>>_vars <=> DeliverStatus(cId)
           /\ <<DeliverCancelled(cId)>>_vars <=> DeliverCancelled(cId)
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, HasFreeDeliverySlot,
        HasFreeDeliverySlotForTerminal, IsDeliveryCallbackRunning,
        vars, ffi_vars, TypeOK

\* The runtime-level pair, for the shutdown frames.
LEMMA RuntimeSubscriptCollapses ==
    ASSUME TypeOK, NEW rtId \in RuntimeIds
    PROVE  /\ <<EmitShutdownComplete(rtId)>>_vars <=>
                  EmitShutdownComplete(rtId)
           /\ <<ShutdownCallbackReturns(rtId)>>_vars <=>
                  ShutdownCallbackReturns(rtId)
<1>1. QED
    BY SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, vars, ffi_vars, TypeOK

\* The buffer pair, for the frames above.
LEMMA BufferSubscriptCollapses ==
    ASSUME TypeOK, NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ <<HostReturnsBuffer(cId, b)>>_vars <=>
                  HostReturnsBuffer(cId, b)
           /\ <<FreeReturnedBuffer(cId, b)>>_vars <=>
                  FreeReturnedBuffer(cId, b)
<1>1. QED
    BY SMT DEF HostReturnsBuffer, FreeReturnedBuffer, IsLentBuffer,
        IsReturnedBuffer, HostHoldsSomeBuffer, vars, ffi_vars, TypeOK

LEMMA SendSubscriptCollapses ==
    ASSUME TypeOK, NEW cId \in CallIds, NEW msg \in Messages, NEW bs \in BufferIds
    PROVE  <<SendMessage(cId, msg, bs)>>_vars <=> SendMessage(cId, msg, bs)
<1>1. QED
    BY SMT DEF SendMessage, HostHoldsSomeBuffer, vars, ffi_vars, TypeOK

\* Cancellation is latched forever: no action clears it.
LEMMA CancelMonotone ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsCancelRequested(cId) /\ [Next]_vars =>
               IsCancelRequested(cId)'
<1>1. ASSUME TypeOK, IsCancelRequested(cId), [Next]_vars
      PROVE  IsCancelRequested(cId)'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
      BY <1>1, <2>1, <3>1, SMTT(45) DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        NextFail, NextExplicitStutter,
        RuntimeCreate, RuntimeBeginShutdown, EmitShutdownComplete,
        ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeRelease, RuntimeFail,
        RemainFailed, RemainReleased,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, RequestCallCancellation, ReleaseCallHandle,
        SendMessage, EndSend, EmitWriteDone, WriteDoneReturns,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, DeliveryCallbackReturns, HostConsumesEvent,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        RequestCancellationOfActiveCalls, HandPayloadToHost, HasFreeDeliverySlot, HasFreeDeliverySlotForTerminal, IsRuntimeDrained,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
        vars, ffi_vars, L0!ChannelsOf, L0!CallsOf,
        L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall,
        L0!ActiveCallStates, L0!HasStatus,
        TypeOK, L0!TypeOK
    <3>2. CASE NextSafeFfiOnly
      <4>1. CASE NextSafeShutdownFfi
        <5>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
          BY <1>1, <5>1, SMT DEF EmitShutdownComplete, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
          BY <1>1, <5>2, SMT DEF ShutdownCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>3. CASE \E rt \in RuntimeIds : RuntimeDestroy(rt)
          BY <1>1, <5>3, SMT DEF RuntimeDestroy, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>35. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
          BY <1>1, <5>35, SMT DEF EmitResourcesReleased, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>36. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
          BY <1>1, <5>36, SMT DEF ResourcesReleasedCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>4. QED BY <4>1, <5>1, <5>2, <5>3, <5>35, <5>36 DEF NextSafeShutdownFfi
      <4>2. CASE NextSafeCallFfi
        <5>1. CASE \E c \in CallIds : RequestCallCancellation(c)
          BY <1>1, <5>1, SMT DEF RequestCallCancellation, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>2. CASE \E c \in CallIds : ReleaseCallHandle(c)
          BY <1>1, <5>2, SMT DEF ReleaseCallHandle, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>3. CASE \E c \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                      LendSendBuffer(c, bb, msg, ch)
          BY <1>1, <5>3, SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>4. CASE \E c \in CallIds, bb \in BufferIds :
                      HostReturnsBuffer(c, bb)
          BY <1>1, <5>4, SMT DEF HostReturnsBuffer, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>45. CASE \E c \in CallIds, bb \in BufferIds :
                      FreeReturnedBuffer(c, bb)
          BY <1>1, <5>45, SMT DEF FreeReturnedBuffer, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>5. CASE \E c \in CallIds : EmitWriteDone(c)
          BY <1>1, <5>5, SMT DEF EmitWriteDone, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>6. CASE \E c \in CallIds : WriteDoneReturns(c)
          BY <1>1, <5>6, SMT DEF WriteDoneReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>7. CASE \E c \in CallIds : DeliveryCallbackReturns(c)
          BY <1>1, <5>7, SMT DEF DeliveryCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>8. CASE \E c \in CallIds : HostConsumesEvent(c)
          BY <1>1, <5>8, SMT DEF HostConsumesEvent, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>10. CASE \E c \in CallIds, msg \in Messages : RefuseLendTooLarge(c, msg)
          BY <1>1, <5>10, SMT DEF RefuseLendTooLarge, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>11. CASE \E c \in CallIds, msg \in Messages : RefuseLendForSlot(c, msg)
          BY <1>1, <5>11, SMT DEF RefuseLendForSlot, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>12. CASE \E c \in CallIds, msg \in Messages, ch \in Sizes : RefuseLendForBudget(c, msg, ch)
          BY <1>1, <5>12, SMT DEF RefuseLendForBudget, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>9. QED BY <4>2, <5>1, <5>2, <5>3, <5>4, <5>45, <5>5, <5>6, <5>7, <5>8, <5>10, <5>11, <5>12 DEF NextSafeCallFfi
      <4>3. QED BY <3>2, <4>1, <4>2 DEF NextSafeFfiOnly
    <3>3. CASE NextFail
      BY <1>1, <2>1, <3>3, SMTT(45) DEF NextFail, NextExplicitStutter, RuntimeFail, IsRuntimeDrained, L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars, vars, ffi_vars, TypeOK, L0!TypeOK
    <3>4. CASE NextExplicitStutter
      BY <1>1, <2>1, <3>4, SMTT(45) DEF NextFail, NextExplicitStutter, RemainFailed, RemainReleased, IsRuntimeDrained, L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars, vars, ffi_vars, TypeOK, L0!TypeOK
    <3>5. QED
      BY <2>1, <3>1, <3>2, <3>3, <3>4, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* The delivery callback stays on the host stack until DeliveryCallbackReturns,
\* and only a delivery puts it back.
LEMMA CallbackFrame ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ [Next]_vars =>
               /\ (IsDeliveryCallbackRunning(cId) /\
                       ~<<DeliveryCallbackReturns(cId)>>_vars =>
                           IsDeliveryCallbackRunning(cId)')
               /\ (~IsDeliveryCallbackRunning(cId) /\
                       ~<<DeliverInitialMetadata(cId)>>_vars /\
                       ~<<DeliverMessage(cId)>>_vars /\
                       ~<<DeliverStatus(cId)>>_vars /\
                       ~<<DeliverCancelled(cId)>>_vars =>
                           ~IsDeliveryCallbackRunning(cId)')
<1>1. ASSUME TypeOK, [Next]_vars
      PROVE  /\ (IsDeliveryCallbackRunning(cId) /\
                     ~<<DeliveryCallbackReturns(cId)>>_vars =>
                         IsDeliveryCallbackRunning(cId)')
             /\ (~IsDeliveryCallbackRunning(cId) /\
                     ~<<DeliverInitialMetadata(cId)>>_vars /\
                     ~<<DeliverMessage(cId)>>_vars /\
                     ~<<DeliverStatus(cId)>>_vars /\
                     ~<<DeliverCancelled(cId)>>_vars =>
                         ~IsDeliveryCallbackRunning(cId)')
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
      BY <1>1, <2>1, <3>1, SMTT(45) DEF NextSafeRefining, NextSafeRuntimeOnly, NextSafeRuntimeChannel,
        NextSafeChannelOnly, NextSafeChannelCall, NextSafeCallOnly,
        NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        NextFail, NextExplicitStutter,
        RuntimeCreate, RuntimeBeginShutdown, EmitShutdownComplete,
        ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeRelease, RuntimeFail,
        RemainFailed, RemainReleased,
        ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
        CallStart, RequestCallCancellation, ReleaseCallHandle,
        SendMessage, EndSend, EmitWriteDone, WriteDoneReturns,
        NetworkSend, NetworkReceive, ReceiveStatus,
        DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, DeliveryCallbackReturns, HostConsumesEvent,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        RequestCancellationOfActiveCalls, HandPayloadToHost, HasFreeDeliverySlot, HasFreeDeliverySlotForTerminal, IsRuntimeDrained,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
        vars, ffi_vars, L0!ChannelsOf, L0!CallsOf,
        L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall,
        L0!ActiveCallStates, L0!HasStatus,
        TypeOK, L0!TypeOK
    <3>2. CASE NextSafeFfiOnly
      <4>1. CASE NextSafeShutdownFfi
        <5>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
          BY <1>1, <5>1, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF EmitShutdownComplete, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
          BY <1>1, <5>2, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF ShutdownCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>3. CASE \E rt \in RuntimeIds : RuntimeDestroy(rt)
          BY <1>1, <5>3, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF RuntimeDestroy, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>35. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
          BY <1>1, <5>35, SMT DEF EmitResourcesReleased, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>36. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
          BY <1>1, <5>36, SMT DEF ResourcesReleasedCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>4. QED BY <4>1, <5>1, <5>2, <5>3, <5>35, <5>36 DEF NextSafeShutdownFfi
      <4>2. CASE NextSafeCallFfi
        <5>1. CASE \E c \in CallIds : RequestCallCancellation(c)
          BY <1>1, <5>1, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF RequestCallCancellation, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>2. CASE \E c \in CallIds : ReleaseCallHandle(c)
          BY <1>1, <5>2, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF ReleaseCallHandle, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>3. CASE \E c \in CallIds, bb \in BufferIds, msg \in Messages, ch \in Sizes :
                      LendSendBuffer(c, bb, msg, ch)
          BY <1>1, <5>3, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>4. CASE \E c \in CallIds, bb \in BufferIds :
                      HostReturnsBuffer(c, bb)
          BY <1>1, <5>4, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF HostReturnsBuffer, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>45. CASE \E c \in CallIds, bb \in BufferIds :
                      FreeReturnedBuffer(c, bb)
          BY <1>1, <5>45, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF FreeReturnedBuffer, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>5. CASE \E c \in CallIds : EmitWriteDone(c)
          BY <1>1, <5>5, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF EmitWriteDone, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>6. CASE \E c \in CallIds : WriteDoneReturns(c)
          BY <1>1, <5>6, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF WriteDoneReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>7. CASE \E c \in CallIds : DeliveryCallbackReturns(c)
          BY <1>1, <5>7, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF DeliveryCallbackReturns, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>8. CASE \E c \in CallIds : HostConsumesEvent(c)
          BY <1>1, <5>8, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF HostConsumesEvent, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>10. CASE \E c \in CallIds, msg \in Messages : RefuseLendTooLarge(c, msg)
          BY <1>1, <5>10, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF RefuseLendTooLarge, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>11. CASE \E c \in CallIds, msg \in Messages : RefuseLendForSlot(c, msg)
          BY <1>1, <5>11, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF RefuseLendForSlot, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>12. CASE \E c \in CallIds, msg \in Messages, ch \in Sizes : RefuseLendForBudget(c, msg, ch)
          BY <1>1, <5>12, SubscriptCollapses, DeliverySubscriptCollapses, SMT DEF RefuseLendForBudget, IsCancelRequested, IsHandleReleased, IsDeliveryCallbackRunning, IsWriteDoneCallbackRunning, IsAwaitingWriteDone, HasNoSendInFlight, WriteDonesReturned, HostOwnsNoPayload, HostOwnsSomePayload, OwedPayloads, HostHoldsNoBuffer, HostHoldsSomeBuffer, HasNoDeliveredEvents, IsClosingChannel, IsRuntimeDestroyed, SendWindowOccupancy, ffi_vars, TypeOK, L0!TypeOK
        <5>9. QED BY <4>2, <5>1, <5>2, <5>3, <5>4, <5>45, <5>5, <5>6, <5>7, <5>8, <5>10, <5>11, <5>12 DEF NextSafeCallFfi
      <4>3. QED BY <3>2, <4>1, <4>2 DEF NextSafeFfiOnly
    <3>3. CASE NextFail
      BY <1>1, <2>1, <3>3, SMTT(45) DEF NextFail, NextExplicitStutter, RuntimeFail, IsRuntimeDrained, L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars, vars, ffi_vars, TypeOK, L0!TypeOK
    <3>4. CASE NextExplicitStutter
      BY <1>1, <2>1, <3>4, SMTT(45) DEF NextFail, NextExplicitStutter, RemainFailed, RemainReleased, IsRuntimeDrained, L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars, vars, ffi_vars, TypeOK, L0!TypeOK
    <3>5. QED
      BY <2>1, <3>1, <3>2, <3>3, <3>4, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* The send-side frame: an owed WRITE_DONE stays owed until EmitWriteDone,
\* the acquittal callback stays on the stack until WriteDoneReturns, and a
\* drained send side stays drained while no new send is accepted.
\* The subscripts go first, once, so the case analysis below never has
\* to reason about inequality of the state tuple.
LEMMA SlotFrame ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ [Next]_vars =>
               /\ (IsAwaitingWriteDone(cId) /\
                       ~IsWriteDoneCallbackRunning(cId) /\
                       ~<<EmitWriteDone(cId)>>_vars =>
                           (IsAwaitingWriteDone(cId) /\
                                ~IsWriteDoneCallbackRunning(cId))')
               /\ (IsWriteDoneCallbackRunning(cId) /\
                       ~<<WriteDoneReturns(cId)>>_vars =>
                           (IsWriteDoneCallbackRunning(cId))')
               /\ (HasNoSendInFlight(cId) /\
                       (\A msg \in Messages, bs \in BufferIds :
                            ~<<SendMessage(cId, msg, bs)>>_vars) =>
                           (HasNoSendInFlight(cId))')
<1>0. ASSUME TypeOK,
             /\ (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId) /\
                     ~EmitWriteDone(cId) =>
                         (IsAwaitingWriteDone(cId) /\
                              ~IsWriteDoneCallbackRunning(cId))')
             /\ (IsWriteDoneCallbackRunning(cId) /\
                     ~WriteDoneReturns(cId) =>
                         (IsWriteDoneCallbackRunning(cId))')
             /\ (HasNoSendInFlight(cId) /\
                     (\A msg \in Messages, bs \in BufferIds :
                          ~SendMessage(cId, msg, bs)) =>
                         (HasNoSendInFlight(cId))')
      PROVE  /\ (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId) /\
                     ~<<EmitWriteDone(cId)>>_vars =>
                         (IsAwaitingWriteDone(cId) /\
                              ~IsWriteDoneCallbackRunning(cId))')
             /\ (IsWriteDoneCallbackRunning(cId) /\
                     ~<<WriteDoneReturns(cId)>>_vars =>
                         (IsWriteDoneCallbackRunning(cId))')
             /\ (HasNoSendInFlight(cId) /\
                     (\A msg \in Messages, bs \in BufferIds :
                          ~<<SendMessage(cId, msg, bs)>>_vars) =>
                         (HasNoSendInFlight(cId))')
  <2>1. <<EmitWriteDone(cId)>>_vars <=> EmitWriteDone(cId)
    BY <1>0, SubscriptCollapses
  <2>2. <<WriteDoneReturns(cId)>>_vars <=> WriteDoneReturns(cId)
    BY <1>0, SubscriptCollapses
  <2>3. (\A msg \in Messages, bs \in BufferIds : ~<<SendMessage(cId, msg, bs)>>_vars) <=>
            (\A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs))
    <3>1. ASSUME NEW msg \in Messages, NEW bs \in BufferIds
          PROVE  <<SendMessage(cId, msg, bs)>>_vars <=> SendMessage(cId, msg, bs)
      BY <1>0, SendSubscriptCollapses
    <3>2. QED
      BY <3>1
  <2>4. QED
    BY <1>0, <2>1, <2>2, <2>3
<1>1. ASSUME TypeOK, [Next]_vars
      PROVE  /\ (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId) /\
                     ~EmitWriteDone(cId) =>
                         (IsAwaitingWriteDone(cId) /\
                              ~IsWriteDoneCallbackRunning(cId))')
             /\ (IsWriteDoneCallbackRunning(cId) /\
                     ~WriteDoneReturns(cId) =>
                         (IsWriteDoneCallbackRunning(cId))')
             /\ (HasNoSendInFlight(cId) /\
                     (\A msg \in Messages, bs \in BufferIds :
                          ~SendMessage(cId, msg, bs)) =>
                         (HasNoSendInFlight(cId))')
  <2>0. /\ write_dones_emitted \in [CallIds -> Nat]
        /\ write_done_callback_running \in [CallIds -> BOOLEAN]
        /\ submitted \in [CallIds -> Seq(Messages)]
    BY <1>1, Zenon DEF TypeOK, L0!TypeOK
\* The acquittal count of one call moves only when that call emits.
  <2>1. ASSUME ~EmitWriteDone(cId)
        PROVE  (write_dones_emitted[cId])' = write_dones_emitted[cId]
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <2>1, <3>1, SMT DEF EmitWriteDone
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherEmitsOrKeepsWriteDones, Zenon
\* Its flag goes up only on its own emission and down only on its own
\* return, so both directions are decided by one action each.
  <2>2. ASSUME ~EmitWriteDone(cId), ~write_done_callback_running[cId]
        PROVE  ~((write_done_callback_running[cId])')
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <2>2, <3>1, SMT DEF EmitWriteDone
    <3>2. CASE \E c \in CallIds : WriteDoneReturns(c)
      BY <1>1, <2>0, <2>2, <3>2, SMT DEF WriteDoneReturns
    <3>3. QED
      BY <1>1, <2>2, <3>1, <3>2,
         EveryStepEitherAcquitsOrKeepsWriteDoneFlag, Zenon
  <2>3. ASSUME ~WriteDoneReturns(cId), write_done_callback_running[cId]
        PROVE  (write_done_callback_running[cId])'
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <2>3, <3>1, SMT DEF EmitWriteDone
    <3>2. CASE \E c \in CallIds : WriteDoneReturns(c)
      BY <1>1, <2>0, <2>3, <3>2, SMT DEF WriteDoneReturns
    <3>3. QED
      BY <1>1, <2>3, <3>1, <3>2,
         EveryStepEitherAcquitsOrKeepsWriteDoneFlag, Zenon
\* A call's submitted sequence only grows, and only when it sends.
  <2>4. /\ (submitted[cId])' \in Seq(Messages)
        /\ Len((submitted[cId])') >= Len(submitted[cId])
    <3>0. /\ submitted[cId] \in Seq(Messages)
          /\ Len(submitted[cId]) \in Nat
      BY <2>0, LenProperties, Zenon
    <3>1. CASE \E c \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(c, m, bs)
      <4>1. PICK c \in CallIds, m \in Messages, bs \in BufferIds :
              SendMessage(c, m, bs)
        BY <3>1
      <4>2. submitted' = [submitted EXCEPT ![c] = Append(@, m)]
        BY <4>1, Zenon DEF SendMessage, L0!SendMessage
      <4>3. CASE c = cId
        <5>1. (submitted[cId])' = Append(submitted[cId], m)
          BY <2>0, <4>2, <4>3, Zenon
        <5>2. /\ Append(submitted[cId], m) \in Seq(Messages)
              /\ Len(Append(submitted[cId], m)) = Len(submitted[cId]) + 1
          BY <3>0, AppendProperties
        <5>3. QED
          BY <3>0, <5>1, <5>2, SMT
      <4>4. CASE c # cId
        <5>1. (submitted[cId])' = submitted[cId]
          BY <2>0, <4>2, <4>4, Zenon
        <5>2. QED
          BY <3>0, <5>1, SMT
      <4>5. QED BY <4>3, <4>4
    <3>2. CASE UNCHANGED submitted
      <4>1. (submitted[cId])' = submitted[cId]
        BY <3>2, Zenon
      <4>2. QED
        BY <3>0, <4>1, SMT
    <3>3. QED
      BY <1>1, <3>1, <3>2, EveryStepEitherSubmitsOrKeepsSubmitted
  <2>5. ASSUME \A msg \in Messages, bs \in BufferIds :
                   ~SendMessage(cId, msg, bs)
        PROVE  Len((submitted[cId])') = Len(submitted[cId])
    <3>1. CASE \E c \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(c, m, bs)
      <4>1. PICK c \in CallIds, m \in Messages, bs \in BufferIds :
              SendMessage(c, m, bs)
        BY <3>1
      <4>2. c # cId
        BY <2>5, <4>1
      <4>3. submitted' = [submitted EXCEPT ![c] = Append(@, m)]
        BY <4>1, Zenon DEF SendMessage, L0!SendMessage
      <4>4. QED BY <2>0, <4>2, <4>3, Zenon
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherSubmitsOrKeepsSubmitted, SMT
  <2>6. ASSUME IsAwaitingWriteDone(cId), ~IsWriteDoneCallbackRunning(cId),
               ~EmitWriteDone(cId)
        PROVE  (IsAwaitingWriteDone(cId) /\
                    ~IsWriteDoneCallbackRunning(cId))'
    <3>1. (write_dones_emitted[cId])' < Len((submitted[cId])')
      <4>1. (write_dones_emitted[cId])' = write_dones_emitted[cId]
        BY <2>1, <2>6
      <4>2. write_dones_emitted[cId] < Len(submitted[cId])
        BY <2>6, Zenon DEF IsAwaitingWriteDone
      <4>3. /\ write_dones_emitted[cId] \in Nat
            /\ Len(submitted[cId]) \in Nat
            /\ Len((submitted[cId])') \in Nat
        BY <2>0, <2>4, LenProperties, Zenon
      <4>4. QED
        BY <2>4, <4>1, <4>2, <4>3, SMT
    <3>2. ~((write_done_callback_running[cId])')
      BY <2>2, <2>6, Zenon DEF IsWriteDoneCallbackRunning
    <3>3. QED
      BY <3>1, <3>2, Zenon
      DEF IsAwaitingWriteDone, IsWriteDoneCallbackRunning
  <2>7. IsWriteDoneCallbackRunning(cId) /\ ~WriteDoneReturns(cId) =>
            (IsWriteDoneCallbackRunning(cId))'
    BY <2>3, Zenon DEF IsWriteDoneCallbackRunning
\* Nothing can emit on a drained call - emission needs an unacquitted
\* send - and nothing can return, since no callback is running.
  <2>8. HasNoSendInFlight(cId) /\
            (\A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs)) =>
                (HasNoSendInFlight(cId))'
    <3>1. SUFFICES ASSUME HasNoSendInFlight(cId),
                          \A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs)
                             PROVE  (HasNoSendInFlight(cId))'
      OBVIOUS
    <3>2. ~EmitWriteDone(cId)
      BY <3>1, SMT DEF EmitWriteDone, HasNoSendInFlight,
          IsAwaitingWriteDone
    <3>3. ~write_done_callback_running[cId]
      BY <3>1, Zenon DEF HasNoSendInFlight
    <3>4. QED
      BY <2>0, <2>1, <2>2, <2>5, <3>1, <3>2, <3>3, SMT
      DEF HasNoSendInFlight
  <2>9. QED
    BY <2>6, <2>7, <2>8, Zenon
<1>2. QED BY <1>0, <1>1

\* Enabledness of the four binding-owned drain actions is their guard.
LEMMA DeliveryCallbackReturnsEnabled ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsDeliveryCallbackRunning(cId) =>
               ENABLED <<DeliveryCallbackReturns(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF DeliveryCallbackReturns, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars

LEMMA EmitWriteDoneEnabled ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsAwaitingWriteDone(cId) /\
               ~IsWriteDoneCallbackRunning(cId) =>
               ENABLED <<EmitWriteDone(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF EmitWriteDone, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars

LEMMA WriteDoneReturnsEnabled ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
               ENABLED <<WriteDoneReturns(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF WriteDoneReturns, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars

LEMMA HostConsumesEventEnabled ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  TypeOK /\ HostOwnsPayload(cId, k) =>
               ENABLED <<HostConsumesEvent(cId)>>_vars
<1>1. SUFFICES ASSUME TypeOK, HostOwnsPayload(cId, k)
               PROVE  ENABLED <<HostConsumesEvent(cId)>>_vars
    OBVIOUS
\* The length is typed explicitly: leaving the solver to work it out from the
\* sequence type put this step on the edge of its budget.
<1>15. /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
       /\ payloads_consumed_by_host \in [CallIds -> Nat]
       /\ Len(events_delivered[cId]) \in Nat
    BY <1>1, LenProperties, Zenon DEF TypeOK, L0!TypeOK
<1>2. HostOwnsSomePayload(cId)
    BY <1>1, <1>15, SMT
    DEF HostOwnsPayload, HostOwnsSomePayload, OwedPayloads
<1>3. QED
    BY <1>1, <1>2, ExpandENABLED, SMT
    DEF HostConsumesEvent, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars

\* What each drain action does to its own variables: the acquittal
\* callback toggles, and only WriteDoneReturns advances the returned
\* count (EmitWriteDone trades one owed emission for the callback).
LEMMA DrainStepEffects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK =>
               /\ (<<DeliveryCallbackReturns(cId)>>_vars =>
                       ~IsDeliveryCallbackRunning(cId)')
               /\ (<<EmitWriteDone(cId)>>_vars =>
                       /\ (IsWriteDoneCallbackRunning(cId))'
                       /\ (WriteDonesReturned(cId))' =
                              WriteDonesReturned(cId))
               /\ (<<WriteDoneReturns(cId)>>_vars =>
                       /\ ~(IsWriteDoneCallbackRunning(cId))'
                       /\ (WriteDonesReturned(cId))' =
                              WriteDonesReturned(cId) + 1)
<1>1. QED
    BY SMT DEF DeliveryCallbackReturns, EmitWriteDone, WriteDoneReturns,
        TypeOK, L0!TypeOK, vars, l0_vars, L0!vars, ffi_vars

(***************************************************************************)
(* THE DELIVER-STATUS LIFT                                                 *)
(* The level-0 action kills its own guard when it fires, so persistent    *)
(* level-0 enabledness is contradictory once the FFI drains run: the WF   *)
(* lifts by absurdity.  StatusReady names the level-0 guard.              *)
(***************************************************************************)

StatusReady(cId) ==
    /\ L0!IsActiveCall(cId)
    /\ status_pending[cId]
    /\ Len(events_delivered[cId]) >= 1
    /\ events_delivered[cId][1] = "INITIAL_METADATA"
    /\ ~L0!HasStatus(cId)
    /\ Len(delivered[cId]) = Len(received[cId])

LEMMA IndInvParts ==
    IndInv => TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits
<1>1. QED
    BY DEF IndInv, FfiCallInv

LEMMA DS0EnabledIsReady ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ ENABLED <<L0!DeliverStatus(cId)>>_l0_vars =>
               StatusReady(cId)
<1>1. QED
    BY ExpandENABLED, SMT
    DEF L0!DeliverStatus, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DS1EnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ ActiveCallPayloadsWithinCredits
           /\ StatusReady(cId)
           /\ ~IsDeliveryCallbackRunning(cId)
           /\ HasNoSendInFlight(cId)
           /\ ~IsCancelRequested(cId)
           => ENABLED <<DeliverStatus(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        ActiveCallPayloadsWithinCredits, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DC1EnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ ActiveCallPayloadsWithinCredits
           /\ L0!IsActiveCall(cId)
           /\ ~L0!HasStatus(cId)
           /\ ~IsDeliveryCallbackRunning(cId)
           /\ HasNoSendInFlight(cId)
           /\ IsCancelRequested(cId)
           => ENABLED <<DeliverCancelled(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        ActiveCallPayloadsWithinCredits, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DS1StepProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverStatus(cId)>>_vars =>
               <<L0!DeliverStatus(cId)>>_l0_vars
<1>1. QED
    BY SMT DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DS1Kills ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverStatus(cId)>>_vars =>
               (~StatusReady(cId))'
<1>1. QED
    BY SMT DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DC1Kills ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
               (~StatusReady(cId))'
<1>1. QED
    BY SMT DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

\* Only the two terminals of the call touch the guard.
\* The subscripts go once, up front: the cases below then reason about
\* the actions themselves rather than about the state tuple.
LEMMA StatusReadyStable ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ StatusReady(cId)
           /\ [Next]_vars
           /\ ~<<DeliverStatus(cId)>>_vars
           /\ ~<<DeliverCancelled(cId)>>_vars
           => StatusReady(cId)'
<1>1. ASSUME TypeOK, StatusReady(cId), [Next]_vars,
             ~DeliverStatus(cId),
             ~DeliverCancelled(cId)
      PROVE  StatusReady(cId)'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
\* Runtime and channel steps do not touch a call at all.
      <4>1. CASE \/ NextSafeRuntimeOnly
                 \/ NextSafeRuntimeChannel
                 \/ NextSafeChannelOnly
        <5>1. UNCHANGED L0!CallVars
          BY <4>1, RuntimeAndChannelStepsKeepCalls
        <5>2. QED
          BY <1>1, <5>1, SMT
          DEF L0!CallVars, StatusReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!IsUnusedCall,
              L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
      <4>2. CASE NextSafeChannelCall
        BY <1>1, <4>2, SMTT(45)
        DEF NextSafeChannelCall, ChannelFinishClosing,
            L0!ChannelFinishClosing, L0!ChannelsOf, L0!CallsOf,
            RequestCancellationOfActiveCalls, L0!RuntimeVars,
            L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
            StatusReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!IsUnusedCall,
              L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
      <4>3. CASE NextSafeCallOnly
        BY <1>1, <4>3, SMTT(45)
        DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
            NetworkSend, NetworkReceive, ReceiveStatus,
            DeliverInitialMetadata, DeliverMessage, DeliverStatus,
            DeliverCancelled, L0!CallStart, L0!SendMessage, L0!EndSend,
            L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
            L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
            L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
            L0!CallVars, L0!vars, l0_vars,
            StatusReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!IsUnusedCall,
              L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
      <4>4. QED BY <3>1, <4>1, <4>2, <4>3 DEF NextSafeRefining
    <3>2. CASE NextSafeFfiOnly
\* Nothing an FFI-only step writes is read here.
      <4>1. UNCHANGED l0_vars
        BY <3>2, FfiOnlyStepsKeepL0
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
            TypeOK, L0!TypeOK, L0!IsActiveCall, L0!ActiveCallStates,
            L0!HasStatus, L0!StatusKinds, L0!EventKinds,
            L0!IsUnusedCall, L0!IsTerminalCall, IsClosingChannel, StatusReady
    <3>3. CASE NextFail \/ NextExplicitStutter
      <4>1. UNCHANGED L0!CallVars
        BY <3>3, FailAndStutterStepsKeepCalls
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF L0!CallVars, StatusReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
            L0!ActiveCallStates, L0!HasStatus, L0!IsUnusedCall,
            L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, StatusReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1, DeliverySubscriptCollapses, Zenon

\* While the trailers are latched, no new send can take the slot and no
\* other delivery can take the callback.
\* Two independent claims, so two proofs: the send side is drained
\* because no send can start on a call with its trailers latched, and the
\* delivery side stays quiet because every delivery that could start one
\* is disabled - the metadata is already out, the messages are all
\* delivered, and the two terminals are the hypothesis.
LEMMA StatusReadyDrainStable ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ (TypeOK /\ StatusReady(cId) /\ HasNoSendInFlight(cId) /\
                   [Next]_vars => (HasNoSendInFlight(cId))')
           /\ (TypeOK /\ StatusReady(cId) /\
                   ~IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
                   ~<<DeliverStatus(cId)>>_vars /\
                   ~<<DeliverCancelled(cId)>>_vars =>
                       ~IsDeliveryCallbackRunning(cId)')
<1>1. ASSUME TypeOK, [Next]_vars, StatusReady(cId), HasNoSendInFlight(cId)
      PROVE  (HasNoSendInFlight(cId))'
  <2>0. /\ write_dones_emitted \in [CallIds -> Nat]
        /\ write_done_callback_running \in [CallIds -> BOOLEAN]
        /\ submitted \in [CallIds -> Seq(Messages)]
    BY <1>1, Zenon DEF TypeOK, L0!TypeOK
\* status_pending is exactly what the level-0 send guard refuses.
  <2>1. \A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs)
    BY <1>1, Zenon DEF SendMessage, L0!SendMessage, StatusReady
  <2>2. ~EmitWriteDone(cId)
    BY <1>1, SMT DEF EmitWriteDone, HasNoSendInFlight, IsAwaitingWriteDone
  <2>3. (submitted[cId])' = submitted[cId]
    <3>1. CASE \E c \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(c, m, bs)
      BY <1>1, <2>0, <2>1, <3>1, SMT DEF SendMessage, L0!SendMessage
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherSubmitsOrKeepsSubmitted, Zenon
  <2>4. (write_dones_emitted[cId])' = write_dones_emitted[cId]
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <2>2, <3>1, SMT DEF EmitWriteDone
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherEmitsOrKeepsWriteDones, Zenon
  <2>5. ~((write_done_callback_running[cId])')
    <3>1. ~write_done_callback_running[cId]
      BY <1>1, Zenon DEF HasNoSendInFlight
    <3>2. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <2>2, <3>1, <3>2, SMT DEF EmitWriteDone
    <3>3. CASE \E c \in CallIds : WriteDoneReturns(c)
      BY <1>1, <2>0, <3>1, <3>3, SMT DEF WriteDoneReturns
    <3>4. QED
      BY <1>1, <3>1, <3>2, <3>3,
         EveryStepEitherAcquitsOrKeepsWriteDoneFlag, Zenon
  <2>6. QED
    BY <1>1, <2>3, <2>4, <2>5, Zenon DEF HasNoSendInFlight
<1>2. ASSUME TypeOK, [Next]_vars, StatusReady(cId),
             ~IsDeliveryCallbackRunning(cId),
             ~DeliverStatus(cId), ~DeliverCancelled(cId)
      PROVE  ~((IsDeliveryCallbackRunning(cId))')
\* The head is already out and every message is delivered, so neither
\* non-terminal delivery is enabled either.
  <2>1. ~DeliverInitialMetadata(cId)
    BY <1>2, SMT
    DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, StatusReady
  <2>2. ~DeliverMessage(cId)
    BY <1>2, SMT DEF DeliverMessage, L0!DeliverMessage, StatusReady
  <2>3. CASE Next
    <3>1. CASE \/ NextSafeRuntimeOnly
               \/ NextSafeRuntimeChannel
               \/ NextSafeChannelOnly
               \/ NextSafeChannelCall
      <4>1. UNCHANGED delivery_callback_running
        BY <3>1, RuntimeAndChannelStepsKeepDeliveryFlags
      <4>2. QED
        BY <1>2, <4>1, Zenon DEF IsDeliveryCallbackRunning
    <3>2. CASE NextSafeCallOnly
\* Only the four deliveries put a callback on the stack, and all four
\* are excluded for this call, so whichever one fires names another.
      <4>1. CASE \/ \E c \in CallIds, ch \in ChannelIds : CallStart(c, ch)
                 \/ \E c \in CallIds, m \in Messages,
                       bs \in BufferIds : SendMessage(c, m, bs)
                 \/ \E c \in CallIds : EndSend(c)
                 \/ \E c \in CallIds : NetworkSend(c)
                 \/ \E c \in CallIds, m \in Messages : NetworkReceive(c, m)
                 \/ \E c \in CallIds : ReceiveStatus(c)
        <5>1. UNCHANGED delivery_callback_running
          BY <4>1, SMT
          DEF CallStart, SendMessage, EndSend, NetworkSend,
              NetworkReceive, ReceiveStatus, ffi_vars
        <5>2. QED
          BY <1>2, <5>1, Zenon DEF IsDeliveryCallbackRunning
      <4>2. CASE \E c \in CallIds : DeliverInitialMetadata(c)
        BY <1>2, <2>1, <2>2, <4>2, SMT
        DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, IsDeliveryCallbackRunning,
            TypeOK, L0!TypeOK
      <4>3. CASE \E c \in CallIds : DeliverMessage(c)
        BY <1>2, <2>1, <2>2, <4>3, SMT
        DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, IsDeliveryCallbackRunning,
            TypeOK, L0!TypeOK
      <4>4. CASE \E c \in CallIds : DeliverStatus(c)
        BY <1>2, <2>1, <2>2, <4>4, SMT
        DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, IsDeliveryCallbackRunning,
            TypeOK, L0!TypeOK
      <4>5. CASE \E c \in CallIds : DeliverCancelled(c)
        BY <1>2, <2>1, <2>2, <4>5, SMT
        DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, IsDeliveryCallbackRunning,
            TypeOK, L0!TypeOK
      <4>6. QED
        BY <3>2, <4>1, <4>2, <4>3, <4>4, <4>5 DEF NextSafeCallOnly
    <3>3. CASE NextSafeFfiOnly
      BY <1>2, <3>3, FfiOnlyStepsNeverStartDelivery
    <3>4. CASE NextFail \/ NextExplicitStutter
      <4>1. UNCHANGED ffi_vars
        BY <3>4, SMT
        DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
            RemainReleased
      <4>2. QED
        BY <1>2, <4>1, SMT DEF ffi_vars, IsDeliveryCallbackRunning
    <3>5. QED
      BY <2>3, <3>1, <3>2, <3>3, <3>4, NextDecomposition
      DEF NextByFootprint, NextSafe, NextSafeRefining
  <2>4. CASE UNCHANGED vars
    BY <1>2, <2>4, SMT
    DEF vars, ffi_vars, IsDeliveryCallbackRunning
  <2>5. QED BY <1>2, <2>3, <2>4
<1>3. QED
  BY <1>1, <1>2, DeliverySubscriptCollapses, Zenon

NoSendsTo(cId) ==
    \A msg \in Messages, bs \in BufferIds : ~<<SendMessage(cId, msg, bs)>>_vars

NoDeliveriesTo(cId) ==
    /\ ~<<DeliverInitialMetadata(cId)>>_vars
    /\ ~<<DeliverMessage(cId)>>_vars
    /\ ~<<DeliverStatus(cId)>>_vars
    /\ ~<<DeliverCancelled(cId)>>_vars

\* The two freeze sources: a latched cancellation refuses every send, and
\* latched trailers refuse it through the level-0 guard.
LEMMA NoSendUnderCancel ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId)
<1>1. QED
    BY SMT DEF NoSendsTo, SendMessage, TypeOK, L0!TypeOK,
        vars, l0_vars, L0!vars, ffi_vars

LEMMA NoSendUnderStatusReady ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ StatusReady(cId) => NoSendsTo(cId)
<1>1. QED
    BY SMT DEF NoSendsTo, SendMessage, L0!SendMessage, StatusReady,
        TypeOK, L0!TypeOK, vars, l0_vars, L0!vars, ffi_vars,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus

\* The send-drain measure: two per owed WRITE_DONE, one for the running
\* callback.  Zero exactly when the send side is drained.
SendDrainMeasure(cId) ==
    2 * (Len(submitted[cId]) - write_dones_emitted[cId]) +
        (IF write_done_callback_running[cId] THEN 1 ELSE 0)

\* Arithmetic bridges of the frozen send drain.
LEMMA SendQuietBridges ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ (TypeOK /\ WriteDonesNeverExceedSends /\
                   SendDrainMeasure(cId) <= 0 => HasNoSendInFlight(cId))
           /\ (TypeOK /\ WriteDonesNeverExceedSends /\
                   ~IsWriteDoneCallbackRunning(cId) /\
                   SendDrainMeasure(cId) > 0 => IsAwaitingWriteDone(cId))
           /\ (TypeOK /\ FfiCallInv =>
                   SendDrainMeasure(cId) <= 2 * MaxSendsInFlight + 1)
<1>1. QED
    BY MaxSendsInFlightIsPositive, SMT
    DEF SendDrainMeasure, TypeOK, L0!TypeOK, FfiCallInv,
        WriteDonesNeverExceedSends, SendsInFlightWithinLimit,
        RunningWriteDoneWasEmitted

\* Both acquittal moves decrement the measure by one.
LEMMA SendQuietEffects ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ (TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
                   <<EmitWriteDone(cId)>>_vars =>
                       (SendDrainMeasure(cId) = n)')
           /\ (TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
                   <<WriteDoneReturns(cId)>>_vars =>
                       (SendDrainMeasure(cId) = n)')
<1>1. QED
    BY SMT DEF SendDrainMeasure, EmitWriteDone, WriteDoneReturns,
        TypeOK, L0!TypeOK, vars, l0_vars, L0!vars, ffi_vars

\* Under the freeze nothing else moves the measure.
\* The three send-side variables move only under the three actions the
\* hypotheses exclude, so the measure is frozen without ever looking at
\* the rest of Next.
LEMMA SendQuietFrame ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ TypeOK
           /\ SendDrainMeasure(cId) = n + 1
           /\ [Next]_vars
           /\ NoSendsTo(cId)
           /\ ~<<EmitWriteDone(cId)>>_vars
           /\ ~<<WriteDoneReturns(cId)>>_vars
           => (SendDrainMeasure(cId) = n + 1)'
<1>1. ASSUME TypeOK, SendDrainMeasure(cId) = n + 1, [Next]_vars,
             \A msg \in Messages, bs \in BufferIds :
                 ~SendMessage(cId, msg, bs),
             ~EmitWriteDone(cId),
             ~WriteDoneReturns(cId)
      PROVE  (SendDrainMeasure(cId) = n + 1)'
  <2>0. /\ write_dones_emitted \in [CallIds -> Nat]
        /\ write_done_callback_running \in [CallIds -> BOOLEAN]
        /\ submitted \in [CallIds -> Seq(Messages)]
    BY <1>1, Zenon DEF TypeOK, L0!TypeOK
  <2>1. (submitted[cId])' = submitted[cId]
    <3>1. CASE \E c \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(c, m, bs)
      BY <1>1, <2>0, <3>1, SMT DEF SendMessage, L0!SendMessage
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherSubmitsOrKeepsSubmitted, Zenon
  <2>2. (write_dones_emitted[cId])' = write_dones_emitted[cId]
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <3>1, SMT DEF EmitWriteDone
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherEmitsOrKeepsWriteDones, Zenon
  <2>3. (write_done_callback_running[cId])' = write_done_callback_running[cId]
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <2>0, <3>1, SMT DEF EmitWriteDone
    <3>2. CASE \E c \in CallIds : WriteDoneReturns(c)
      BY <1>1, <2>0, <3>2, SMT DEF WriteDoneReturns
    <3>3. QED
      BY <1>1, <3>1, <3>2,
         EveryStepEitherAcquitsOrKeepsWriteDoneFlag, Zenon
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, Zenon DEF SendDrainMeasure
<1>2. ASSUME TypeOK, NoSendsTo(cId)
      PROVE  \A msg \in Messages, bs \in BufferIds : ~SendMessage(cId, msg, bs)
  <2>1. ASSUME NEW msg \in Messages, NEW bs \in BufferIds
        PROVE  <<SendMessage(cId, msg, bs)>>_vars <=> SendMessage(cId, msg, bs)
    BY <1>2, SendSubscriptCollapses
  <2>2. QED
    BY <1>2, <2>1 DEF NoSendsTo
<1>3. QED
  BY <1>1, <1>2, SubscriptCollapses, Zenon

\* One frozen rung: the measure drops by one.
THEOREM SendQuietDescentFor ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ [](NoSendsTo(cId))
           => ((SendDrainMeasure(cId) = n + 1) ~>
                   (SendDrainMeasure(cId) = n))
<1>100. TypeOK /\ WriteDonesNeverExceedSends /\
            ~IsWriteDoneCallbackRunning(cId) /\
            SendDrainMeasure(cId) > 0 => IsAwaitingWriteDone(cId)
    BY SendQuietBridges, Zenon
<1>101. TypeOK /\ WriteDonesNeverExceedSends /\
            ~IsWriteDoneCallbackRunning(cId) /\
            SendDrainMeasure(cId) = n + 1 => IsAwaitingWriteDone(cId)
    BY <1>100, SMT
<1>10. [](TypeOK /\ WriteDonesNeverExceedSends /\
              ~IsWriteDoneCallbackRunning(cId) /\
              SendDrainMeasure(cId) = n + 1 => IsAwaitingWriteDone(cId))
    BY <1>101, PTL
<1>110. TypeOK /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) =>
                ENABLED <<EmitWriteDone(cId)>>_vars
    BY EmitWriteDoneEnabled, Zenon
<1>11. [](TypeOK /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) =>
                  ENABLED <<EmitWriteDone(cId)>>_vars)
    BY <1>110, PTL
<1>120. TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
            ENABLED <<WriteDoneReturns(cId)>>_vars
    BY WriteDoneReturnsEnabled, Zenon
<1>12. [](TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
              ENABLED <<WriteDoneReturns(cId)>>_vars)
    BY <1>120, PTL
<1>130. TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
            <<EmitWriteDone(cId)>>_vars => (SendDrainMeasure(cId) = n)'
    BY SendQuietEffects, Zenon
<1>13. [](TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
              <<EmitWriteDone(cId)>>_vars => (SendDrainMeasure(cId) = n)')
    BY <1>130, PTL
<1>140. TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
            <<WriteDoneReturns(cId)>>_vars => (SendDrainMeasure(cId) = n)'
    BY SendQuietEffects, Zenon
<1>14. [](TypeOK /\ SendDrainMeasure(cId) = n + 1 /\
              <<WriteDoneReturns(cId)>>_vars =>
                  (SendDrainMeasure(cId) = n)')
    BY <1>140, PTL
<1>150. TypeOK /\ SendDrainMeasure(cId) = n + 1 /\ [Next]_vars /\
            NoSendsTo(cId) /\
            ~<<EmitWriteDone(cId)>>_vars /\
            ~<<WriteDoneReturns(cId)>>_vars =>
                (SendDrainMeasure(cId) = n + 1)'
    BY SendQuietFrame, Zenon
<1>15. [](TypeOK /\ SendDrainMeasure(cId) = n + 1 /\ [Next]_vars /\
              NoSendsTo(cId) /\
              ~<<EmitWriteDone(cId)>>_vars /\
              ~<<WriteDoneReturns(cId)>>_vars =>
                  (SendDrainMeasure(cId) = n + 1)')
    BY <1>150, PTL
<1>160. TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) /\
            ~<<EmitWriteDone(cId)>>_vars =>
                (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>16. [](TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) /\
              ~<<EmitWriteDone(cId)>>_vars =>
                  (IsAwaitingWriteDone(cId) /\
                       ~IsWriteDoneCallbackRunning(cId))')
    BY <1>160, PTL
<1>170. TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
            ~<<WriteDoneReturns(cId)>>_vars =>
                (IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>17. [](TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
              ~<<WriteDoneReturns(cId)>>_vars =>
                  (IsWriteDoneCallbackRunning(cId))')
    BY <1>170, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             [](NoSendsTo(cId))
      PROVE  (SendDrainMeasure(cId) = n + 1) ~>
                 (SendDrainMeasure(cId) = n)
  <2>1. []TypeOK
    BY <1>1, PTL
  <2>2. [](TypeOK /\ FfiCallInv => WriteDonesNeverExceedSends)
    <3>1. TypeOK /\ FfiCallInv => WriteDonesNeverExceedSends
      BY Zenon DEF FfiCallInv
    <3>2. QED BY <3>1, PTL
  <2>3. QED
    BY <1>1, <2>1, <2>2, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15,
       <1>16, <1>17, PTL
<1>2. QED BY <1>1, PTL

\* The bound splits of the frozen ladder, standalone: the induction
\* hypothesis in the consumer's scope bans necessitation there.
LEMMA SendDrainBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ [](TypeOK => (SendDrainMeasure(cId) <= n + 1 =>
                                \/ SendDrainMeasure(cId) <= n
                                \/ SendDrainMeasure(cId) = n + 1))
           /\ [](TypeOK => (SendDrainMeasure(cId) = n =>
                                SendDrainMeasure(cId) <= n))
<1>1. TypeOK => (SendDrainMeasure(cId) <= n + 1 =>
                     \/ SendDrainMeasure(cId) <= n
                     \/ SendDrainMeasure(cId) = n + 1)
    BY SMT DEF SendDrainMeasure, TypeOK, L0!TypeOK
<1>2. TypeOK => (SendDrainMeasure(cId) = n =>
                     SendDrainMeasure(cId) <= n)
    BY SMT DEF SendDrainMeasure, TypeOK, L0!TypeOK
<1>3. QED BY <1>1, <1>2, PTL

\* Under the freeze the send side fully drains and stays drained.
THEOREM SendSideQuiets ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ [](NoSendsTo(cId))
           => <>[]HasNoSendInFlight(cId)
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             [](NoSendsTo(cId))
      PROVE  <>[]HasNoSendInFlight(cId)
  <2> DEFINE Ind(n) == (SendDrainMeasure(cId) <= n) ~> HasNoSendInFlight(cId)
  <2>1. [](TypeOK /\ FfiCallInv =>
               (SendDrainMeasure(cId) <= 0 => HasNoSendInFlight(cId)))
    <3>1. TypeOK /\ FfiCallInv =>
              (SendDrainMeasure(cId) <= 0 => HasNoSendInFlight(cId))
      BY SendQuietBridges, Zenon DEF FfiCallInv
    <3>2. QED BY <3>1, PTL
  <2>2. Ind(0)
    BY <1>1, <2>1, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE Ind(n + 1)
    <3>1. (SendDrainMeasure(cId) = n + 1) ~> (SendDrainMeasure(cId) = n)
      BY <1>1, <2>3, SendQuietDescentFor, PTL
    <3>2. [](TypeOK => (SendDrainMeasure(cId) <= n + 1 =>
                            \/ SendDrainMeasure(cId) <= n
                            \/ SendDrainMeasure(cId) = n + 1))
      BY <2>3, SendDrainBoundSplit, Zenon
    <3>3. [](TypeOK => (SendDrainMeasure(cId) = n =>
                            SendDrainMeasure(cId) <= n))
      BY <2>3, SendDrainBoundSplit, Zenon
    <3>4. []TypeOK
      BY <1>1, PTL
    <3>5. QED BY <2>3, <3>1, <3>2, <3>3, <3>4, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(600)
  <2>5. 2 * MaxSendsInFlight + 1 \in Nat
    BY MaxSendsInFlightIsPositive, SMT
  <2>6. Ind(2 * MaxSendsInFlight + 1)
    BY <2>4, <2>5, Zenon
  <2>7. [](TypeOK /\ FfiCallInv =>
               SendDrainMeasure(cId) <= 2 * MaxSendsInFlight + 1)
    <3>1. TypeOK /\ FfiCallInv =>
              SendDrainMeasure(cId) <= 2 * MaxSendsInFlight + 1
      BY SendQuietBridges, Zenon
    <3>2. QED BY <3>1, PTL
  <2>8. [](TypeOK /\ [Next]_vars /\ NoSendsTo(cId) /\
               HasNoSendInFlight(cId) => (HasNoSendInFlight(cId))')
    <3>1. TypeOK /\ [Next]_vars /\ NoSendsTo(cId) /\
              HasNoSendInFlight(cId) => (HasNoSendInFlight(cId))'
      BY SlotFrame, Zenon DEF NoSendsTo
    <3>2. QED BY <3>1, PTL
  <2>9. QED
    BY <1>1, <2>6, <2>7, <2>8, PTL DEF Ind
<1>2. QED BY <1>1, PTL

\* A held payload is held until its own consumption.
\* The two payload counters move one way only: deliveries append to the
\* event stream, and nothing but a consumption of this very call raises
\* its released count.  Proved once by the full case split, so every
\* consumer downstream - stability, credit recovery, the ladder - is a
\* three-line argument instead of another twenty-action enumeration.
\* Who writes what, stated once.  Every step is either one of the actions
\* that touch the variable, or it leaves it alone; the goal is a bare
\* disjunction, so the twenty-action enumeration is paid once and never
\* again carries an arithmetic argument with it.
\* The event stream only grows.  One fact, one lemma: bundling it with
\* the release counter forced every case to carry both arguments.
LEMMA EventsOnlyGrow ==
    ASSUME NEW cId \in CallIds, TypeOK, [Next]_vars
    PROVE  Len(events_delivered[cId]) <= Len((events_delivered[cId])')
<1>0. /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
      /\ Len(events_delivered[cId]) \in Nat
  <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, LenProperties, Zenon
<1>00. \A ev \in L0!EventKinds :
           Len(Append(events_delivered[cId], ev)) =
               Len(events_delivered[cId]) + 1
  <2>1. events_delivered[cId] \in Seq(L0!EventKinds)
    BY <1>0, Zenon
  <2>2. QED BY <2>1, AppendProperties
<1>1. CASE \/ \E c \in CallIds : DeliverInitialMetadata(c)
           \/ \E c \in CallIds : DeliverMessage(c)
           \/ \E c \in CallIds : DeliverStatus(c)
           \/ \E c \in CallIds : DeliverCancelled(c)
    BY <1>0, <1>00, <1>1, SMTT(60)
    DEF DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, L0!DeliverInitialMetadata, L0!DeliverMessage,
        L0!DeliverStatus, L0!CallCancel
<1>2. CASE UNCHANGED events_delivered
    BY <1>0, <1>2, SMT
<1>3. QED
    BY <1>1, <1>2, EveryStepEitherDeliversOrKeepsEvents

\* The release counter of a call rises only when that call consumes.
LEMMA ReleasesOnlyRiseOnConsume ==
    ASSUME NEW cId \in CallIds, TypeOK, [Next]_vars,
           ~<<HostConsumesEvent(cId)>>_vars
    PROVE  (payloads_consumed_by_host[cId])' =
               payloads_consumed_by_host[cId]
<1>0. payloads_consumed_by_host \in [CallIds -> Nat]
    BY Zenon DEF TypeOK
<1>1. CASE \E c \in CallIds : HostConsumesEvent(c)
    BY <1>0, <1>1, SMT DEF HostConsumesEvent, vars, l0_vars, L0!vars,
        ffi_vars
<1>2. CASE UNCHANGED payloads_consumed_by_host
    BY <1>2, Zenon
<1>3. QED
    BY <1>1, <1>2, EveryStepEitherConsumesOrKeepsReleases

\* A payload the host still owes stays owed until it releases it: the
\* event index cannot fall below k and the released count cannot pass it.
LEMMA HeldPayloadStable ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat,
           TypeOK, TypeOK', HostOwnsPayload(cId, k), [Next]_vars,
           ~<<HostConsumesEvent(cId)>>_vars
    PROVE  (HostOwnsPayload(cId, k))'
<1>0. /\ payloads_consumed_by_host[cId] \in Nat
      /\ Len(events_delivered[cId]) \in Nat
      /\ Len((events_delivered[cId])') \in Nat
  <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. (events_delivered \in [CallIds -> Seq(L0!EventKinds)])'
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>3. QED
    BY <2>1, <2>2, LenProperties, Zenon DEF TypeOK
<1>1. Len(events_delivered[cId]) <= Len((events_delivered[cId])')
    BY EventsOnlyGrow
<1>2. (payloads_consumed_by_host[cId])' = payloads_consumed_by_host[cId]
    BY ReleasesOnlyRiseOnConsume
<1>3. QED
    BY <1>0, <1>1, <1>2, SMT

\* A consumption advances the release counter by exactly one.
LEMMA ConsumeAdvancesRelease ==
    ASSUME NEW cId \in CallIds, TypeOK,
           <<HostConsumesEvent(cId)>>_vars
    PROVE  (payloads_consumed_by_host[cId])' =
               payloads_consumed_by_host[cId] + 1
<1>1. payloads_consumed_by_host \in [CallIds -> Nat]
    BY Zenon DEF TypeOK
<1>2. QED
    BY <1>1, SMT DEF HostConsumesEvent, vars, l0_vars, L0!vars, ffi_vars

\* The bound split of the frozen ladder, standalone: the induction
\* hypothesis in the consumer's scope bans necessitation there.
LEMMA PayloadBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat, NEW n \in Nat
    PROVE  /\ [](TypeOK =>
                     (/\ HostOwnsPayload(cId, k)
                      /\ k - payloads_consumed_by_host[cId] <= n + 1
                      => \/ /\ HostOwnsPayload(cId, k)
                            /\ k - payloads_consumed_by_host[cId] <= n
                         \/ /\ HostOwnsPayload(cId, k)
                            /\ k - payloads_consumed_by_host[cId] = n + 1))
           /\ [](TypeOK =>
                     (/\ HostOwnsPayload(cId, k)
                      /\ k - payloads_consumed_by_host[cId] = n
                      => /\ HostOwnsPayload(cId, k)
                         /\ k - payloads_consumed_by_host[cId] <= n))
<1>1. TypeOK =>
          (/\ HostOwnsPayload(cId, k)
           /\ k - payloads_consumed_by_host[cId] <= n + 1
           => \/ /\ HostOwnsPayload(cId, k)
                 /\ k - payloads_consumed_by_host[cId] <= n
              \/ /\ HostOwnsPayload(cId, k)
                 /\ k - payloads_consumed_by_host[cId] = n + 1)
    BY SMT DEF TypeOK
<1>2. TypeOK =>
          (/\ HostOwnsPayload(cId, k)
           /\ k - payloads_consumed_by_host[cId] = n
           => /\ HostOwnsPayload(cId, k)
              /\ k - payloads_consumed_by_host[cId] <= n)
    BY SMT DEF TypeOK
<1>3. QED BY <1>1, <1>2, PTL

\* One rung down: the host owes payload k with n+1 releases to go, so a
\* consumption is enabled, it fires, and the distance shrinks by one.
THEOREM PayloadDescentFor ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat, NEW n \in Nat
    PROVE  /\ []TypeOK
           /\ [][Next]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((/\ HostOwnsPayload(cId, k)
                /\ k - payloads_consumed_by_host[cId] = n + 1)
               ~> (\/ ~HostOwnsPayload(cId, k)
                   \/ /\ HostOwnsPayload(cId, k)
                      /\ k - payloads_consumed_by_host[cId] = n))
<1>100. TypeOK /\ HostOwnsPayload(cId, k) =>
            ENABLED <<HostConsumesEvent(cId)>>_vars
    BY HostConsumesEventEnabled, Zenon
<1>10. [](TypeOK /\ HostOwnsPayload(cId, k) =>
              ENABLED <<HostConsumesEvent(cId)>>_vars)
    BY <1>100, PTL
\* The two rung facts are stated on the whole rung, arithmetic included:
\* PTL assembles the fairness argument but cannot itself subtract.
<1>110. TypeOK /\ TypeOK' /\
            (HostOwnsPayload(cId, k) /\
             k - payloads_consumed_by_host[cId] = n + 1) /\
            [Next]_vars /\ ~<<HostConsumesEvent(cId)>>_vars =>
                (HostOwnsPayload(cId, k) /\
                 k - payloads_consumed_by_host[cId] = n + 1)'
  <2>1. TypeOK /\ TypeOK' /\ HostOwnsPayload(cId, k) /\ [Next]_vars /\
            ~<<HostConsumesEvent(cId)>>_vars =>
                (HostOwnsPayload(cId, k))'
    BY HeldPayloadStable, Zenon
  <2>2. TypeOK /\ [Next]_vars /\ ~<<HostConsumesEvent(cId)>>_vars =>
            (payloads_consumed_by_host[cId])' =
                payloads_consumed_by_host[cId]
    BY ReleasesOnlyRiseOnConsume, Zenon
  <2>3. QED
    BY <2>1, <2>2, SMT
<1>11. [](TypeOK /\ TypeOK' /\
              (HostOwnsPayload(cId, k) /\
               k - payloads_consumed_by_host[cId] = n + 1) /\
              [Next]_vars /\ ~<<HostConsumesEvent(cId)>>_vars =>
                  (HostOwnsPayload(cId, k) /\
                   k - payloads_consumed_by_host[cId] = n + 1)')
    BY <1>110, PTL
<1>120. TypeOK /\
            (HostOwnsPayload(cId, k) /\
             k - payloads_consumed_by_host[cId] = n + 1) /\
            <<HostConsumesEvent(cId)>>_vars =>
                (\/ ~HostOwnsPayload(cId, k)
                 \/ (HostOwnsPayload(cId, k) /\
                     k - payloads_consumed_by_host[cId] = n))'
  <2>1. TypeOK /\ <<HostConsumesEvent(cId)>>_vars =>
            /\ (payloads_consumed_by_host[cId])' =
                   payloads_consumed_by_host[cId] + 1
            /\ (events_delivered[cId])' = events_delivered[cId]
    <3>1. TypeOK /\ <<HostConsumesEvent(cId)>>_vars =>
              (payloads_consumed_by_host[cId])' =
                  payloads_consumed_by_host[cId] + 1
      BY ConsumeAdvancesRelease, Zenon
    <3>2. <<HostConsumesEvent(cId)>>_vars =>
              (events_delivered[cId])' = events_delivered[cId]
      BY SMT DEF HostConsumesEvent, vars, l0_vars, L0!vars
    <3>3. QED BY <3>1, <3>2, Zenon
  <2>2. TypeOK => payloads_consumed_by_host[cId] \in Nat
    BY Zenon DEF TypeOK
  <2>3. QED
    BY <2>1, <2>2, SMT
<1>12. [](TypeOK /\
              (HostOwnsPayload(cId, k) /\
               k - payloads_consumed_by_host[cId] = n + 1) /\
              <<HostConsumesEvent(cId)>>_vars =>
                  (\/ ~HostOwnsPayload(cId, k)
                   \/ (HostOwnsPayload(cId, k) /\
                       k - payloads_consumed_by_host[cId] = n))')
    BY <1>120, PTL
<1>13. [](TypeOK /\
              (HostOwnsPayload(cId, k) /\
               k - payloads_consumed_by_host[cId] = n + 1) =>
                  ENABLED <<HostConsumesEvent(cId)>>_vars)
    BY <1>100, PTL
<1>1. ASSUME []TypeOK,
             [][Next]_vars,
             WF_vars(HostConsumesEvent(cId))
      PROVE  (/\ HostOwnsPayload(cId, k)
              /\ k - payloads_consumed_by_host[cId] = n + 1)
             ~> (\/ ~HostOwnsPayload(cId, k)
                 \/ /\ HostOwnsPayload(cId, k)
                    /\ k - payloads_consumed_by_host[cId] = n)
  <2>1. [](TypeOK /\ TypeOK')
    BY <1>1, PTL
  <2>2. QED
    BY <1>1, <2>1, <1>11, <1>12, <1>13, PTL
<1>2. QED BY <1>1, PTL

\* The per-payload guarantee, as a measure induction on the distance
\* between the payload and the release counter.  Release being FIFO, one
\* fairness conjunct per call carries every payload of that call: the
\* host cannot consume past k without consuming k.
THEOREM PayloadConsumedFor ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  /\ []TypeOK
           /\ [][Next]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((HostOwnsPayload(cId, k)) ~> (~HostOwnsPayload(cId, k)))
<1>0. k \in Nat
    OBVIOUS
<1> DEFINE P == HostOwnsPayload(cId, k)
           M == k - payloads_consumed_by_host[cId]
<1>1. ASSUME []TypeOK,
             [][Next]_vars,
             WF_vars(HostConsumesEvent(cId))
      PROVE  (HostOwnsPayload(cId, k)) ~> (~HostOwnsPayload(cId, k))
  <2> DEFINE Ind(n) == (P /\ M <= n) ~> ~P
  <2>2. Ind(0)
    <3>1. TypeOK => ~(P /\ M <= 0)
      BY <1>0, SMT DEF TypeOK
    <3>2. QED BY <1>1, <3>1, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE  Ind(n + 1)
    <3>1. (P /\ M = n + 1) ~> (~P \/ (P /\ M = n))
      BY <1>0, <1>1, <2>3, PayloadDescentFor, PTL
    <3>2. [](TypeOK => (P /\ M <= n + 1 =>
                            (P /\ M <= n) \/ (P /\ M = n + 1)))
      BY <1>0, <2>3, PayloadBoundSplit, PTL
    <3>3. [](TypeOK => (P /\ M = n => P /\ M <= n))
      BY <1>0, <2>3, PayloadBoundSplit, PTL
    <3>4. []TypeOK
      BY <1>1, PTL
    <3>5. QED BY <2>3, <3>1, <3>2, <3>3, <3>4, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(600)
  <2>5. Ind(k)
    BY <1>0, <2>4
  <2>6. [](TypeOK => (P => (P /\ M <= k)))
    <3>1. TypeOK => (P => (P /\ M <= k))
      BY <1>0, SMT DEF TypeOK
    <3>2. QED BY <3>1, PTL
  <2>7. QED
    BY <1>1, <2>5, <2>6, PTL DEF Ind
<1>2. QED BY <1>1, PTL

\* A call out of credit holds a payload to consume.
LEMMA CreditExhibit ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ ~HostHasDeliveryCredit(cId) =>
               \E k \in Nat : HostOwnsPayload(cId, k)
<1>0. TypeOK => /\ payloads_consumed_by_host[cId] \in Nat
                /\ Len(events_delivered[cId]) \in Nat
  <2>1. TypeOK => events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, LenProperties, Zenon DEF TypeOK
\* Out of credit means the debt reached the line, which is positive, so
\* the release counter has not caught up: the next index is still owed.
<1>1. TypeOK /\ ~HostHasDeliveryCredit(cId) =>
          /\ payloads_consumed_by_host[cId] + 1 \in Nat
          /\ HostOwnsPayload(cId, payloads_consumed_by_host[cId] + 1)
    BY <1>0, DeliveryCreditsArePositive, SMT
<1>2. QED
    BY <1>1, Zenon

\* Consuming a held payload of a within-credits call frees a credit.
LEMMA CreditCrossing ==
    ASSUME NEW cId \in CallIds, TypeOK, FfiCallInv,
           L0!IsActiveCall(cId), <<HostConsumesEvent(cId)>>_vars
    PROVE  (HostHasDeliveryCredit(cId))'
<1>0. /\ payloads_consumed_by_host[cId] \in Nat
      /\ Len(events_delivered[cId]) \in Nat
  <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, LenProperties, Zenon DEF TypeOK
<1>1. OwedPayloads(cId) <= DeliveryCredits
    BY Zenon DEF FfiCallInv, ActiveCallPayloadsWithinCredits
<1>2. /\ (payloads_consumed_by_host[cId])' =
             payloads_consumed_by_host[cId] + 1
      /\ (events_delivered[cId])' = events_delivered[cId]
  <2>1. (payloads_consumed_by_host[cId])' =
            payloads_consumed_by_host[cId] + 1
    BY ConsumeAdvancesRelease, Zenon
  <2>2. (events_delivered[cId])' = events_delivered[cId]
    BY SMT DEF HostConsumesEvent, vars, l0_vars, L0!vars
  <2>3. QED BY <2>1, <2>2, Zenon
<1>3. QED
    BY <1>0, <1>1, <1>2, DeliveryCreditsArePositive, SMT

\* A delivery is never a stutter: it puts the callback on the host stack.
\* Stated once, with the state tuple expanded here and nowhere else, so
\* the consumers can contradict NoDeliveriesTo without carrying it.
LEMMA DeliveriesAreNotStutter ==
    ASSUME NEW cId \in CallIds, TypeOK
    PROVE  /\ (DeliverInitialMetadata(cId) =>
                   <<DeliverInitialMetadata(cId)>>_vars)
           /\ (DeliverMessage(cId) => <<DeliverMessage(cId)>>_vars)
           /\ (DeliverStatus(cId) => <<DeliverStatus(cId)>>_vars)
           /\ (DeliverCancelled(cId) => <<DeliverCancelled(cId)>>_vars)
<1>1. delivery_callback_running \in [CallIds -> BOOLEAN]
    BY Zenon DEF TypeOK
<1>2. QED
    BY <1>1, SMT
    DEF DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, HandPayloadToHost, HasFreeDeliverySlot,
        HasFreeDeliverySlotForTerminal, vars, l0_vars, L0!vars, ffi_vars

\* The debt only shrinks while no delivery refills it.
LEMMA DebtShrinksUnderNoDeliveries ==
    ASSUME NEW cId \in CallIds, TypeOK, [Next]_vars, NoDeliveriesTo(cId)
    PROVE  (OwedPayloads(cId))' <= OwedPayloads(cId)
<1>0T. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>0. /\ payloads_consumed_by_host[cId] \in Nat
      /\ Len(events_delivered[cId]) \in Nat
  <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, LenProperties, Zenon DEF TypeOK
\* No delivery means the event stream stands still, and the release
\* counter only ever rises, so the gap between them can only close.
<1>1. (events_delivered[cId])' = events_delivered[cId]
  <2>1I. CASE \E c \in CallIds : DeliverInitialMetadata(c)
    <3>0. \A c \in CallIds :
              DeliverInitialMetadata(c) =>
                  events_delivered' =
                      [events_delivered EXCEPT ![c] =
                           Append(events_delivered[c], "INITIAL_METADATA")]
      BY Zenon DEF DeliverInitialMetadata, L0!DeliverInitialMetadata
    <3>1. \A c \in CallIds :
              DeliverInitialMetadata(c) /\ c # cId =>
                  (events_delivered[cId])' = events_delivered[cId]
      BY <3>0, <1>0T, Zenon
    <3>2. ~DeliverInitialMetadata(cId)
      BY DeliveriesAreNotStutter, Zenon DEF NoDeliveriesTo
    <3>3. QED BY <2>1I, <3>1, <3>2, Zenon
  <2>1M. CASE \E c \in CallIds : DeliverMessage(c)
    <3>0. \A c \in CallIds :
              DeliverMessage(c) =>
                  events_delivered' =
                      [events_delivered EXCEPT ![c] =
                           Append(events_delivered[c], "MESSAGE")]
      BY Zenon DEF DeliverMessage, L0!DeliverMessage
    <3>1. \A c \in CallIds :
              DeliverMessage(c) /\ c # cId =>
                  (events_delivered[cId])' = events_delivered[cId]
      BY <3>0, <1>0T, Zenon
    <3>2. ~DeliverMessage(cId)
      BY DeliveriesAreNotStutter, Zenon DEF NoDeliveriesTo
    <3>3. QED BY <2>1M, <3>1, <3>2, Zenon
  <2>1S. CASE \E c \in CallIds : DeliverStatus(c)
    <3>0. \A c \in CallIds :
              DeliverStatus(c) =>
                  events_delivered' =
                      [events_delivered EXCEPT ![c] =
                           Append(events_delivered[c], "COMPLETED")]
      BY Zenon DEF DeliverStatus, L0!DeliverStatus
    <3>1. \A c \in CallIds :
              DeliverStatus(c) /\ c # cId =>
                  (events_delivered[cId])' = events_delivered[cId]
      BY <3>0, <1>0T, Zenon
    <3>2. ~DeliverStatus(cId)
      BY DeliveriesAreNotStutter, Zenon DEF NoDeliveriesTo
    <3>3. QED BY <2>1S, <3>1, <3>2, Zenon
  <2>1C. CASE \E c \in CallIds : DeliverCancelled(c)
\* The cancel terminal is the one delivery with two shapes: it seeds the
\* metadata when nothing was delivered yet.  Only the EXCEPT matters.
    <3>0. \A c \in CallIds :
              DeliverCancelled(c) =>
                  \/ events_delivered' =
                         [events_delivered EXCEPT ![c] =
                              <<"INITIAL_METADATA", "CANCELLED">>]
                  \/ events_delivered' =
                         [events_delivered EXCEPT ![c] =
                              Append(events_delivered[c], "CANCELLED")]
      BY Zenon DEF DeliverCancelled, L0!CallCancel
    <3>1. \A c \in CallIds :
              DeliverCancelled(c) /\ c # cId =>
                  (events_delivered[cId])' = events_delivered[cId]
      BY <3>0, <1>0T, Zenon
    <3>2. ~DeliverCancelled(cId)
      BY DeliveriesAreNotStutter, Zenon DEF NoDeliveriesTo
    <3>3. QED BY <2>1C, <3>1, <3>2, Zenon
  <2>1. CASE \/ \E c \in CallIds : DeliverInitialMetadata(c)
             \/ \E c \in CallIds : DeliverMessage(c)
             \/ \E c \in CallIds : DeliverStatus(c)
             \/ \E c \in CallIds : DeliverCancelled(c)
    BY <2>1I, <2>1M, <2>1S, <2>1C, <2>1
  <2>2. CASE UNCHANGED events_delivered
    BY <2>2, Zenon
  <2>3. QED
    BY <2>1, <2>2, EveryStepEitherDeliversOrKeepsEvents
<1>2. payloads_consumed_by_host[cId] <=
          (payloads_consumed_by_host[cId])'
  <2>1. CASE <<HostConsumesEvent(cId)>>_vars
    BY <1>0, <2>1, ConsumeAdvancesRelease, SMT
  <2>2. CASE ~<<HostConsumesEvent(cId)>>_vars
    BY <1>0, <2>2, ReleasesOnlyRiseOnConsume, SMT
  <2>3. QED BY <2>1, <2>2
<1>20. Len((events_delivered[cId])') = Len(events_delivered[cId])
    BY <1>1, Zenon
<1>21. (payloads_consumed_by_host[cId])' \in Nat
  <2>1. CASE <<HostConsumesEvent(cId)>>_vars
    BY <1>0, <2>1, ConsumeAdvancesRelease, SMT
  <2>2. CASE ~<<HostConsumesEvent(cId)>>_vars
    BY <1>0, <2>2, ReleasesOnlyRiseOnConsume, SMT
  <2>3. QED BY <2>1, <2>2
<1>3. QED
    BY <1>0, <1>2, <1>20, <1>21, SMT

\* A free credit stays free while no delivery refills the debt.
LEMMA CreditStableUnderNoDeliveries ==
    ASSUME NEW cId \in CallIds, TypeOK, TypeOK',
           HostHasDeliveryCredit(cId), [Next]_vars, NoDeliveriesTo(cId)
    PROVE  (HostHasDeliveryCredit(cId))'
<1>1. (OwedPayloads(cId))' <= OwedPayloads(cId)
    BY DebtShrinksUnderNoDeliveries
<1>2. DeliveryCredits \in Nat
    BY DeliveryCreditsArePositive
<1>3. /\ payloads_consumed_by_host[cId] \in Nat
      /\ Len(events_delivered[cId]) \in Nat
      /\ (payloads_consumed_by_host[cId])' \in Nat
      /\ Len((events_delivered[cId])') \in Nat
  <2>1. events_delivered \in [CallIds -> Seq(L0!EventKinds)]
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>2. (events_delivered \in [CallIds -> Seq(L0!EventKinds)])'
    BY Zenon DEF TypeOK, L0!TypeOK
  <2>3. QED BY <2>1, <2>2, LenProperties, Zenon DEF TypeOK
<1>4. QED
    BY <1>1, <1>2, <1>3, SMT

\* Per payload: a held payload leads the active call back under the
\* credit line, on that payload's own host fairness.
THEOREM PayloadCrossesFor ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           /\ [](L0!IsActiveCall(cId))
           => ((HostOwnsPayload(cId, k)) ~> (HostHasDeliveryCredit(cId)))
<1>100. TypeOK /\ HostOwnsPayload(cId, k) =>
            ENABLED <<HostConsumesEvent(cId)>>_vars
    BY HostConsumesEventEnabled, Zenon
<1>10. [](TypeOK /\ HostOwnsPayload(cId, k) =>
              ENABLED <<HostConsumesEvent(cId)>>_vars)
    BY <1>100, PTL
<1>110. TypeOK /\ TypeOK' /\ HostOwnsPayload(cId, k) /\ [Next]_vars /\
            ~<<HostConsumesEvent(cId)>>_vars =>
                (HostOwnsPayload(cId, k))'
    BY HeldPayloadStable, Zenon
<1>11. [](TypeOK /\ TypeOK' /\ HostOwnsPayload(cId, k) /\
              [Next]_vars /\ ~<<HostConsumesEvent(cId)>>_vars =>
                  (HostOwnsPayload(cId, k))')
    BY <1>110, PTL
<1>120. TypeOK /\ FfiCallInv /\ L0!IsActiveCall(cId) /\
            <<HostConsumesEvent(cId)>>_vars =>
                (HostHasDeliveryCredit(cId))'
    BY CreditCrossing, Zenon
<1>12. [](TypeOK /\ FfiCallInv /\ L0!IsActiveCall(cId) /\
              <<HostConsumesEvent(cId)>>_vars =>
                  (HostHasDeliveryCredit(cId))')
    BY <1>120, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(HostConsumesEvent(cId)),
             [](L0!IsActiveCall(cId))
      PROVE  (HostOwnsPayload(cId, k)) ~> (HostHasDeliveryCredit(cId))
  <2>1. []TypeOK
    BY <1>1, PTL
  <2>2. QED
    BY <1>1, <2>1, <1>10, <1>11, <1>12, PTL
<1>2. QED BY <1>1, PTL

\* The boxed forms, standalone: necessitation is banned under the
\* consumers' temporal hypotheses.
LEMMA CreditExhibitBoxed ==
    ASSUME NEW cId \in CallIds
    PROVE  [](TypeOK /\ ~HostHasDeliveryCredit(cId) =>
                  \E k \in Nat : HostOwnsPayload(cId, k))
<1>10. TypeOK /\ ~HostHasDeliveryCredit(cId) =>
           \E k \in Nat : HostOwnsPayload(cId, k)
    BY CreditExhibit, Zenon
<1>1. QED BY <1>10, PTL

LEMMA CreditStableBoxed ==
    ASSUME NEW cId \in CallIds
    PROVE  [](TypeOK /\ TypeOK' /\ HostHasDeliveryCredit(cId) /\
                  [Next]_vars /\ NoDeliveriesTo(cId) =>
                      (HostHasDeliveryCredit(cId))')
<1>10. TypeOK /\ TypeOK' /\ HostHasDeliveryCredit(cId) /\
           [Next]_vars /\ NoDeliveriesTo(cId) =>
               (HostHasDeliveryCredit(cId))'
    BY CreditStableUnderNoDeliveries, Zenon
<1>1. QED BY <1>10, PTL

\* The chain implication, standalone so PTL may necessitate it under the
\* consumers' temporal hypotheses.
LEMMA CreditChainImpl ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ \A k \in Nat :
                   (HostOwnsPayload(cId, k) => <>HostHasDeliveryCredit(cId))
           /\ (~HostHasDeliveryCredit(cId) =>
                   \E k \in Nat : HostOwnsPayload(cId, k))
           => (~HostHasDeliveryCredit(cId) => <>HostHasDeliveryCredit(cId))
<1>1. QED
    BY Zenon

\* The delivery credit comes back and stays once no delivery refills the
\* debt: each held payload crosses on its own host fairness.
THEOREM CreditRecoversFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           /\ [](L0!IsActiveCall(cId))
           /\ [](NoDeliveriesTo(cId))
           => <>[]HostHasDeliveryCredit(cId)
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(HostConsumesEvent(cId)),
             [](L0!IsActiveCall(cId)),
             [](NoDeliveriesTo(cId))
      PROVE  <>[]HostHasDeliveryCredit(cId)
  <2>1. ASSUME NEW k \in Nat
        PROVE [](HostOwnsPayload(cId, k) => <>HostHasDeliveryCredit(cId))
    <3>1. WF_vars(HostConsumesEvent(cId))
      BY <1>1, IsaT(600)
    <3>2. (HostOwnsPayload(cId, k)) ~> (HostHasDeliveryCredit(cId))
      BY <1>1, <3>1, PayloadCrossesFor, PTL
    <3>3. QED
      BY <3>2, PTL
  <2>2. [](\A k \in Nat :
               (HostOwnsPayload(cId, k) => <>HostHasDeliveryCredit(cId)))
        <=> \A k \in Nat :
                [](HostOwnsPayload(cId, k) => <>HostHasDeliveryCredit(cId))
    OBVIOUS
  <2>3. [](\A k \in Nat :
               (HostOwnsPayload(cId, k) => <>HostHasDeliveryCredit(cId)))
    BY <2>1, <2>2, Zenon
  <2>4. [](TypeOK /\ ~HostHasDeliveryCredit(cId) =>
               \E k \in Nat : HostOwnsPayload(cId, k))
    BY CreditExhibitBoxed
  <2>5. [](TypeOK /\ TypeOK' /\ HostHasDeliveryCredit(cId) /\
               [Next]_vars /\ NoDeliveriesTo(cId) =>
                   (HostHasDeliveryCredit(cId))')
    BY CreditStableBoxed
  <2>50. [](TypeOK /\ TypeOK')
    BY <1>1, PTL
  <2>6. []TypeOK
    BY <1>1, PTL
  <2>7. QED
    BY <1>1, <2>3, <2>4, <2>5, <2>50, <2>6, CreditChainImpl, PTL
<1>2. QED BY <1>1, PTL

THEOREM StatusDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverStatus(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ WF_vars(DeliverCancelled(cId))
           => WF_l0_vars(L0!DeliverStatus(cId))

\* The schemas, instantiated at cId and boxed OUTSIDE the scope
\* that assumes the WF hypotheses: necessitation is banned there.
<1>10. [](TypeOK /\ ENABLED <<L0!DeliverStatus(cId)>>_l0_vars =>
            StatusReady(cId))
    BY DS0EnabledIsReady, PTL
<1>11. [](/\ TypeOK
        /\ ActiveCallPayloadsWithinCredits
        /\ StatusReady(cId)
        /\ ~IsDeliveryCallbackRunning(cId)
        /\ HasNoSendInFlight(cId)
        /\ ~IsCancelRequested(cId)
        => ENABLED <<DeliverStatus(cId)>>_vars)
    BY DS1EnabledBridge, PTL
<1>12. [](/\ TypeOK
        /\ ActiveCallPayloadsWithinCredits
        /\ L0!IsActiveCall(cId)
        /\ ~L0!HasStatus(cId)
        /\ ~IsDeliveryCallbackRunning(cId)
        /\ HasNoSendInFlight(cId)
        /\ IsCancelRequested(cId)
        => ENABLED <<DeliverCancelled(cId)>>_vars)
    BY DC1EnabledBridge, PTL
<1>30. [](StatusReady(cId) =>
              L0!IsActiveCall(cId) /\ ~L0!HasStatus(cId))
    BY PTL DEF StatusReady
<1>13. [](TypeOK /\ <<DeliverStatus(cId)>>_vars =>
            <<L0!DeliverStatus(cId)>>_l0_vars)
    BY DS1StepProjects, PTL
<1>14. [](TypeOK /\ <<DeliverStatus(cId)>>_vars => (~StatusReady(cId))')
    BY DS1Kills, PTL
<1>15. [](TypeOK /\ <<DeliverCancelled(cId)>>_vars => (~StatusReady(cId))')
    BY DC1Kills, PTL
<1>16. [](/\ TypeOK
        /\ StatusReady(cId)
        /\ [Next]_vars
        /\ ~<<DeliverStatus(cId)>>_vars
        /\ ~<<DeliverCancelled(cId)>>_vars
        => (StatusReady(cId))')
    BY StatusReadyStable, PTL
<1>170. TypeOK /\ StatusReady(cId) /\ HasNoSendInFlight(cId) /\
              [Next]_vars => (HasNoSendInFlight(cId))'
    BY StatusReadyDrainStable, Zenon
<1>17. [](TypeOK /\ StatusReady(cId) /\ HasNoSendInFlight(cId) /\
              [Next]_vars => (HasNoSendInFlight(cId))')
    BY <1>170, PTL
<1>250. TypeOK /\ StatusReady(cId) /\
              ~IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
              ~<<DeliverStatus(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars =>
                  (~IsDeliveryCallbackRunning(cId))'
    BY StatusReadyDrainStable, Zenon
<1>25. [](TypeOK /\ StatusReady(cId) /\
              ~IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
              ~<<DeliverStatus(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars =>
                  (~IsDeliveryCallbackRunning(cId))')
    BY <1>250, PTL
<1>18. [](TypeOK /\ IsCancelRequested(cId) /\ [Next]_vars =>
             (IsCancelRequested(cId))')
    BY CancelMonotone, PTL
<1>19. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
             ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY DeliveryCallbackReturnsEnabled, PTL
<1>200. TypeOK /\ StatusReady(cId) => NoSendsTo(cId)
    BY NoSendUnderStatusReady, Zenon
<1>20. [](TypeOK /\ StatusReady(cId) => NoSendsTo(cId))
    BY <1>200, PTL
<1>220. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>22. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>220, PTL
<1>24. [](TypeOK /\ [Next]_vars =>
             /\ (IsDeliveryCallbackRunning(cId) /\
                     ~<<DeliveryCallbackReturns(cId)>>_vars =>
                         IsDeliveryCallbackRunning(cId)')
             /\ (~IsDeliveryCallbackRunning(cId) /\
                     ~<<DeliverInitialMetadata(cId)>>_vars /\
                     ~<<DeliverMessage(cId)>>_vars /\
                     ~<<DeliverStatus(cId)>>_vars /\
                     ~<<DeliverCancelled(cId)>>_vars =>
                         (~IsDeliveryCallbackRunning(cId))'))
    BY CallbackFrame, PTL

<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliverStatus(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             WF_vars(DeliverCancelled(cId))
      PROVE  WF_l0_vars(L0!DeliverStatus(cId))
  <2>1. [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits)
    BY <1>1, IndInvParts, PTL
  <2>2. [](TypeOK /\ FfiCallInv)
    BY <2>1, PTL
  <2>3. /\ [](TypeOK /\ FfiCallInv)
        /\ [][Next]_vars
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        /\ [](NoSendsTo(cId))
        => <>[]HasNoSendInFlight(cId)
    BY SendSideQuiets, PTL
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15,
       <1>16, <1>17, <1>18, <1>19, <1>20, <1>22, <1>24, <1>25, <1>30, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* THE DELIVER-MESSAGE LIFT                                                *)
(* The recurrent lift: firing does not kill the guard, so under a          *)
(* persistently enabled level-0 action either the level-1 action fires     *)
(* infinitely often (the goal, via projection) or it eventually never      *)
(* fires and the drains force its enabledness forever, contradicting its   *)
(* WF.  ActiveCallPayloadsWithinCredits keeps the payload two-valued, so LS4 never  *)
(* has to count.                                                           *)
(***************************************************************************)

MsgReady(cId) ==
    /\ L0!IsActiveCall(cId)
    /\ Len(events_delivered[cId]) >= 1
    /\ events_delivered[cId][1] = "INITIAL_METADATA"
    /\ ~L0!HasStatus(cId)
    /\ Len(delivered[cId]) < Len(received[cId])

LEMMA DM0EnabledIsMsgReady ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ ENABLED <<L0!DeliverMessage(cId)>>_l0_vars =>
               MsgReady(cId)
<1>1. QED
    BY ExpandENABLED, SMT
    DEF L0!DeliverMessage, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DM1EnabledBridge ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ MsgReady(cId)
           /\ ~IsDeliveryCallbackRunning(cId)
           /\ HostHasDeliveryCredit(cId)
           /\ ~IsCancelRequested(cId)
           => ENABLED <<DeliverMessage(cId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DM1StepProjects ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverMessage(cId)>>_vars =>
               <<L0!DeliverMessage(cId)>>_l0_vars
<1>1. QED
    BY SMT DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost, HasFreeDeliverySlot,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

LEMMA DC1KillsMsg ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
               (~MsgReady(cId))'
<1>1. QED
    BY SMT DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

\* DeliverStatus is refused while a backlog remains.
LEMMA NoDSUnderMsgReady ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ MsgReady(cId) => ~<<DeliverStatus(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

\* The subscripts go once, up front: the cases below then reason about
\* the actions themselves rather than about the state tuple.
LEMMA MsgReadyStable ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ MsgReady(cId)
           /\ [Next]_vars
           /\ ~<<DeliverMessage(cId)>>_vars
           /\ ~<<DeliverCancelled(cId)>>_vars
           => MsgReady(cId)'
<1>1. ASSUME TypeOK, MsgReady(cId), [Next]_vars,
             ~DeliverMessage(cId),
             ~DeliverCancelled(cId)
      PROVE  MsgReady(cId)'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
\* Runtime and channel steps do not touch a call at all.
      <4>1. CASE \/ NextSafeRuntimeOnly
                 \/ NextSafeRuntimeChannel
                 \/ NextSafeChannelOnly
        <5>1. UNCHANGED L0!CallVars
          BY <4>1, RuntimeAndChannelStepsKeepCalls
        <5>2. QED
          BY <1>1, <5>1, SMT
          DEF L0!CallVars, MsgReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!StatusKinds,
              L0!EventKinds, L0!CallStates
      <4>2. CASE NextSafeChannelCall
        BY <1>1, <4>2, SMTT(45)
        DEF NextSafeChannelCall, ChannelFinishClosing,
            L0!ChannelFinishClosing, L0!ChannelsOf, L0!CallsOf,
            RequestCancellationOfActiveCalls, L0!RuntimeVars,
            L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
            MsgReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!StatusKinds,
              L0!EventKinds, L0!CallStates
      <4>3. CASE NextSafeCallOnly
        BY <1>1, <4>3, SMTT(45)
        DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
            NetworkSend, NetworkReceive, ReceiveStatus,
            DeliverInitialMetadata, DeliverMessage, DeliverStatus,
            DeliverCancelled, L0!CallStart, L0!SendMessage, L0!EndSend,
            L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
            L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
            L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
            L0!CallVars, L0!vars, l0_vars,
            MsgReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!StatusKinds,
              L0!EventKinds, L0!CallStates
      <4>4. QED BY <3>1, <4>1, <4>2, <4>3 DEF NextSafeRefining
    <3>2. CASE NextSafeFfiOnly
\* Nothing an FFI-only step writes is read here.
      <4>1. UNCHANGED l0_vars
        BY <3>2, FfiOnlyStepsKeepL0
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
            TypeOK, L0!TypeOK, L0!IsActiveCall, L0!ActiveCallStates,
            L0!HasStatus, L0!StatusKinds, L0!EventKinds,
            L0!IsUnusedCall, L0!IsTerminalCall, IsClosingChannel, MsgReady
    <3>3. CASE NextFail \/ NextExplicitStutter
      <4>1. UNCHANGED L0!CallVars
        BY <3>3, FailAndStutterStepsKeepCalls
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF L0!CallVars, MsgReady, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!HasStatus, L0!StatusKinds,
              L0!EventKinds, L0!CallStates
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1, DeliverySubscriptCollapses, Zenon

\* Once cancellation is latched no send can retake the slot.
LEMMA SlotFreeStableUnderCancel ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ IsCancelRequested(cId)
           /\ HasNoSendInFlight(cId)
           /\ [Next]_vars
           => (HasNoSendInFlight(cId))'
<1>1. ASSUME TypeOK, IsCancelRequested(cId), HasNoSendInFlight(cId),
             [Next]_vars
      PROVE  (HasNoSendInFlight(cId))'
  <2>0. CASE NextSafeRefining
    <3>1. CASE NextSafeCallOnly
      BY <1>1, <3>1, SMTT(120)
      DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
          NetworkSend, NetworkReceive, ReceiveStatus,
          DeliverInitialMetadata, DeliverMessage, DeliverStatus,
          DeliverCancelled, HandPayloadToHost, HasFreeDeliverySlot, HasFreeDeliverySlotForTerminal,
          L0!CallStart, L0!SendMessage, L0!EndSend, L0!NetworkSend,
          L0!NetworkReceive, L0!ReceiveStatus,
          L0!DeliverInitialMetadata, L0!DeliverMessage,
          L0!DeliverStatus, L0!CallCancel,
          L0!RuntimeVars, L0!ChannelVars, ffi_vars,
          TypeOK, L0!TypeOK
    <3>2. CASE NextSafeRuntimeOnly \/ NextSafeRuntimeChannel \/
               NextSafeChannelOnly \/ NextSafeChannelCall
      BY <1>1, <3>2, SMTT(120)
      DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel,
          NextSafeChannelOnly, NextSafeChannelCall,
          RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
          ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
          RequestCancellationOfActiveCalls, L0!RuntimeCreate, L0!RuntimeRelease,
          L0!RuntimeBeginShutdown, L0!ChannelCreate,
          L0!ChannelStartClosing, L0!ChannelFinishClosing,
          L0!ChannelsOf, L0!CallsOf, L0!IsActiveCall,
          L0!ActiveCallStates, L0!HasStatus,
          L0!RuntimeVars, L0!ChannelVars, L0!CallVars, ffi_vars,
          TypeOK, L0!TypeOK
    <3>3. QED BY <1>1, <2>0, <3>1, <3>2 DEF NextSafeRefining
  <2>1. CASE NextSafeFfiOnly
    BY <1>1, <2>1, SMT
    DEF NextSafeFfiOnly, NextSafeShutdownFfi, NextSafeCallFfi,
        EmitShutdownComplete, ShutdownCallbackReturns, EmitResourcesReleased,
        ResourcesReleasedCallbackReturns, RuntimeDestroy,
        RequestCallCancellation, ReleaseCallHandle, EmitWriteDone, WriteDoneReturns,
        DeliveryCallbackReturns, HostConsumesEvent,
        LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer, IsRuntimeDrained, L0!ChannelsOf,
        L0!IsUnusedCall, l0_vars, L0!vars, ffi_vars,
        TypeOK, L0!TypeOK
  <2>4. CASE NextFail \/ NextExplicitStutter
    BY <1>1, <2>4, SMT
    DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
        RemainReleased, L0!RuntimeFail, L0!RemainFailed,
        L0!RemainReleased, L0!ChannelVars, L0!CallVars, L0!vars,
        l0_vars, ffi_vars, TypeOK, L0!TypeOK
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>0, <2>1, <2>4, <2>2, NextDecomposition
        DEF NextByFootprint, NextSafe
<1>2. QED BY <1>1

LEMMA NoDIMUnderMsgReady ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ MsgReady(cId) => ~<<DeliverInitialMetadata(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
        HandPayloadToHost, HasFreeDeliverySlot, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, MsgReady, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus,
        L0!IsUnusedCall, L0!IsTerminalCall, L0!StatusKinds, L0!EventKinds

\* DeliverCancelled only fires on a latched cancellation.
LEMMA DCRequiresCancel ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
               IsCancelRequested(cId)
<1>1. QED
    BY SMT DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost,
        HasFreeDeliverySlotForTerminal, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus

\* While the backlog is ready and neither the message nor the cancel
\* terminal fires, no delivery of the call fires at all.
LEMMA NoDeliveriesFromMsgReady ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ MsgReady(cId)
           /\ ~<<DeliverMessage(cId)>>_vars
           /\ ~<<DeliverCancelled(cId)>>_vars
           => NoDeliveriesTo(cId)
<1>1. QED
    BY NoDIMUnderMsgReady, NoDSUnderMsgReady, Zenon DEF NoDeliveriesTo

\* The cancelled branch of the message lift, as one absurdity: under a
\* forever-latched cancellation, a forever-ready backlog and no firing of
\* the message or cancel deliveries, the cancel terminal both must fire
\* and cannot.
THEOREM DMCancelBranchFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits)
           /\ [][Next]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ [](IsCancelRequested(cId))
           /\ [](MsgReady(cId))
           /\ [](~<<DeliverMessage(cId)>>_vars)
           /\ [](~<<DeliverCancelled(cId)>>_vars)
           => FALSE
<1>520. TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId)
    BY NoSendUnderCancel, Zenon
<1>52. [](TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId))
    BY <1>520, PTL
<1>500. TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
            ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId)
    BY NoDeliveriesFromMsgReady, Zenon
<1>50. [](TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId))
    BY <1>500, PTL
<1>42. [](MsgReady(cId) =>
              L0!IsActiveCall(cId) /\ ~L0!HasStatus(cId))
    BY PTL DEF MsgReady
<1>22. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY DeliveryCallbackReturnsEnabled, PTL
<1>270. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
            (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>27. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>270, PTL
<1>360. TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
            ~<<DeliveryCallbackReturns(cId)>>_vars =>
                (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>37. [](TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
    BY <1>360, PTL
<1>380. TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
            NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon DEF NoDeliveriesTo
<1>39. [](TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
              NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))')
    BY <1>380, PTL
<1>12. [](TypeOK /\ ActiveCallPayloadsWithinCredits /\ L0!IsActiveCall(cId) /\
              ~L0!HasStatus(cId) /\
              ~IsDeliveryCallbackRunning(cId) /\
              HasNoSendInFlight(cId) /\ IsCancelRequested(cId)
              => ENABLED <<DeliverCancelled(cId)>>_vars)
    BY DC1EnabledBridge, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits),
             [][Next]_vars,
             WF_vars(DeliverCancelled(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             [](IsCancelRequested(cId)),
             [](MsgReady(cId)),
             [](~<<DeliverMessage(cId)>>_vars),
             [](~<<DeliverCancelled(cId)>>_vars)
      PROVE  FALSE
  <2>1. [](TypeOK /\ FfiCallInv)
    BY <1>1, PTL
  <2>2. [](NoSendsTo(cId))
    BY <1>1, <1>52, PTL
  <2>3. /\ [](TypeOK /\ FfiCallInv)
        /\ [][Next]_vars
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        /\ [](NoSendsTo(cId))
        => <>[]HasNoSendInFlight(cId)
    BY SendSideQuiets, PTL
  <2>4. [](NoDeliveriesTo(cId))
    BY <1>1, <1>50, PTL
  <2>5. QED
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <1>42, <1>22, <1>27, <1>37, <1>39,
       <1>12, PTL
<1>2. QED BY <1>1, PTL

\* The uncancelled branch: the credit recovers, the callback returns, the
\* message delivery is enabled forever, and its fairness both must fire
\* it and cannot.
THEOREM DMCreditBranchFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits)
           /\ [][Next]_vars
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ []WF_vars(HostConsumesEvent(cId))
           /\ [](~IsCancelRequested(cId))
           /\ [](MsgReady(cId))
           /\ [](~<<DeliverMessage(cId)>>_vars)
           /\ [](~<<DeliverCancelled(cId)>>_vars)
           => FALSE
<1>500. TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
            ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId)
    BY NoDeliveriesFromMsgReady, Zenon
<1>50. [](TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId))
    BY <1>500, PTL
<1>42. [](MsgReady(cId) =>
              L0!IsActiveCall(cId) /\ ~L0!HasStatus(cId))
    BY PTL DEF MsgReady
<1>22. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY DeliveryCallbackReturnsEnabled, PTL
<1>270. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
            (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>27. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>270, PTL
<1>360. TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
            ~<<DeliveryCallbackReturns(cId)>>_vars =>
                (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>37. [](TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
    BY <1>360, PTL
<1>380. TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
            NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon DEF NoDeliveriesTo
<1>39. [](TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
              NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))')
    BY <1>380, PTL
<1>11. [](TypeOK /\ MsgReady(cId) /\ ~IsDeliveryCallbackRunning(cId) /\
              HostHasDeliveryCredit(cId) /\ ~IsCancelRequested(cId)
              => ENABLED <<DeliverMessage(cId)>>_vars)
    BY DM1EnabledBridge, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits),
             [][Next]_vars,
             WF_vars(DeliverMessage(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             []WF_vars(HostConsumesEvent(cId)),
             [](~IsCancelRequested(cId)),
             [](MsgReady(cId)),
             [](~<<DeliverMessage(cId)>>_vars),
             [](~<<DeliverCancelled(cId)>>_vars)
      PROVE  FALSE
  <2>1. [](TypeOK /\ FfiCallInv)
    BY <1>1, PTL
  <2>2. [](NoDeliveriesTo(cId))
    BY <1>1, <1>50, PTL
  <2>3. [](L0!IsActiveCall(cId))
    BY <1>1, <1>42, PTL
  <2>4. WF_vars(HostConsumesEvent(cId))
    BY <1>1, PTL
  <2>5. /\ [](TypeOK /\ FfiCallInv)
        /\ [][Next]_vars
        /\ WF_vars(HostConsumesEvent(cId))
        /\ [](L0!IsActiveCall(cId))
        /\ [](NoDeliveriesTo(cId))
        => <>[]HostHasDeliveryCredit(cId)
    BY CreditRecoversFor, PTL
  <2>6. <>[]HostHasDeliveryCredit(cId)
    BY <1>1, <2>1, <2>2, <2>3, <2>4, <2>5, PTL
  <2>7. QED
    BY <1>1, <2>1, <2>2, <2>6, <1>22, <1>27, <1>37, <1>39, <1>11, PTL
<1>2. QED BY <1>1, PTL

\* The per-payload fairness family is one opaque atom for LS4 (a
\* quantified formula cannot be unfolded into its box-diamond shape), so
\* its suffix invariance must be handed over explicitly, three-move
\* style like the level-0 BoxedChannelFairness.
\* A single fairness conjunct is its own invariant.  Under the set model
\* this needed a three-move dance, because a family quantified over an
\* unbounded index is one opaque atom for the temporal backend; with one
\* conjunct per call there is nothing left to unfold.
LEMMA BoxedConsumeFairness ==
    ASSUME NEW cId \in CallIds
    PROVE  WF_vars(HostConsumesEvent(cId))
           <=> []WF_vars(HostConsumesEvent(cId))
<1>1. QED BY PTL

THEOREM MessageDeliveryLift ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => WF_l0_vars(L0!DeliverMessage(cId))

\* The schemas, instantiated at cId and boxed outside the WF scope.  The
\* send drain and the credit recovery enter as the two frozen-world
\* theorems; the freezes are discharged by the cancel latch on one branch
\* and by the persisting backlog on the other.
<1>10. [](TypeOK /\ ENABLED <<L0!DeliverMessage(cId)>>_l0_vars =>
              MsgReady(cId))
    BY DM0EnabledIsMsgReady, PTL
<1>11. [](TypeOK /\ MsgReady(cId) /\ ~IsDeliveryCallbackRunning(cId) /\
              HostHasDeliveryCredit(cId) /\ ~IsCancelRequested(cId)
              => ENABLED <<DeliverMessage(cId)>>_vars)
    BY DM1EnabledBridge, PTL
<1>12. [](TypeOK /\ ActiveCallPayloadsWithinCredits /\ L0!IsActiveCall(cId) /\
              ~L0!HasStatus(cId) /\
              ~IsDeliveryCallbackRunning(cId) /\
              HasNoSendInFlight(cId) /\ IsCancelRequested(cId)
              => ENABLED <<DeliverCancelled(cId)>>_vars)
    BY DC1EnabledBridge, PTL
<1>13. [](TypeOK /\ <<DeliverMessage(cId)>>_vars =>
              <<L0!DeliverMessage(cId)>>_l0_vars)
    BY DM1StepProjects, PTL
<1>14. [](TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
              (~MsgReady(cId))')
    BY DC1KillsMsg, PTL
<1>16. [](TypeOK /\ MsgReady(cId) /\ [Next]_vars /\
              ~<<DeliverMessage(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars => MsgReady(cId)')
    BY MsgReadyStable, PTL
<1>21. [](TypeOK /\ IsCancelRequested(cId) /\ [Next]_vars =>
              (IsCancelRequested(cId))')
    BY CancelMonotone, PTL
<1>22. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY DeliveryCallbackReturnsEnabled, PTL
<1>260. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
            (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>27. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>260, PTL
<1>360. TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
            ~<<DeliveryCallbackReturns(cId)>>_vars =>
                (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>37. [](TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
    BY <1>360, PTL
<1>380. TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
            NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon DEF NoDeliveriesTo
<1>39. [](TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
              NoDeliveriesTo(cId) => (~IsDeliveryCallbackRunning(cId))')
    BY <1>380, PTL
<1>42. [](MsgReady(cId) =>
              L0!IsActiveCall(cId) /\ ~L0!HasStatus(cId))
    BY PTL DEF MsgReady
<1>500. TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
            ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId)
    BY NoDeliveriesFromMsgReady, Zenon
<1>50. [](TypeOK /\ MsgReady(cId) /\ ~<<DeliverMessage(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars => NoDeliveriesTo(cId))
    BY <1>500, PTL
<1>510. TypeOK /\ <<DeliverCancelled(cId)>>_vars => IsCancelRequested(cId)
    BY DCRequiresCancel, Zenon
<1>51. [](TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
              IsCancelRequested(cId))
    BY <1>510, PTL
<1>520. TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId)
    BY NoSendUnderCancel, Zenon
<1>52. [](TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId))
    BY <1>520, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliverMessage(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(HostConsumesEvent(cId)),
             WF_vars(DeliverCancelled(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  WF_l0_vars(L0!DeliverMessage(cId))
  <2>1. [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits)
    BY <1>1, IndInvParts, PTL
  <2>2. [](TypeOK /\ FfiCallInv)
    BY <2>1, PTL
  <2>40. []WF_vars(HostConsumesEvent(cId))
    BY <1>1, BoxedConsumeFairness, PTL
  <2>5. QED
    <3>1. SUFFICES ASSUME <>[](ENABLED <<L0!DeliverMessage(cId)>>_l0_vars),
                          <>[](~<<L0!DeliverMessage(cId)>>_l0_vars)
                   PROVE  FALSE
      BY <3>1, PTL
    <3>2. <>[](MsgReady(cId))
      BY <3>1, <2>1, <1>10, PTL
    <3>3. <>[](~<<DeliverMessage(cId)>>_vars)
      BY <3>1, <2>1, <1>13, PTL
    <3>4. CASE <>(IsCancelRequested(cId))
      <4>1. <>[](IsCancelRequested(cId))
        BY <1>1, <3>4, <2>1, <1>21, PTL
      <4>2. CASE []<><<DeliverCancelled(cId)>>_vars
        BY <4>2, <3>2, <2>1, <1>14, PTL
      <4>3. CASE <>[](~<<DeliverCancelled(cId)>>_vars)
        BY <1>1, <2>1, DMCancelBranchFor, <3>2, <3>3, <4>1, <4>3, PTL
      <4>4. QED BY <4>2, <4>3, PTL
    <3>5. CASE [](~IsCancelRequested(cId))
      <4>1. [](~<<DeliverCancelled(cId)>>_vars)
        BY <3>5, <2>1, <1>51, PTL
      <4>2. QED
        BY <1>1, <2>1, DMCreditBranchFor, <2>40, <3>2, <3>3, <3>5,
           <4>1, PTL
    <3>6. QED BY <3>4, <3>5, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* THE CANCELLATION DRAIN, FACTORED                                        *)
(* A cancelled active call always dies: the send slot is acquitted, the    *)
(* callback returns, at most one metadata delivery interferes, and         *)
(* DeliverCancelled fires.  Consumed by the ChannelFinishClosing lift and  *)
(* by CancellationCompletes.                                               *)
(***************************************************************************)

LEMMA DCDeactivates ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
               (~L0!IsActiveCall(cId))'
<1>1. QED
    BY SMT DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        TypeOK, L0!TypeOK, L0!IsActiveCall, L0!ActiveCallStates,
        L0!HasStatus

\* An active cancelled call stays so until a terminal deactivates it.
LEMMA ActiveCancelUnless ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ L0!IsActiveCall(cId)
           /\ IsCancelRequested(cId)
           /\ [Next]_vars
           => \/ (L0!IsActiveCall(cId) /\ IsCancelRequested(cId))'
              \/ (~L0!IsActiveCall(cId))'
<1>1. ASSUME TypeOK, L0!IsActiveCall(cId), IsCancelRequested(cId),
             [Next]_vars
      PROVE  \/ (L0!IsActiveCall(cId) /\ IsCancelRequested(cId))'
             \/ (~L0!IsActiveCall(cId))'
\* Latching is all this needs: cancellation never comes off, so either
\* the call is still active and still cancelled, or it is no longer
\* active.  The case analysis over Next it used to carry proved a
\* tautology sixteen times.
  <2>1. IsCancelRequested(cId)'
    BY <1>1, CancelMonotone
  <2>2. QED
    BY <2>1
<1>2. QED BY <1>1

\* Message and status deliveries are refused on a cancelled call.
LEMMA NoDMUnderCancel ==
    ASSUME NEW cId \in CallIds
    PROVE  IsCancelRequested(cId) => ~<<DeliverMessage(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverMessage, vars, l0_vars, ffi_vars

LEMMA NoDSUnderCancel ==
    ASSUME NEW cId \in CallIds
    PROVE  IsCancelRequested(cId) => ~<<DeliverStatus(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverStatus, vars, l0_vars, ffi_vars

\* The head event, once delivered, never disappears.
LEMMA DIMBreaksEmpty ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliverInitialMetadata(cId)>>_vars =>
               (events_delivered[cId] # <<>>)'
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
        HandPayloadToHost, HasFreeDeliverySlot, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, TypeOK, L0!TypeOK,
        L0!IsActiveCall, L0!ActiveCallStates

LEMMA EventsMonotone ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ events_delivered[cId] # <<>>
           /\ [Next]_vars
           => (events_delivered[cId] # <<>>)'
<1>1. ASSUME TypeOK, events_delivered[cId] # <<>>, [Next]_vars
      PROVE  (events_delivered[cId] # <<>>)'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
\* Runtime and channel steps do not touch a call at all.
      <4>1. CASE \/ NextSafeRuntimeOnly
                 \/ NextSafeRuntimeChannel
                 \/ NextSafeChannelOnly
        <5>1. UNCHANGED L0!CallVars
          BY <4>1, RuntimeAndChannelStepsKeepCalls
        <5>2. QED
          BY <1>1, <5>1, SMT
          DEF L0!CallVars, TypeOK, L0!TypeOK, L0!CallStates
      <4>2. CASE NextSafeChannelCall
        BY <1>1, <4>2, SMTT(45)
        DEF NextSafeChannelCall, ChannelFinishClosing,
            L0!ChannelFinishClosing, L0!ChannelsOf, L0!CallsOf,
            RequestCancellationOfActiveCalls, L0!RuntimeVars,
            L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
            TypeOK, L0!TypeOK, L0!CallStates
      <4>3. CASE NextSafeCallOnly
        BY <1>1, <4>3, SMTT(45)
        DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
            NetworkSend, NetworkReceive, ReceiveStatus,
            DeliverInitialMetadata, DeliverMessage, DeliverStatus,
            DeliverCancelled, L0!CallStart, L0!SendMessage, L0!EndSend,
            L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
            L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
            L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
            L0!CallVars, L0!vars, l0_vars,
            TypeOK, L0!TypeOK, L0!CallStates
      <4>4. QED BY <3>1, <4>1, <4>2, <4>3 DEF NextSafeRefining
    <3>2. CASE NextSafeFfiOnly
\* Nothing an FFI-only step writes is read here.
      <4>1. UNCHANGED l0_vars
        BY <3>2, FfiOnlyStepsKeepL0
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
            TypeOK, L0!TypeOK, L0!IsActiveCall, L0!ActiveCallStates,
            L0!HasStatus, L0!StatusKinds, L0!EventKinds,
            L0!IsUnusedCall, L0!IsTerminalCall, IsClosingChannel, L0!CallStates
    <3>3. CASE NextFail \/ NextExplicitStutter
      <4>1. UNCHANGED L0!CallVars
        BY <3>3, FailAndStutterStepsKeepCalls
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF L0!CallVars, TypeOK, L0!TypeOK, L0!CallStates
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* Metadata delivery needs an empty event trace.
LEMMA NoDIMWhenNonEmpty ==
    ASSUME NEW cId \in CallIds
    PROVE  events_delivered[cId] # <<>> =>
               ~<<DeliverInitialMetadata(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, L0!DeliverInitialMetadata,
        HandPayloadToHost, HasFreeDeliverySlot, vars, l0_vars, L0!vars, ffi_vars,
        L0!RuntimeVars, L0!ChannelVars, L0!IsActiveCall,
        L0!ActiveCallStates

LEMMA ActiveCallHasNoStatusAt ==
    ASSUME NEW cId \in CallIds
    PROVE  FfiCallInv /\ L0!IsActiveCall(cId) => ~L0!HasStatus(cId)
<1>1. QED
    BY DEF FfiCallInv, ActiveCallHasNoStatus

LEMMA ActivePayloadAt ==
    ASSUME NEW cId \in CallIds
    PROVE  FfiCallInv => ActiveCallPayloadsWithinCredits
<1>1. QED
    BY DEF FfiCallInv

THEOREM CancelledCallDies ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                   ~L0!IsActiveCall(cId))

<1>10. [](TypeOK /\ <<DeliverCancelled(cId)>>_vars =>
              (~L0!IsActiveCall(cId))')
    BY DCDeactivates, PTL
<1>11. [](TypeOK /\ L0!IsActiveCall(cId) /\ IsCancelRequested(cId) /\
              [Next]_vars =>
                  \/ (L0!IsActiveCall(cId) /\ IsCancelRequested(cId))'
                  \/ (~L0!IsActiveCall(cId))')
    BY ActiveCancelUnless, PTL
<1>12. [](IsCancelRequested(cId) => ~<<DeliverMessage(cId)>>_vars)
    BY NoDMUnderCancel, PTL
<1>13. [](IsCancelRequested(cId) => ~<<DeliverStatus(cId)>>_vars)
    BY NoDSUnderCancel, PTL
<1>14. [](TypeOK /\ <<DeliverInitialMetadata(cId)>>_vars =>
              (events_delivered[cId] # <<>>)')
    BY DIMBreaksEmpty, PTL
<1>15. [](TypeOK /\ events_delivered[cId] # <<>> /\ [Next]_vars =>
              (events_delivered[cId] # <<>>)')
    BY EventsMonotone, PTL
<1>16. [](events_delivered[cId] # <<>> =>
              ~<<DeliverInitialMetadata(cId)>>_vars)
    BY NoDIMWhenNonEmpty, PTL
<1>17. [](FfiCallInv /\ L0!IsActiveCall(cId) => ~L0!HasStatus(cId))
    BY ActiveCallHasNoStatusAt, PTL
<1>18. [](/\ TypeOK
        /\ ActiveCallPayloadsWithinCredits
        /\ L0!IsActiveCall(cId)
        /\ ~L0!HasStatus(cId)
        /\ ~IsDeliveryCallbackRunning(cId)
        /\ HasNoSendInFlight(cId)
        /\ IsCancelRequested(cId)
        => ENABLED <<DeliverCancelled(cId)>>_vars)
    BY DC1EnabledBridge, PTL
<1>19. [](TypeOK /\ IsCancelRequested(cId) /\ [Next]_vars =>
              (IsCancelRequested(cId))')
    BY CancelMonotone, PTL
<1>20. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY DeliveryCallbackReturnsEnabled, PTL
<1>210. TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId)
    BY NoSendUnderCancel, Zenon
<1>21. [](TypeOK /\ IsCancelRequested(cId) => NoSendsTo(cId))
    BY <1>210, PTL
<1>23. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
           (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>24. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>23, PTL
<1>33. [](TypeOK /\ IsCancelRequested(cId) /\ HasNoSendInFlight(cId) /\
              [Next]_vars => (HasNoSendInFlight(cId))')
    BY SlotFreeStableUnderCancel, PTL
<1>34. TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
           ~<<DeliveryCallbackReturns(cId)>>_vars =>
               (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>35. [](TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
    BY <1>34, PTL
<1>36. TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
           ~<<DeliverInitialMetadata(cId)>>_vars /\
           ~<<DeliverMessage(cId)>>_vars /\
           ~<<DeliverStatus(cId)>>_vars /\
           ~<<DeliverCancelled(cId)>>_vars =>
               (~IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>37. [](TypeOK /\ [Next]_vars /\ ~IsDeliveryCallbackRunning(cId) /\
              ~<<DeliverInitialMetadata(cId)>>_vars /\
              ~<<DeliverMessage(cId)>>_vars /\
              ~<<DeliverStatus(cId)>>_vars /\
              ~<<DeliverCancelled(cId)>>_vars =>
                  (~IsDeliveryCallbackRunning(cId))')
    BY <1>36, PTL
<1>40. [](FfiCallInv => ActiveCallPayloadsWithinCredits)
    BY ActivePayloadAt, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliverCancelled(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                 ~L0!IsActiveCall(cId)
  <2>1. [](TypeOK /\ FfiCallInv /\ ActiveCallPayloadsWithinCredits)
    BY <1>1, IndInvParts, PTL
  <2>2. [](TypeOK /\ FfiCallInv)
    BY <2>1, PTL
  <2>3. /\ [](TypeOK /\ FfiCallInv)
        /\ [][Next]_vars
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        /\ [](NoSendsTo(cId))
        => <>[]HasNoSendInFlight(cId)
    BY SendSideQuiets, PTL
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15,
       <1>16, <1>17, <1>18, <1>19, <1>20, <1>21, <1>24, <1>33, <1>35,
       <1>37, <1>40, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* THE CHANNEL-CLOSE LIFT                                                  *)
(* A closing channel closes: every call on it is cancelled by the         *)
(* invariant, dies by the cancellation drain, the drained guard is        *)
(* accumulated over the finite CallIds, and the level-1 close fires.      *)
(***************************************************************************)

LEMMA CFC0EnabledIsClosing ==
    ASSUME NEW chId \in ChannelIds
    PROVE  TypeOK /\ ENABLED <<L0!ChannelFinishClosing(chId)>>_l0_vars =>
               IsClosingChannel(chId)
<1>1. QED
    BY ExpandENABLED, SMT
    DEF L0!ChannelFinishClosing, l0_vars, L0!vars,
        L0!RuntimeVars, L0!ChannelVars, L0!CallsOf, L0!HasStatus,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK

LEMMA CFC1EnabledBridge ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ TypeOK
           /\ IsClosingChannel(chId)
           /\ (\A cId \in CallIds :
                   call_channel[cId] = chId => ~L0!IsActiveCall(cId))
           => ENABLED <<ChannelFinishClosing(chId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF ChannelFinishClosing, L0!ChannelFinishClosing, l0_vars, L0!vars,
        vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars, L0!CallsOf,
        L0!HasStatus, L0!IsActiveCall, L0!ActiveCallStates,
        TypeOK, L0!TypeOK

LEMMA CFC1StepProjects ==
    ASSUME NEW chId \in ChannelIds
    PROVE  TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
               <<L0!ChannelFinishClosing(chId)>>_l0_vars
<1>1. QED
    BY SMT DEF ChannelFinishClosing, L0!ChannelFinishClosing,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!CallsOf, L0!HasStatus, L0!IsActiveCall, L0!ActiveCallStates,
        TypeOK, L0!TypeOK

LEMMA CFC1KillsClosing ==
    ASSUME NEW chId \in ChannelIds
    PROVE  TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
               (channel_state[chId] # "closing")'
<1>1. QED
    BY SMT DEF ChannelFinishClosing, L0!ChannelFinishClosing,
        l0_vars, L0!vars, vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!CallsOf, L0!HasStatus, L0!IsActiveCall, L0!ActiveCallStates,
        TypeOK, L0!TypeOK

\* Cancellation is already latched on every active call of a closing
\* channel: the invariant, instantiated.
LEMMA ClosingCancelsAt ==
    ASSUME NEW chId \in ChannelIds, NEW cId \in CallIds
    PROVE  /\ FfiCallInv
           /\ IsClosingChannel(chId)
           /\ call_channel[cId] = chId
           /\ L0!IsActiveCall(cId)
           => IsCancelRequested(cId)
<1>1. QED
    BY Zenon DEF FfiCallInv, ClosingChannelCallsCancelRequested

\* The per-call goal of the close: off the channel or inactive.  Stable
\* while the channel is closing: a new call cannot start on it.
LEMMA OffOrDeadStable ==
    ASSUME NEW chId \in ChannelIds, NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ IsClosingChannel(chId)
           /\ (call_channel[cId] = chId => ~L0!IsActiveCall(cId))
           /\ [Next]_vars
           => (call_channel[cId] = chId => ~L0!IsActiveCall(cId))'
<1>1. ASSUME TypeOK, IsClosingChannel(chId),
             call_channel[cId] = chId => ~L0!IsActiveCall(cId),
             [Next]_vars
      PROVE  (call_channel[cId] = chId => ~L0!IsActiveCall(cId))'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
\* Runtime and channel steps do not touch a call at all.
      <4>1. CASE \/ NextSafeRuntimeOnly
                 \/ NextSafeRuntimeChannel
                 \/ NextSafeChannelOnly
        <5>1. UNCHANGED L0!CallVars
          BY <4>1, RuntimeAndChannelStepsKeepCalls
        <5>2. QED
          BY <1>1, <5>1, SMT
          DEF L0!CallVars, IsClosingChannel, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!ChannelStates, L0!CallStates
      <4>2. CASE NextSafeChannelCall
        BY <1>1, <4>2, SMTT(45)
        DEF NextSafeChannelCall, ChannelFinishClosing,
            L0!ChannelFinishClosing, L0!ChannelsOf, L0!CallsOf,
            RequestCancellationOfActiveCalls, L0!RuntimeVars,
            L0!ChannelVars, L0!CallVars, L0!vars, l0_vars,
            IsClosingChannel, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!ChannelStates, L0!CallStates
      <4>3. CASE NextSafeCallOnly
        BY <1>1, <4>3, SMTT(45)
        DEF NextSafeCallOnly, CallStart, SendMessage, EndSend,
            NetworkSend, NetworkReceive, ReceiveStatus,
            DeliverInitialMetadata, DeliverMessage, DeliverStatus,
            DeliverCancelled, L0!CallStart, L0!SendMessage, L0!EndSend,
            L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
            L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
            L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot,
            HasFreeDeliverySlotForTerminal, L0!RuntimeVars, L0!ChannelVars,
            L0!CallVars, L0!vars, l0_vars,
            IsClosingChannel, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!ChannelStates, L0!CallStates
      <4>4. QED BY <3>1, <4>1, <4>2, <4>3 DEF NextSafeRefining
    <3>2. CASE NextSafeFfiOnly
\* Nothing an FFI-only step writes is read here.
      <4>1. UNCHANGED l0_vars
        BY <3>2, FfiOnlyStepsKeepL0
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
            TypeOK, L0!TypeOK, L0!IsActiveCall, L0!ActiveCallStates,
            L0!HasStatus, L0!StatusKinds, L0!EventKinds,
            L0!IsUnusedCall, L0!IsTerminalCall, IsClosingChannel, L0!ChannelStates
    <3>3. CASE NextFail \/ NextExplicitStutter
      <4>1. UNCHANGED L0!CallVars
        BY <3>3, FailAndStutterStepsKeepCalls
      <4>2. QED
        BY <1>1, <4>1, SMT
        DEF L0!CallVars, IsClosingChannel, TypeOK, L0!TypeOK, L0!IsActiveCall,
              L0!ActiveCallStates, L0!ChannelStates, L0!CallStates
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars,
        L0!IsActiveCall, L0!ActiveCallStates
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

THEOREM CallLeavesChannelFor ==
    ASSUME NEW chId \in ChannelIds, NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ [](IsClosingChannel(chId))
           => <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId))

<1>10. [](FfiCallInv /\ IsClosingChannel(chId) /\
              call_channel[cId] = chId /\ L0!IsActiveCall(cId) =>
                  IsCancelRequested(cId))
    BY ClosingCancelsAt, PTL
<1>11. [](TypeOK /\ IsClosingChannel(chId) /\
              (call_channel[cId] = chId => ~L0!IsActiveCall(cId)) /\
              [Next]_vars =>
                  (call_channel[cId] = chId => ~L0!IsActiveCall(cId))')
    BY OffOrDeadStable, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliverCancelled(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             [](IsClosingChannel(chId))
      PROVE  <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId))
  <2>1. [](TypeOK /\ FfiCallInv)
    BY <1>1, IndInvParts, PTL
  <2>2. /\ []IndInv
        /\ [][Next]_vars
        /\ WF_vars(DeliverCancelled(cId))
        /\ WF_vars(DeliveryCallbackReturns(cId))
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                ~L0!IsActiveCall(cId))
    BY CancelledCallDies, PTL
  <2>3. (L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
            ~L0!IsActiveCall(cId)
    BY <1>1, <2>2, PTL
  <2>4. QED
    BY <1>1, <2>1, <2>3, <1>10, <1>11, PTL
<1>2. QED BY <1>1, PTL

\* The accumulated form over the finite CallIds.
THEOREM AllCallsLeaveChannel ==
    ASSUME NEW chId \in ChannelIds
    PROVE  (\A cId \in CallIds :
                <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId)))
           => <>[](\A cId \in CallIds :
                       call_channel[cId] = chId => ~L0!IsActiveCall(cId))
<1>0. USE FiniteCallIds DEF FiniteCallIds
<1> DEFINE G(c) == call_channel[c] = chId => ~L0!IsActiveCall(c)
           K(c) == <>[]G(c)
           I(T) == (\A cId \in T : K(cId)) =>
                       <>[](\A cId \in T : G(cId))
<1>1. I({})
  <2>1. \A cId \in {} : G(cId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET CallIds, NEW x \in CallIds \ T
       PROVE <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
                 <>[](\A cId \in T \cup {x} : G(cId))
  <2>1. (\A cId \in T : G(cId)) /\ G(x) =>
            (\A cId \in T \cup {x} : G(cId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET CallIds, IsFiniteSet(T), I(T),
             NEW x \in CallIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A cId \in T \cup {x} : K(cId)) =>
            (\A cId \in T : K(cId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
            <>[](\A cId \in T \cup {x} : G(cId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(CallIds)
    BY <1>1, <1>2, FS_Induction, IsaMT("blast", 600)
<1>4. QED BY <1>3, Zenon DEF I

(***************************************************************************)
(* Quantified weak fairness is invariant, per family: the three-move.      *)
(***************************************************************************)

THEOREM BoxedDCFairness ==
    (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
    <=> [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
<1>1. [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
      <=> \A cId \in CallIds : [](WF_vars(DeliverCancelled(cId)))
    OBVIOUS
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](WF_vars(DeliverCancelled(cId)))
            <=> WF_vars(DeliverCancelled(cId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600)

THEOREM BoxedCBRFairness ==
    (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
    <=> [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
<1>1. [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
      <=> \A cId \in CallIds : [](WF_vars(DeliveryCallbackReturns(cId)))
    OBVIOUS
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](WF_vars(DeliveryCallbackReturns(cId)))
            <=> WF_vars(DeliveryCallbackReturns(cId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600)

THEOREM BoxedEWFairness ==
    (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
    <=> [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
<1>1. [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
      <=> \A cId \in CallIds : [](WF_vars(EmitWriteDone(cId)))
    OBVIOUS
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](WF_vars(EmitWriteDone(cId)))
            <=> WF_vars(EmitWriteDone(cId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600)

THEOREM BoxedWRFairness ==
    (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
    <=> [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
<1>1. [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
      <=> \A cId \in CallIds : [](WF_vars(WriteDoneReturns(cId)))
    OBVIOUS
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](WF_vars(WriteDoneReturns(cId)))
            <=> WF_vars(WriteDoneReturns(cId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600), PTL

\* The collected drain: while the channel keeps closing, every call ends
\* off it or inactive, jointly.
THEOREM AllCallsDrainFor ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           /\ [](IsClosingChannel(chId))
           => <>[](\A cId \in CallIds :
                       call_channel[cId] = chId => ~L0!IsActiveCall(cId))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId)),
             [](IsClosingChannel(chId))
      PROVE  <>[](\A cId \in CallIds :
                      call_channel[cId] = chId => ~L0!IsActiveCall(cId))
  <2>1. ASSUME NEW cId \in CallIds
        PROVE  <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId))
    <3>1. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliverCancelled(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          /\ [](IsClosingChannel(chId))
          => <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId))
      BY CallLeavesChannelFor, PTL
    <3>2. QED BY <1>1, <3>1, IsaT(600), PTL
  <2>2. \A cId \in CallIds :
            <>[](call_channel[cId] = chId => ~L0!IsActiveCall(cId))
    BY <2>1
  <2>3. QED BY <2>2, AllCallsLeaveChannel
<1>2. QED BY <1>1, PTL

\* The drain, necessitated: the hypotheses are boxed, so the conclusion
\* holds from every suffix, which is where the lift needs it.
THEOREM AllCallsDrainForBoxed ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           => []([](IsClosingChannel(chId)) =>
                     <>[](\A cId \in CallIds :
                              call_channel[cId] = chId =>
                                  ~L0!IsActiveCall(cId)))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId))),
             [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))),
             [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId))),
             [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
      PROVE  []([](IsClosingChannel(chId)) =>
                    <>[](\A cId \in CallIds :
                             call_channel[cId] = chId =>
                                 ~L0!IsActiveCall(cId)))
  <2>1. [][]IndInv
    BY <1>1, PTL
  <2>2. [][][Next]_vars
    BY <1>1, PTL
  <2>3. [][](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
    BY <1>1, BoxedDCFairness, PTL
  <2>4. [][](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
    BY <1>1, BoxedCBRFairness, PTL
  <2>5. [][](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
    BY <1>1, BoxedEWFairness, PTL
  <2>6. [][](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
    BY <1>1, BoxedWRFairness, PTL
  <2>7. /\ []IndInv
        /\ [][Next]_vars
        /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
        /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
        /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
        /\ [](IsClosingChannel(chId))
        => <>[](\A cId \in CallIds :
                    call_channel[cId] = chId => ~L0!IsActiveCall(cId))
    BY AllCallsDrainFor, PTL
  <2>8. QED
    BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, PTL
<1>2. QED BY <1>1

THEOREM ChannelCloseLift ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => WF_l0_vars(L0!ChannelFinishClosing(chId))

<1>10. [](TypeOK /\ ENABLED <<L0!ChannelFinishClosing(chId)>>_l0_vars =>
              IsClosingChannel(chId))
    BY CFC0EnabledIsClosing, PTL
<1>11. [](TypeOK /\ IsClosingChannel(chId) /\
              (\A cId \in CallIds :
                   call_channel[cId] = chId => ~L0!IsActiveCall(cId))
              => ENABLED <<ChannelFinishClosing(chId)>>_vars)
    BY CFC1EnabledBridge, PTL
<1>12. [](TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
              <<L0!ChannelFinishClosing(chId)>>_l0_vars)
    BY CFC1StepProjects, PTL
<1>13. [](TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
              (channel_state[chId] # "closing")')
    BY CFC1KillsClosing, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(ChannelFinishClosing(chId)),
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
      PROVE  WF_l0_vars(L0!ChannelFinishClosing(chId))
  <2>1. []TypeOK
    BY <1>1, IndInvParts, PTL
  <2>2. /\ []IndInv
        /\ [][Next]_vars
        /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
        /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
        /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
        => []([](IsClosingChannel(chId)) =>
                  <>[](\A cId \in CallIds :
                           call_channel[cId] = chId =>
                               ~L0!IsActiveCall(cId)))
    BY AllCallsDrainForBoxed
  <2>3. []([](IsClosingChannel(chId)) =>
               <>[](\A cId \in CallIds :
                        call_channel[cId] = chId =>
                            ~L0!IsActiveCall(cId)))
    BY <1>1, <2>2, BoxedDCFairness, BoxedCBRFairness,
       BoxedEWFairness, BoxedWRFairness, PTL
  <2>4. QED
    BY <1>1, <2>1, <2>3, <1>10, <1>11, <1>12, <1>13, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* THE RUNTIME-RELEASE LIFT                                                *)
(* Under a persistently enabled level-0 release the runtime's calls are   *)
(* terminal at every instant, so the callbacks drain for good, the         *)
(* runtime quiesces, SHUTDOWN_COMPLETE goes out, its callback returns,     *)
(* and the level-1 release fires.                                          *)
(***************************************************************************)

ReleaseReady(rtId) ==
    /\ IsStoppingRuntime(rtId)
    /\ \A chId \in L0!ChannelsOf(rtId) : IsClosedChannel(chId)
    /\ \A chId \in L0!ChannelsOf(rtId) :
           \A cId \in L0!CallsOf(chId) : call_state[cId] = "terminal"

LEMMA Release0EnabledIsReady ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ ENABLED <<L0!RuntimeRelease(rtId)>>_l0_vars =>
               ReleaseReady(rtId)
<1>1. QED
    BY ExpandENABLED, SMT
    DEF L0!RuntimeRelease, ReleaseReady, l0_vars, L0!vars,
        L0!ChannelVars, L0!CallVars, L0!ChannelsOf, L0!CallsOf,
        TypeOK, L0!TypeOK, L0!RuntimeStates

\* A call of a ready runtime is inactive, and no delivery can then touch
\* its callback flag.
LEMMA ReadyCallsInactive ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ ReleaseReady(rtId)
           /\ call_channel[cId] \in L0!ChannelsOf(rtId)
           => ~L0!IsActiveCall(cId)
<1>1. QED
    BY SMT DEF ReleaseReady, L0!ChannelsOf, L0!CallsOf,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK

LEMMA NoDeliveryWhenInactive ==
    ASSUME NEW cId \in CallIds
    PROVE  ~L0!IsActiveCall(cId) =>
               /\ ~<<DeliverInitialMetadata(cId)>>_vars
               /\ ~<<DeliverMessage(cId)>>_vars
               /\ ~<<DeliverStatus(cId)>>_vars
               /\ ~<<DeliverCancelled(cId)>>_vars
<1>1. QED
    BY SMT DEF DeliverInitialMetadata, DeliverMessage, DeliverStatus,
        DeliverCancelled, L0!DeliverInitialMetadata, L0!DeliverMessage,
        L0!DeliverStatus, L0!CallCancel, HandPayloadToHost, HasFreeDeliverySlot, HasFreeDeliverySlotForTerminal,
        vars, l0_vars, L0!vars, ffi_vars, L0!RuntimeVars, L0!ChannelVars,
        L0!IsActiveCall, L0!ActiveCallStates, L0!HasStatus

\* Once out of the runtime, a call stays out while the runtime is ready:
\* a start targets an open channel and the ready runtime has none.
LEMMA QuietStable ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds
    PROVE  /\ TypeOK
           /\ ReleaseReady(rtId)
           /\ (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                   ~IsDeliveryCallbackRunning(cId))
           /\ [Next]_vars
           /\ ~<<DeliveryCallbackReturns(cId)>>_vars
           => (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                   ~IsDeliveryCallbackRunning(cId))'
<1>1. ASSUME TypeOK, ReleaseReady(rtId),
             call_channel[cId] \in L0!ChannelsOf(rtId) =>
                 ~IsDeliveryCallbackRunning(cId),
             [Next]_vars,
             ~<<DeliveryCallbackReturns(cId)>>_vars
      PROVE  (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  ~IsDeliveryCallbackRunning(cId))'
  <2>1. CASE Next
    <3>1. CASE NextSafeRefining
\* The runtime's channel set cannot grow: ownership is written once, and
\* creating a channel needs a RUNNING runtime, which a ready one is not.
      <4>0. (L0!ChannelsOf(rtId))' \subseteq L0!ChannelsOf(rtId)
        <5>1. CASE UNCHANGED channel_runtime
          BY <5>1, Zenon DEF L0!ChannelsOf
        <5>2. CASE \E ch \in ChannelIds, rt \in RuntimeIds :
                     ChannelCreate(ch, rt)
          BY <1>1, <5>2, SMT
          DEF ChannelCreate, L0!ChannelCreate, L0!ChannelsOf, ReleaseReady,
              TypeOK, L0!TypeOK, L0!RuntimeStates
        <5>3. QED
          BY <1>1, <5>1, <5>2, OnlyChannelCreateWritesOwnership, Zenon
\* No call of a released-ready runtime is active, so no delivery can
\* start on one; the closing paths write the cancel latch and nothing
\* else of the FFI state.  Split into three questions - the flag, the
\* call's channel, the runtime's channels - because one call over all of
\* them sits at its time budget rather than inside it.
      <4>1. CASE \/ NextSafeRuntimeOnly
                 \/ NextSafeRuntimeChannel
                 \/ NextSafeChannelOnly
                 \/ NextSafeChannelCall
        <5>1. UNCHANGED delivery_callback_running
          BY <4>1, RuntimeAndChannelStepsKeepDeliveryFlags
        <5>2. UNCHANGED call_channel
          BY <4>1, SMT
          DEF NextSafeRuntimeOnly, NextSafeRuntimeChannel,
              NextSafeChannelOnly, NextSafeChannelCall,
              RuntimeCreate, RuntimeRelease, RuntimeBeginShutdown,
              ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
              L0!RuntimeCreate, L0!RuntimeRelease, L0!RuntimeBeginShutdown,
              L0!ChannelCreate, L0!ChannelStartClosing,
              L0!ChannelFinishClosing, RequestCancellationOfActiveCalls,
              L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars, l0_vars
        <5>3. QED
          BY <1>1, <4>0, <5>1, <5>2, Zenon
      <4>2. CASE NextSafeCallOnly
        <5>0. SUFFICES ASSUME (call_channel[cId])' \in (L0!ChannelsOf(rtId))'
                       PROVE  ~((IsDeliveryCallbackRunning(cId))')
          OBVIOUS
\* The call was already on this runtime: starting one is the only writer of
\* a call's channel, and it needs that channel's runtime RUNNING.
        <5>1. call_channel[cId] \in L0!ChannelsOf(rtId)
          <6>1. CASE UNCHANGED call_channel
            BY <4>0, <5>0, <6>1, Zenon
          <6>2. CASE \E c2 \in CallIds, ch \in ChannelIds :
                       CallStart(c2, ch)
            BY <1>1, <4>0, <5>0, <6>2, SMT
            DEF CallStart, L0!CallStart, L0!ChannelsOf, ReleaseReady,
                TypeOK, L0!TypeOK, L0!RuntimeStates
          <6>3. QED
            BY <1>1, <6>1, <6>2, OnlyCallStartWritesCallChannel, Zenon
        <5>2. ~L0!IsActiveCall(cId)
          BY <1>1, <5>1, SMT
          DEF ReleaseReady, L0!ChannelsOf, L0!CallsOf, L0!IsActiveCall,
              L0!ActiveCallStates, TypeOK, L0!TypeOK, L0!CallStates
        <5>15. ~IsDeliveryCallbackRunning(cId)
          BY <1>1, <5>1
\* A delivery needs an active call, so nothing here can put a callback on
\* the stack; the six other steps of the family do not touch the flag.
        <5>3. CASE \/ \E d \in CallIds : DeliverInitialMetadata(d)
                   \/ \E d \in CallIds : DeliverMessage(d)
                   \/ \E d \in CallIds : DeliverStatus(d)
                   \/ \E d \in CallIds : DeliverCancelled(d)
          BY <1>1, <5>15, <5>2, <5>3, SMT
          DEF DeliverInitialMetadata, DeliverMessage, DeliverStatus,
              DeliverCancelled, HandPayloadToHost,
              L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
              L0!CallCancel, L0!IsActiveCall, L0!ActiveCallStates,
              TypeOK, L0!TypeOK
        <5>4. CASE \/ \E c2 \in CallIds, ch \in ChannelIds :
                        CallStart(c2, ch)
                   \/ \E c2 \in CallIds, m \in Messages,
                        b \in BufferIds : SendMessage(c2, m, b)
                   \/ \E c2 \in CallIds : EndSend(c2)
                   \/ \E c2 \in CallIds : NetworkSend(c2)
                   \/ \E c2 \in CallIds, m \in Messages :
                        NetworkReceive(c2, m)
                   \/ \E c2 \in CallIds : ReceiveStatus(c2)
          <6>1. UNCHANGED delivery_callback_running
            BY <5>4, SMT
            DEF CallStart, SendMessage, EndSend, NetworkSend,
                NetworkReceive, ReceiveStatus, ffi_vars
          <6>2. QED
            BY <5>15, <6>1, Zenon
        <5>5. QED
          BY <4>2, <5>3, <5>4 DEF NextSafeCallOnly
      <4>3. QED BY <3>1, <4>1, <4>2 DEF NextSafeRefining
    <3>2. CASE NextSafeFfiOnly
      <4>1. UNCHANGED l0_vars
        BY <3>2, FfiOnlyStepsKeepL0
      <4>2. SUFFICES ASSUME (call_channel[cId])' \in L0!ChannelsOf(rtId)'
                     PROVE  ~((IsDeliveryCallbackRunning(cId))')
        OBVIOUS
      <4>3. call_channel[cId] \in L0!ChannelsOf(rtId)
        BY <4>1, <4>2, SMT
        DEF l0_vars, L0!vars, L0!RuntimeVars, L0!ChannelVars, L0!CallVars,
            L0!ChannelsOf
      <4>4. ~IsDeliveryCallbackRunning(cId)
        BY <1>1, <4>3
      <4>5. QED
        BY <1>1, <3>2, <4>4, FfiOnlyStepsNeverStartDelivery
    <3>3. CASE NextFail \/ NextExplicitStutter
\* Everything this reads is frozen; the release-ready hypothesis is not
\* needed, and expanding it here is what put the step out of reach.
      <4>1. UNCHANGED L0!CallVars
        BY <3>3, FailAndStutterStepsKeepCalls
      <4>2. UNCHANGED L0!ChannelVars /\ UNCHANGED ffi_vars
        BY <3>3, SMT
        DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
            RemainReleased, L0!RuntimeFail, L0!RemainFailed,
            L0!RemainReleased, L0!vars, L0!ChannelVars
      <4>3. QED
        BY <1>1, <4>1, <4>2, SMT
        DEF L0!CallVars, L0!ChannelVars, ffi_vars,
            IsDeliveryCallbackRunning, L0!ChannelsOf
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition
      DEF NextByFootprint, NextSafe
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars,
        L0!ChannelsOf
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

THEOREM CallQuietsFor ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ [](ReleaseReady(rtId))
           => <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(cId))
<1>100. TypeOK /\ ReleaseReady(rtId) /\
              call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  ~L0!IsActiveCall(cId)
    BY ReadyCallsInactive, Zenon
<1>10. [](TypeOK /\ ReleaseReady(rtId) /\
              call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  ~L0!IsActiveCall(cId))
    BY <1>100, PTL
<1>110. ~L0!IsActiveCall(cId) =>
              /\ ~<<DeliverInitialMetadata(cId)>>_vars
              /\ ~<<DeliverMessage(cId)>>_vars
              /\ ~<<DeliverStatus(cId)>>_vars
              /\ ~<<DeliverCancelled(cId)>>_vars
    BY NoDeliveryWhenInactive, Zenon
<1>11. [](~L0!IsActiveCall(cId) =>
              /\ ~<<DeliverInitialMetadata(cId)>>_vars
              /\ ~<<DeliverMessage(cId)>>_vars
              /\ ~<<DeliverStatus(cId)>>_vars
              /\ ~<<DeliverCancelled(cId)>>_vars)
    BY <1>110, PTL
<1>120. TypeOK /\ ReleaseReady(rtId) /\
              (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                   ~IsDeliveryCallbackRunning(cId)) /\
              [Next]_vars /\ ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(cId))'
    BY QuietStable, Zenon
<1>12. [](TypeOK /\ ReleaseReady(rtId) /\
              (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                   ~IsDeliveryCallbackRunning(cId)) /\
              [Next]_vars /\ ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (call_channel[cId] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(cId))')
    BY <1>120, PTL
<1>130. TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars
    BY DeliveryCallbackReturnsEnabled, Zenon
<1>13. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
    BY <1>130, PTL
<1>14. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
           (~IsDeliveryCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>15. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
    BY <1>14, PTL
<1>16. TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
           ~<<DeliveryCallbackReturns(cId)>>_vars =>
               (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
<1>17. [](TypeOK /\ [Next]_vars /\ IsDeliveryCallbackRunning(cId) /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
    BY <1>16, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(DeliveryCallbackReturns(cId)),
             [](ReleaseReady(rtId))
      PROVE  <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                      ~IsDeliveryCallbackRunning(cId))
  <2>1. []TypeOK
    BY <1>1, IndInvParts, PTL
  <2>2. QED
    BY <1>1, <2>1, <1>10, <1>11, <1>12, <1>13, <1>15, <1>17, PTL
<1>2. QED BY <1>1, PTL

THEOREM AllCallsQuiet ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  (\A cId \in CallIds :
                <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                         ~IsDeliveryCallbackRunning(cId)))
           => <>[](\A cId \in CallIds :
                       call_channel[cId] \in L0!ChannelsOf(rtId) =>
                           ~IsDeliveryCallbackRunning(cId))
<1>0. USE FiniteCallIds DEF FiniteCallIds
<1> DEFINE G(c) == call_channel[c] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(c)
           K(c) == <>[]G(c)
           I(T) == (\A cId \in T : K(cId)) =>
                       <>[](\A cId \in T : G(cId))
<1>1. I({})
  <2>1. \A cId \in {} : G(cId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET CallIds, NEW x \in CallIds \ T
       PROVE <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
                 <>[](\A cId \in T \cup {x} : G(cId))
  <2>1. (\A cId \in T : G(cId)) /\ G(x) =>
            (\A cId \in T \cup {x} : G(cId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET CallIds, IsFiniteSet(T), I(T),
             NEW x \in CallIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A cId \in T \cup {x} : K(cId)) =>
            (\A cId \in T : K(cId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
            <>[](\A cId \in T \cup {x} : G(cId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(CallIds)
    BY <1>1, <1>2, FS_Induction, IsaMT("blast", 600)
<1>4. QED BY <1>3, Zenon DEF I

THEOREM RuntimeQuiesces ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ [](ReleaseReady(rtId))
           => <>[]IsRuntimeDrained(rtId)
<1>10. [](TypeOK /\ FfiCallInv /\ ReleaseReady(rtId) /\
              (\A cId \in CallIds :
                   call_channel[cId] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(cId))
              => IsRuntimeDrained(rtId))
  <2>1. ASSUME TypeOK, FfiCallInv, ReleaseReady(rtId),
               \A cId \in CallIds :
                   call_channel[cId] \in L0!ChannelsOf(rtId) =>
                       ~IsDeliveryCallbackRunning(cId)
        PROVE  IsRuntimeDrained(rtId)
    <3>1. \A cId \in CallIds :
              call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  HasNoSendInFlight(cId)
      BY <2>1, SMT DEF ReleaseReady, FfiCallInv, TerminalCallHasNoSendInFlight,
          L0!ChannelsOf, L0!CallsOf, L0!IsTerminalCall,
          TypeOK, L0!TypeOK
    <3>2. QED
      BY <2>1, <3>1, SMT DEF IsRuntimeDrained, ReleaseReady, L0!ChannelsOf
  <2>2. QED BY <2>1, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             [](ReleaseReady(rtId))
      PROVE  <>[]IsRuntimeDrained(rtId)
  <2>1. [](TypeOK /\ FfiCallInv)
    BY <1>1, IndInvParts, PTL
  <2>2. ASSUME NEW cId \in CallIds
        PROVE  <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                        ~IsDeliveryCallbackRunning(cId))
    <3>1. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ [](ReleaseReady(rtId))
          => <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                      ~IsDeliveryCallbackRunning(cId))
      BY CallQuietsFor, PTL
    <3>2. QED BY <1>1, <3>1, IsaT(600)
  <2>3. \A cId \in CallIds :
            <>[](call_channel[cId] \in L0!ChannelsOf(rtId) =>
                     ~IsDeliveryCallbackRunning(cId))
    BY <2>2
  <2>4. <>[](\A cId \in CallIds :
                 call_channel[cId] \in L0!ChannelsOf(rtId) =>
                     ~IsDeliveryCallbackRunning(cId))
    BY <2>3, AllCallsQuiet
  <2>5. QED
    BY <1>1, <2>1, <2>4, <1>10, PTL
<1>2. QED BY <1>1, PTL

THEOREM RuntimeQuiescesBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           => []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
      PROVE  []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
  <2>1. [][]IndInv
    BY <1>1, PTL
  <2>2. [][][Next]_vars
    BY <1>1, PTL
  <2>3. [][](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
    BY <1>1, BoxedCBRFairness, PTL
  <2>4. /\ []IndInv
        /\ [][Next]_vars
        /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ [](ReleaseReady(rtId))
        => <>[]IsRuntimeDrained(rtId)
    BY RuntimeQuiesces, PTL
  <2>5. QED
    BY <2>1, <2>2, <2>3, <2>4, PTL
<1>2. QED BY <1>1

(***************************************************************************)
(* The shutdown-event chain under a quiesced stopping runtime.             *)
(***************************************************************************)

LEMMA EmitEnabledBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsStoppingRuntime(rtId)
           /\ ~IsShutdownEventEmitted(rtId)
           /\ IsRuntimeDrained(rtId)
           => ENABLED <<EmitShutdownComplete(rtId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF EmitShutdownComplete, IsRuntimeDrained, L0!ChannelsOf,
        vars, l0_vars, L0!vars, ffi_vars, TypeOK, L0!TypeOK

LEMMA EmitSets ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
               (IsShutdownEventEmitted(rtId))'
<1>1. QED
    BY SMT DEF EmitShutdownComplete, IsRuntimeDrained, L0!ChannelsOf,
        vars, l0_vars, L0!vars, ffi_vars, TypeOK, L0!TypeOK

LEMMA EmittedMonotone ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsShutdownEventEmitted(rtId)
           /\ [Next]_vars
           => (IsShutdownEventEmitted(rtId))'
<1>1. ASSUME TypeOK, IsShutdownEventEmitted(rtId), [Next]_vars
      PROVE  (IsShutdownEventEmitted(rtId))'
  <2>1. CASE Next
    <3>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
      BY <1>1, <3>1, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
      BY <1>1, <3>2, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>3. QED
      BY <1>1, <3>1, <3>2, OnlyShutdownStepsWriteShutdownFlags, Zenon
      DEF IsShutdownEventEmitted
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA SCREnabled ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ IsShutdownCallbackRunning(rtId) =>
               ENABLED <<ShutdownCallbackReturns(rtId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF ShutdownCallbackReturns, vars, l0_vars, L0!vars, ffi_vars,
        TypeOK, L0!TypeOK

LEMMA SCRClears ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<ShutdownCallbackReturns(rtId)>>_vars =>
               (~IsShutdownCallbackRunning(rtId))'
<1>1. QED
    BY SMT DEF ShutdownCallbackReturns, vars, l0_vars, L0!vars,
        ffi_vars, TypeOK, L0!TypeOK

LEMMA RunningStable ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsShutdownCallbackRunning(rtId)
           /\ [Next]_vars
           /\ ~<<ShutdownCallbackReturns(rtId)>>_vars
           => (IsShutdownCallbackRunning(rtId))'
<1>1. ASSUME TypeOK, IsShutdownCallbackRunning(rtId), [Next]_vars,
             ~<<ShutdownCallbackReturns(rtId)>>_vars
      PROVE  (IsShutdownCallbackRunning(rtId))'
  <2>1. CASE Next
\* The subscript is collapsed where it is read, once.
    <3>0. ~ShutdownCallbackReturns(rtId)
      BY <1>1, RuntimeSubscriptCollapses
    <3>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
      BY <1>1, <3>1, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
      BY <1>1, <3>0, <3>2, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>3. QED
      BY <1>1, <3>1, <3>2, OnlyShutdownStepsWriteShutdownFlags, Zenon
      DEF IsShutdownCallbackRunning
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA RunningStaysDown ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsShutdownEventEmitted(rtId)
           /\ ~IsShutdownCallbackRunning(rtId)
           /\ [Next]_vars
           => (~IsShutdownCallbackRunning(rtId))'
<1>1. ASSUME TypeOK, IsShutdownEventEmitted(rtId),
             ~IsShutdownCallbackRunning(rtId), [Next]_vars
      PROVE  (~IsShutdownCallbackRunning(rtId))'
  <2>1. CASE Next
    <3>1. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
      BY <1>1, <3>1, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>2. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
      BY <1>1, <3>2, SMT DEF EmitShutdownComplete, ShutdownCallbackReturns,
        IsShutdownEventEmitted, IsShutdownCallbackRunning,
        IsRuntimeDrained, TypeOK, L0!TypeOK
    <3>3. QED
      BY <1>1, <3>1, <3>2, OnlyShutdownStepsWriteShutdownFlags, Zenon
      DEF IsShutdownCallbackRunning
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA Release1EnabledBridge ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsShutdownEventEmitted(rtId)
           /\ ~IsShutdownCallbackRunning(rtId)
           /\ ReleaseReady(rtId)
           => ENABLED <<RuntimeRelease(rtId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF RuntimeRelease, L0!RuntimeRelease, ReleaseReady,
        L0!ChannelsOf, L0!CallsOf, vars, l0_vars, L0!vars, ffi_vars,
        L0!ChannelVars, L0!CallVars, TypeOK, L0!TypeOK, L0!RuntimeStates

LEMMA Release1StepProjects ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
               <<L0!RuntimeRelease(rtId)>>_l0_vars
<1>1. QED
    BY SMT DEF RuntimeRelease, L0!RuntimeRelease, L0!ChannelsOf,
        L0!CallsOf, vars, l0_vars, L0!vars, ffi_vars,
        L0!ChannelVars, L0!CallVars, TypeOK, L0!TypeOK, L0!RuntimeStates

LEMMA Release1Kills ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
               (runtime_state[rtId] # "STOPPING")'
<1>1. QED
    BY SMT DEF RuntimeRelease, L0!RuntimeRelease, L0!ChannelsOf,
        L0!CallsOf, vars, l0_vars, L0!vars, ffi_vars,
        L0!ChannelVars, L0!CallVars, TypeOK, L0!TypeOK, L0!RuntimeStates

LEMMA ReadyIsStopping ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  ReleaseReady(rtId) => IsStoppingRuntime(rtId)
<1>1. QED
    BY DEF ReleaseReady

THEOREM ReleaseLift ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(RuntimeRelease(rtId))
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ WF_vars(ShutdownCallbackReturns(rtId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           => WF_l0_vars(L0!RuntimeRelease(rtId))

<1>101. TypeOK /\ ENABLED <<L0!RuntimeRelease(rtId)>>_l0_vars =>
              ReleaseReady(rtId)
    BY Release0EnabledIsReady, Zenon
<1>10. [](TypeOK /\ ENABLED <<L0!RuntimeRelease(rtId)>>_l0_vars =>
              ReleaseReady(rtId))
    BY <1>101, PTL
<1>110. TypeOK /\ IsStoppingRuntime(rtId) /\
              ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
              => ENABLED <<EmitShutdownComplete(rtId)>>_vars
    BY EmitEnabledBridge, Zenon
<1>11. [](TypeOK /\ IsStoppingRuntime(rtId) /\
              ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
              => ENABLED <<EmitShutdownComplete(rtId)>>_vars)
    BY <1>110, PTL
<1>120. TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
              (IsShutdownEventEmitted(rtId))'
    BY EmitSets, Zenon
<1>12. [](TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
              (IsShutdownEventEmitted(rtId))')
    BY <1>120, PTL
<1>130. TypeOK /\ IsShutdownEventEmitted(rtId) /\ [Next]_vars =>
              (IsShutdownEventEmitted(rtId))'
    BY EmittedMonotone, Zenon
<1>13. [](TypeOK /\ IsShutdownEventEmitted(rtId) /\ [Next]_vars =>
              (IsShutdownEventEmitted(rtId))')
    BY <1>130, PTL
<1>140. TypeOK /\ IsShutdownCallbackRunning(rtId) =>
              ENABLED <<ShutdownCallbackReturns(rtId)>>_vars
    BY SCREnabled, Zenon
<1>14. [](TypeOK /\ IsShutdownCallbackRunning(rtId) =>
              ENABLED <<ShutdownCallbackReturns(rtId)>>_vars)
    BY <1>140, PTL
<1>150. TypeOK /\ <<ShutdownCallbackReturns(rtId)>>_vars =>
              (~IsShutdownCallbackRunning(rtId))'
    BY SCRClears, Zenon
<1>15. [](TypeOK /\ <<ShutdownCallbackReturns(rtId)>>_vars =>
              (~IsShutdownCallbackRunning(rtId))')
    BY <1>150, PTL
<1>160. TypeOK /\ IsShutdownCallbackRunning(rtId) /\ [Next]_vars /\
              ~<<ShutdownCallbackReturns(rtId)>>_vars =>
                  (IsShutdownCallbackRunning(rtId))'
    BY RunningStable, Zenon
<1>16. [](TypeOK /\ IsShutdownCallbackRunning(rtId) /\ [Next]_vars /\
              ~<<ShutdownCallbackReturns(rtId)>>_vars =>
                  (IsShutdownCallbackRunning(rtId))')
    BY <1>160, PTL
<1>170. TypeOK /\ IsShutdownEventEmitted(rtId) /\
              ~IsShutdownCallbackRunning(rtId) /\ [Next]_vars =>
                  (~IsShutdownCallbackRunning(rtId))'
    BY RunningStaysDown, Zenon
<1>17. [](TypeOK /\ IsShutdownEventEmitted(rtId) /\
              ~IsShutdownCallbackRunning(rtId) /\ [Next]_vars =>
                  (~IsShutdownCallbackRunning(rtId))')
    BY <1>170, PTL
<1>180. TypeOK /\ IsShutdownEventEmitted(rtId) /\
              ~IsShutdownCallbackRunning(rtId) /\ ReleaseReady(rtId)
              => ENABLED <<RuntimeRelease(rtId)>>_vars
    BY Release1EnabledBridge, Zenon
<1>18. [](TypeOK /\ IsShutdownEventEmitted(rtId) /\
              ~IsShutdownCallbackRunning(rtId) /\ ReleaseReady(rtId)
              => ENABLED <<RuntimeRelease(rtId)>>_vars)
    BY <1>180, PTL
<1>190. TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
              <<L0!RuntimeRelease(rtId)>>_l0_vars
    BY Release1StepProjects, Zenon
<1>19. [](TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
              <<L0!RuntimeRelease(rtId)>>_l0_vars)
    BY <1>190, PTL
<1>200. TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
              (runtime_state[rtId] # "STOPPING")'
    BY Release1Kills, Zenon
<1>20. [](TypeOK /\ <<RuntimeRelease(rtId)>>_vars =>
              (runtime_state[rtId] # "STOPPING")')
    BY <1>200, PTL
<1>210. ReleaseReady(rtId) => IsStoppingRuntime(rtId)
    BY ReadyIsStopping, Zenon
<1>21. [](ReleaseReady(rtId) => IsStoppingRuntime(rtId))
    BY <1>210, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(RuntimeRelease(rtId)),
             WF_vars(EmitShutdownComplete(rtId)),
             WF_vars(ShutdownCallbackReturns(rtId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
      PROVE  WF_l0_vars(L0!RuntimeRelease(rtId))
  <2>1. []TypeOK
    BY <1>1, IndInvParts, PTL
  <2>2. /\ []IndInv
        /\ [][Next]_vars
        /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        => []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
    BY RuntimeQuiescesBoxed
  <2>3. []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
    BY <1>1, <2>2, BoxedCBRFairness, PTL
  <2>4. QED
    BY <1>1, <2>1, <2>3, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15,
       <1>16, <1>17, <1>18, <1>19, <1>20, <1>21, PTL
<1>2. QED BY <1>1, PTL

(***************************************************************************)
(* FAIRNESS, EXTRACTED ONCE                                                *)
(* Every liveness argument needs a handful of weak-fairness conjuncts at   *)
(* one fixed identifier.  Unfolding the whole of Fairness at each use site *)
(* hands a backend nineteen quantified temporal atoms to select from and   *)
(* instantiate in a single step, and whether it succeeds depends on the    *)
(* load the run puts on it.  These five lemmas do that work once, so a use *)
(* site cites a small fact already stated at its own identifier.           *)
(***************************************************************************)

LEMMA FairnessAtCall ==
    ASSUME Fairness, NEW cId \in CallIds
    PROVE  /\ WF_vars(NetworkSend(cId))
           /\ WF_vars(ReceiveStatus(cId))
           /\ WF_vars(DeliverInitialMetadata(cId))
           /\ WF_vars(DeliverMessage(cId))
           /\ WF_vars(DeliverStatus(cId))
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           /\ WF_vars(ReleaseCallHandle(cId))
BY IsaT(600) DEF Fairness

LEMMA FairnessAtRuntime ==
    ASSUME Fairness, NEW rtId \in RuntimeIds
    PROVE  /\ WF_vars(RuntimeRelease(rtId))
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ WF_vars(ShutdownCallbackReturns(rtId))
           /\ WF_vars(EmitResourcesReleased(rtId))
           /\ WF_vars(ResourcesReleasedCallbackReturns(rtId))
BY IsaT(600) DEF Fairness

LEMMA FairnessAtChannel ==
    ASSUME Fairness, NEW chId \in ChannelIds
    PROVE  WF_vars(ChannelFinishClosing(chId))
BY IsaT(600) DEF Fairness

LEMMA FairnessAtBuffer ==
    ASSUME Fairness, NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ WF_vars(HostReturnsBuffer(cId, b))
           /\ WF_vars(FreeReturnedBuffer(cId, b))
BY IsaT(600) DEF Fairness

\* The conjuncts some goals need still quantified: selection with no
\* instantiation.
LEMMA FairnessEverywhere ==
    ASSUME Fairness
    PROVE  /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           /\ \A cId \in CallIds : WF_vars(HostConsumesEvent(cId))
           /\ \A cId \in CallIds, b \in BufferIds :
                  WF_vars(HostReturnsBuffer(cId, b))
           /\ \A cId \in CallIds, b \in BufferIds :
                  WF_vars(FreeReturnedBuffer(cId, b))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
BY IsaT(600) DEF Fairness
(***************************************************************************)
(* REFINEMENT, ASSEMBLED                                                   *)
(* Spec implies the whole level-0 specification: the step half through    *)
(* RefinesInit/RefinesNext, the fairness half through the seven lifts,    *)
(* each level-0 WF conjunct rebuilt from the level-1 fairness.            *)
(***************************************************************************)

THEOREM RefinesSpec == Spec => L0!Spec
<1>1. ASSUME Spec PROVE L0!Spec
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. L0!Init
    BY <2>0, RefinesInit, PTL
  <2>3. [][L0!Next]_l0_vars
    BY <2>0, RefinesNext, PTL
  <2>4. ASSUME NEW cId \in CallIds
        PROVE  WF_l0_vars(L0!NetworkSend(cId))
    <3>1. WF_vars(NetworkSend(cId))
      BY <2>0, FairnessAtCall, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(NetworkSend(cId))
          => WF_l0_vars(L0!NetworkSend(cId))
      BY NetworkSendLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, PTL
  <2>5. ASSUME NEW cId \in CallIds
        PROVE  WF_l0_vars(L0!ReceiveStatus(cId))
    <3>1. WF_vars(ReceiveStatus(cId))
      BY <2>0, FairnessAtCall, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(ReceiveStatus(cId))
          => WF_l0_vars(L0!ReceiveStatus(cId))
      BY ReceiveStatusLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, PTL
  <2>6. ASSUME NEW cId \in CallIds
        PROVE  WF_l0_vars(L0!DeliverInitialMetadata(cId))
    <3>1. WF_vars(DeliverInitialMetadata(cId))
      BY <2>0, FairnessAtCall, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliverInitialMetadata(cId))
          => WF_l0_vars(L0!DeliverInitialMetadata(cId))
      BY MetadataDeliveryLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, PTL
  <2>7. ASSUME NEW cId \in CallIds
        PROVE  WF_l0_vars(L0!DeliverMessage(cId))
    <3>1. /\ WF_vars(DeliverMessage(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ \A k \in PayloadIndices :
                 WF_vars(HostConsumesEvent(cId))
          /\ WF_vars(DeliverCancelled(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
      BY <2>0, FairnessAtCall, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliverMessage(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ \A k \in PayloadIndices :
                 WF_vars(HostConsumesEvent(cId))
          /\ WF_vars(DeliverCancelled(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          => WF_l0_vars(L0!DeliverMessage(cId))
      BY MessageDeliveryLift, IsaT(600)
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, IsaT(600)
  <2>8. ASSUME NEW cId \in CallIds
        PROVE  WF_l0_vars(L0!DeliverStatus(cId))
    <3>1. /\ WF_vars(DeliverStatus(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          /\ WF_vars(DeliverCancelled(cId))
      BY <2>0, FairnessAtCall, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliverStatus(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          /\ WF_vars(DeliverCancelled(cId))
          => WF_l0_vars(L0!DeliverStatus(cId))
      BY StatusDeliveryLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, PTL
  <2>9. ASSUME NEW rtId \in RuntimeIds
        PROVE  WF_l0_vars(L0!RuntimeRelease(rtId))
    <3>1. /\ WF_vars(RuntimeRelease(rtId))
          /\ WF_vars(EmitShutdownComplete(rtId))
          /\ WF_vars(ShutdownCallbackReturns(rtId))
          /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
      BY <2>0, FairnessAtRuntime, FairnessEverywhere, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(RuntimeRelease(rtId))
          /\ WF_vars(EmitShutdownComplete(rtId))
          /\ WF_vars(ShutdownCallbackReturns(rtId))
          /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
          => WF_l0_vars(L0!RuntimeRelease(rtId))
      BY ReleaseLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, IsaT(600)
  <2>10. ASSUME NEW chId \in ChannelIds
         PROVE  WF_l0_vars(L0!ChannelFinishClosing(chId))
    <3>1. /\ WF_vars(ChannelFinishClosing(chId))
          /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
          /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
          /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
          /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
      BY <2>0, FairnessAtChannel, FairnessEverywhere, IsaT(600)
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(ChannelFinishClosing(chId))
          /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
          /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
          /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
          /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
          => WF_l0_vars(L0!ChannelFinishClosing(chId))
      BY ChannelCloseLift, PTL
    <3>3. QED BY <2>0, <2>1, <3>1, <3>2, IsaT(600)
  <2>11. L0!Fairness
    BY <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, IsaT(600)
    DEF L0!Fairness, l0_vars
  <2>12. QED
    BY <2>2, <2>3, <2>11, PTL DEF L0!Spec, l0_vars
<1>2. QED BY <1>1

(***************************************************************************)
(* INHERITED GUARANTEES                                                    *)
(***************************************************************************)

THEOREM InheritedSafety == Spec => []L0!SafetyInvariant
<1>1. Spec => []SafetyInvariant
    BY SafetyTheorem
<1>2. SafetyInvariant => L0!SafetyInvariant
    BY DEF SafetyInvariant
<1>3. QED
    BY <1>1, <1>2, PTL

THEOREM InheritedLivenessTheorem == Spec => L0!LivenessProperties
<1>1. L0!Spec => L0!LivenessProperties
    BY L0!LivenessTheorem, L0Assumptions, Zenon
<1>2. QED
    BY <1>1, RefinesSpec, PTL

(***************************************************************************)
(* WORLD BRIDGES                                                           *)
(* A safe behavior is a behavior, and the healthy strengthening implies    *)
(* the inductive invariant: every full-world drain serves the safe world.  *)
(***************************************************************************)

LEMMA SafeIsNext == NextSafe => Next
<1>1. QED
    BY Zenon DEF NextSafe, NextSafeRefining, NextSafeFfiOnly,
        NextSafeRuntimeOnly, NextSafeRuntimeChannel, NextSafeChannelOnly,
        NextSafeChannelCall, NextSafeCallOnly, NextSafeShutdownFfi,
        NextSafeCallFfi, Next

LEMMA StrongInvImpliesIndInv == StrongInv => IndInv
<1>1. QED
    BY Zenon DEF StrongInv, IndInv, L0!StrongInv, L0!StructuralInv

(***************************************************************************)
(* CANCELLATION COMPLETES                                                  *)
(***************************************************************************)

THEOREM CancellationProgressSafeFor ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                   ~L0!IsActiveCall(cId))
<1>10. [](StrongInv => IndInv)
    BY StrongInvImpliesIndInv, PTL
<1>11. [](NextSafe => Next)
    BY SafeIsNext, PTL
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(DeliverCancelled(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                 ~L0!IsActiveCall(cId)
  <2>1. []IndInv
    BY <1>1, <1>10, PTL
  <2>2. [][Next]_vars
    BY <1>1, <1>11, PTL
  <2>3. /\ []IndInv
        /\ [][Next]_vars
        /\ WF_vars(DeliverCancelled(cId))
        /\ WF_vars(DeliveryCallbackReturns(cId))
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                ~L0!IsActiveCall(cId))
    BY CancelledCallDies, PTL
  <2>4. QED BY <1>1, <2>1, <2>2, <2>3, PTL
<1>2. QED BY <1>1, PTL

THEOREM CancellationFairnessRequirement ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(DeliverCancelled(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                   ~L0!IsActiveCall(cId))
<1>1. /\ Init
      /\ [][NextSafe]_vars
      => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED
    BY <1>1, CancellationProgressSafeFor, PTL

(***************************************************************************)
(* EVERY ACCEPTED SEND IS ACQUITTED                                        *)
(* Per send: the k-th accepted send is eventually acquitted by its         *)
(* in-order WRITE_DONE, callback returned.  Induction on the missing       *)
(* returns k - WriteDonesReturned; new sends only grow Len(submitted), so  *)
(* no freeze hypothesis is needed.                                         *)
(***************************************************************************)

\* The k-th accepted send stays accepted: Len(submitted) is monotone.
LEMMA AcceptedSendStable ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat
    PROVE  TypeOK /\ HasAcceptedSendAt(cId, k) /\ [Next]_vars =>
               (HasAcceptedSendAt(cId, k))'
<1>1. ASSUME TypeOK, HasAcceptedSendAt(cId, k), [Next]_vars
      PROVE  (HasAcceptedSendAt(cId, k))'
  <2>1. CASE Next
    <3>1. CASE \E c \in CallIds, m \in Messages,
                  bs \in BufferIds : SendMessage(c, m, bs)
      <4>1. PICK c \in CallIds, m \in Messages, bs \in BufferIds :
              SendMessage(c, m, bs)
        BY <3>1
      <4>2. submitted' = [submitted EXCEPT ![c] = Append(@, m)]
        BY <4>1, Zenon DEF SendMessage, L0!SendMessage
      <4>3. submitted[cId] \in Seq(Messages)
        BY <1>1, Zenon DEF TypeOK, L0!TypeOK
      <4>4. CASE c = cId
        <5>1. (submitted[cId])' = Append(submitted[cId], m)
          BY <1>1, <4>2, <4>4, Zenon DEF TypeOK, L0!TypeOK
        <5>2. /\ Append(submitted[cId], m) \in Seq(Messages)
              /\ Len(Append(submitted[cId], m)) = Len(submitted[cId]) + 1
          BY <4>3, AppendProperties
        <5>3. Len(submitted[cId]) \in Nat
          BY <4>3, LenProperties
        <5>4. QED
          BY <1>1, <5>1, <5>2, <5>3, SMT DEF HasAcceptedSendAt
      <4>5. CASE c # cId
        <5>1. (submitted[cId])' = submitted[cId]
          BY <1>1, <4>2, <4>5, Zenon DEF TypeOK, L0!TypeOK
        <5>2. QED
          BY <1>1, <5>1, Zenon DEF HasAcceptedSendAt
      <4>6. QED BY <4>4, <4>5
    <3>2. QED
      BY <1>1, <3>1, EveryStepEitherSubmitsOrKeepsSubmitted, Zenon
      DEF HasAcceptedSendAt
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* The returned count moves only through WriteDoneReturns: EmitWriteDone
\* trades one owed emission for the running callback, everything else
\* leaves both components alone.
LEMMA ReturnedFrameUnlessWDR ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ [Next]_vars /\ ~<<WriteDoneReturns(cId)>>_vars =>
               (WriteDonesReturned(cId))' = WriteDonesReturned(cId)
<1>1. ASSUME TypeOK, [Next]_vars, ~<<WriteDoneReturns(cId)>>_vars
      PROVE  (WriteDonesReturned(cId))' = WriteDonesReturned(cId)
  <2>1. CASE Next
    <3>0. ~WriteDoneReturns(cId)
      BY <1>1, SubscriptCollapses
    <3>1. CASE \E c \in CallIds : EmitWriteDone(c)
      BY <1>1, <3>1, SMT
      DEF EmitWriteDone, WriteDonesReturned, IsAwaitingWriteDone,
          IsWriteDoneCallbackRunning, TypeOK, L0!TypeOK
    <3>2. CASE \E c \in CallIds : WriteDoneReturns(c)
      BY <1>1, <3>0, <3>2, SMT
      DEF WriteDoneReturns, WriteDonesReturned,
          IsWriteDoneCallbackRunning, TypeOK, L0!TypeOK
    <3>3. QED
      BY <1>1, <3>1, <3>2, EveryStepEitherEmitsOrKeepsWriteDones,
         EveryStepEitherAcquitsOrKeepsWriteDoneFlag, Zenon
      DEF WriteDonesReturned
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* The arithmetic bridges of the acquittal ladder.
LEMMA SendRungBridges ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat
    PROVE  /\ (TypeOK /\ HasAcceptedSendAt(cId, k) /\
                   ~IsSendAcquittedAt(cId, k) /\
                   ~IsWriteDoneCallbackRunning(cId) =>
                       IsAwaitingWriteDone(cId))
           /\ (TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
                   ~<<WriteDoneReturns(cId)>>_vars)
<1>1. QED
    BY SMT DEF TypeOK, L0!TypeOK, EmitWriteDone, WriteDoneReturns,
        vars, l0_vars, L0!vars, ffi_vars

\* One rung: with n + 1 returns missing for send k, the running callback
\* returns, or the owed WRITE_DONE goes out and then returns; either way
\* the missing count drops.
THEOREM SendDescentFor ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((/\ HasAcceptedSendAt(cId, k)
                /\ ~IsSendAcquittedAt(cId, k)
                /\ k - WriteDonesReturned(cId) = n + 1)
               ~> (\/ IsSendAcquittedAt(cId, k)
                   \/ /\ HasAcceptedSendAt(cId, k)
                      /\ ~IsSendAcquittedAt(cId, k)
                      /\ k - WriteDonesReturned(cId) = n))
<1>100. TypeOK /\ HasAcceptedSendAt(cId, k) /\
            ~IsSendAcquittedAt(cId, k) /\
            ~IsWriteDoneCallbackRunning(cId) => IsAwaitingWriteDone(cId)
    BY SendRungBridges, Zenon
<1>10. [](TypeOK /\ HasAcceptedSendAt(cId, k) /\
              ~IsSendAcquittedAt(cId, k) /\
              ~IsWriteDoneCallbackRunning(cId) => IsAwaitingWriteDone(cId))
    BY <1>100, PTL
<1>110. TypeOK /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) =>
                ENABLED <<EmitWriteDone(cId)>>_vars
    BY EmitWriteDoneEnabled, Zenon
<1>11. [](TypeOK /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) =>
                  ENABLED <<EmitWriteDone(cId)>>_vars)
    BY <1>110, PTL
<1>120. TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
            ENABLED <<WriteDoneReturns(cId)>>_vars
    BY WriteDoneReturnsEnabled, Zenon
<1>12. [](TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
              ENABLED <<WriteDoneReturns(cId)>>_vars)
    BY <1>120, PTL
<1>130. TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
            (IsWriteDoneCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>13. [](TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
              (IsWriteDoneCallbackRunning(cId))')
    BY <1>130, PTL
<1>140. TypeOK /\ k - WriteDonesReturned(cId) = n + 1 /\
            HasAcceptedSendAt(cId, k) /\ <<WriteDoneReturns(cId)>>_vars =>
                (\/ IsSendAcquittedAt(cId, k)
                 \/ /\ HasAcceptedSendAt(cId, k)
                    /\ ~IsSendAcquittedAt(cId, k)
                    /\ k - WriteDonesReturned(cId) = n)'
    BY SMT DEF TypeOK, L0!TypeOK, WriteDoneReturns,
        vars, l0_vars, L0!vars, ffi_vars
<1>14. [](TypeOK /\ k - WriteDonesReturned(cId) = n + 1 /\
              HasAcceptedSendAt(cId, k) /\ <<WriteDoneReturns(cId)>>_vars =>
                  (\/ IsSendAcquittedAt(cId, k)
                   \/ /\ HasAcceptedSendAt(cId, k)
                      /\ ~IsSendAcquittedAt(cId, k)
                      /\ k - WriteDonesReturned(cId) = n)')
    BY <1>140, PTL
<1>150. TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) /\
            ~<<EmitWriteDone(cId)>>_vars =>
                (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>15. [](TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) /\
              ~<<EmitWriteDone(cId)>>_vars =>
                  (IsAwaitingWriteDone(cId) /\
                       ~IsWriteDoneCallbackRunning(cId))')
    BY <1>150, PTL
<1>160. TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
            ~<<WriteDoneReturns(cId)>>_vars =>
                (IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>16. [](TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
              ~<<WriteDoneReturns(cId)>>_vars =>
                  (IsWriteDoneCallbackRunning(cId))')
    BY <1>160, PTL
<1>170. TypeOK /\ [Next]_vars /\ ~<<WriteDoneReturns(cId)>>_vars /\
            k - WriteDonesReturned(cId) = n + 1 =>
                (k - WriteDonesReturned(cId) = n + 1)'
    BY ReturnedFrameUnlessWDR, SMT
<1>17. [](TypeOK /\ [Next]_vars /\ ~<<WriteDoneReturns(cId)>>_vars /\
              k - WriteDonesReturned(cId) = n + 1 =>
                  (k - WriteDonesReturned(cId) = n + 1)')
    BY <1>170, PTL
<1>180. TypeOK /\ [Next]_vars /\ HasAcceptedSendAt(cId, k) =>
            (HasAcceptedSendAt(cId, k))'
    BY AcceptedSendStable, Zenon
<1>18. [](TypeOK /\ [Next]_vars /\ HasAcceptedSendAt(cId, k) =>
              (HasAcceptedSendAt(cId, k))')
    BY <1>180, PTL
<1>190. TypeOK /\ ~IsSendAcquittedAt(cId, k) /\
            k - WriteDonesReturned(cId) = n + 1 /\
            ~<<WriteDoneReturns(cId)>>_vars /\ [Next]_vars =>
                (~IsSendAcquittedAt(cId, k))'
    BY ReturnedFrameUnlessWDR, SMT
<1>19. [](TypeOK /\ ~IsSendAcquittedAt(cId, k) /\
              k - WriteDonesReturned(cId) = n + 1 /\
              ~<<WriteDoneReturns(cId)>>_vars /\ [Next]_vars =>
                  (~IsSendAcquittedAt(cId, k))')
    BY <1>190, PTL
<1>200. TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
            ~<<WriteDoneReturns(cId)>>_vars
    BY SendRungBridges, Zenon
<1>20. [](TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
              ~<<WriteDoneReturns(cId)>>_vars)
    BY <1>200, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (/\ HasAcceptedSendAt(cId, k)
              /\ ~IsSendAcquittedAt(cId, k)
              /\ k - WriteDonesReturned(cId) = n + 1)
             ~> (\/ IsSendAcquittedAt(cId, k)
                 \/ /\ HasAcceptedSendAt(cId, k)
                    /\ ~IsSendAcquittedAt(cId, k)
                    /\ k - WriteDonesReturned(cId) = n)
  <2>1. []TypeOK
    BY <1>1, PTL
  <2>2. QED
    BY <1>1, <2>1, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, <1>16,
       <1>17, <1>18, <1>19, <1>20, PTL
<1>2. QED BY <1>1, PTL

\* The bound splits of the acquittal ladder, standalone: the induction
\* hypothesis in the consumer's scope bans necessitation there.
LEMMA SendAcquittalBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW k \in Nat, NEW n \in Nat
    PROVE  /\ [](TypeOK =>
                     (/\ HasAcceptedSendAt(cId, k)
                      /\ ~IsSendAcquittedAt(cId, k)
                      /\ k - WriteDonesReturned(cId) <= n + 1
                      => \/ /\ HasAcceptedSendAt(cId, k)
                            /\ ~IsSendAcquittedAt(cId, k)
                            /\ k - WriteDonesReturned(cId) <= n
                         \/ /\ HasAcceptedSendAt(cId, k)
                            /\ ~IsSendAcquittedAt(cId, k)
                            /\ k - WriteDonesReturned(cId) = n + 1))
           /\ [](TypeOK =>
                     (/\ HasAcceptedSendAt(cId, k)
                      /\ ~IsSendAcquittedAt(cId, k)
                      /\ k - WriteDonesReturned(cId) = n
                      => /\ HasAcceptedSendAt(cId, k)
                         /\ ~IsSendAcquittedAt(cId, k)
                         /\ k - WriteDonesReturned(cId) <= n))
<1>1. TypeOK =>
          (/\ HasAcceptedSendAt(cId, k)
           /\ ~IsSendAcquittedAt(cId, k)
           /\ k - WriteDonesReturned(cId) <= n + 1
           => \/ /\ HasAcceptedSendAt(cId, k)
                 /\ ~IsSendAcquittedAt(cId, k)
                 /\ k - WriteDonesReturned(cId) <= n
              \/ /\ HasAcceptedSendAt(cId, k)
                 /\ ~IsSendAcquittedAt(cId, k)
                 /\ k - WriteDonesReturned(cId) = n + 1)
    BY SMT DEF TypeOK, L0!TypeOK
<1>2. TypeOK =>
          (/\ HasAcceptedSendAt(cId, k)
           /\ ~IsSendAcquittedAt(cId, k)
           /\ k - WriteDonesReturned(cId) = n
           => /\ HasAcceptedSendAt(cId, k)
              /\ ~IsSendAcquittedAt(cId, k)
              /\ k - WriteDonesReturned(cId) <= n)
    BY SMT DEF TypeOK, L0!TypeOK
<1>3. QED BY <1>1, <1>2, PTL

\* The full ladder, level-0 measure-induction style.
THEOREM SendAcquittedFor ==
    ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))
<1>0. k \in Nat
    BY DEF L0!PositiveNaturals
<1> DEFINE P == HasAcceptedSendAt(cId, k) /\ ~IsSendAcquittedAt(cId, k)
           M == k - WriteDonesReturned(cId)
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k))
  <2> DEFINE Ind(n) == (P /\ M <= n) ~> IsSendAcquittedAt(cId, k)
  <2>2. Ind(0)
    <3>1. TypeOK => ~(P /\ M <= 0)
      BY <1>0, SMT DEF TypeOK, L0!TypeOK
    <3>2. QED BY <1>1, <3>1, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE Ind(n + 1)
    <3>1. (P /\ M = n + 1) ~>
              (IsSendAcquittedAt(cId, k) \/ (P /\ M = n))
      BY <1>0, <1>1, <2>3, SendDescentFor, PTL
    <3>2. [](TypeOK => (P /\ M <= n + 1 =>
                            (P /\ M <= n) \/ (P /\ M = n + 1)))
      BY <1>0, <2>3, SendAcquittalBoundSplit, PTL
    <3>3. [](TypeOK => (P /\ M = n => P /\ M <= n))
      BY <1>0, <2>3, SendAcquittalBoundSplit, PTL
    <3>4. []TypeOK
      BY <1>1, PTL
    <3>5. QED BY <2>3, <3>1, <3>2, <3>3, <3>4, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(600)
  <2>5. Ind(k)
    BY <1>0, <2>4
  <2>6. [](TypeOK /\ FfiCallInv =>
               (HasAcceptedSendAt(cId, k) =>
                    (P /\ M <= k) \/ IsSendAcquittedAt(cId, k)))
    <3>0. TypeOK /\ FfiCallInv =>
              /\ write_dones_emitted \in [CallIds -> Nat]
              /\ write_done_callback_running \in [CallIds -> BOOLEAN]
              /\ submitted \in [CallIds -> Seq(Messages)]
              /\ (IsWriteDoneCallbackRunning(cId) =>
                      write_dones_emitted[cId] >= 1)
      BY Zenon DEF TypeOK, L0!TypeOK, FfiCallInv,
          RunningWriteDoneWasEmitted
    <3>1. TypeOK /\ FfiCallInv =>
              (HasAcceptedSendAt(cId, k) =>
                   (P /\ M <= k) \/ IsSendAcquittedAt(cId, k))
      BY <1>0, <3>0, SMT
    <3>2. QED BY <1>1, <3>1, PTL
  <2>7. [](TypeOK /\ FfiCallInv)
    BY <1>1, PTL
  <2>8. QED BY <1>1, <2>5, <2>6, <2>7, PTL DEF Ind
<1>2. QED BY <1>1, PTL

THEOREM SendAcquittalProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))
<1>100. StrongInv => TypeOK /\ FfiCallInv
    BY Zenon DEF StrongInv
<1>10. [](StrongInv => TypeOK /\ FfiCallInv)
    BY <1>100, PTL
<1>11. [](NextSafe => Next)
    BY SafeIsNext, PTL
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k))
  <2>1. /\ [](TypeOK /\ FfiCallInv)
        /\ [][Next]_vars
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))
    BY SendAcquittedFor, PTL
  <2>2. QED BY <1>1, <1>10, <1>11, <2>1, PTL
<1>2. QED BY <1>1, PTL

THEOREM SendAcquittalFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))
<1>1. /\ Init
      /\ [][NextSafe]_vars
      => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED
    BY <1>1, SendAcquittalProgressSafeFor, PTL

(***************************************************************************)
(* PAYLOADS ARE CONSUMED                                                   *)
(* Per payload: the host hypothesis WF(HostConsumesEvent(cId)) for the  *)
(* very payload k is the only fairness consumed.  A held payload stays     *)
(* held until its own consumption (a delivery adds the fresh index above   *)
(* every held one, another consumption removes only its own index), so     *)
(* the guarantee is a plain WF1.                                           *)
(***************************************************************************)

THEOREM PayloadsProgressSafeFor ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((HostOwnsPayload(cId, k)) ~>
                   (~HostOwnsPayload(cId, k)))
<1>100. StrongInv => TypeOK
    BY Zenon DEF StrongInv
<1>10. [](StrongInv => TypeOK)
    BY <1>100, PTL
<1>11. [](NextSafe => Next)
    BY SafeIsNext, PTL
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(HostConsumesEvent(cId))
      PROVE  (HostOwnsPayload(cId, k)) ~>
                 (~HostOwnsPayload(cId, k))
  <2>1. /\ []TypeOK
        /\ [][Next]_vars
        /\ WF_vars(HostConsumesEvent(cId))
        => ((HostOwnsPayload(cId, k)) ~>
                (~HostOwnsPayload(cId, k)))
    BY PayloadConsumedFor, PTL
  <2>2. QED BY <1>1, <1>10, <1>11, <2>1, PTL
<1>2. QED BY <1>1, PTL

THEOREM PayloadsFairnessRequirement ==
    ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(HostConsumesEvent(cId))
           => ((HostOwnsPayload(cId, k)) ~>
                   (~HostOwnsPayload(cId, k)))
<1>1. /\ Init
      /\ [][NextSafe]_vars
      => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED
    BY <1>1, PayloadsProgressSafeFor, PTL

(***************************************************************************)
(* SHUTDOWN EMITS ITS EVENT                                                *)
(* A stopping runtime's channels finish closing, the calls die and quiet   *)
(* down, the runtime quiesces, and SHUTDOWN_COMPLETE goes out.  All of it  *)
(* on binding-owned fairness alone.                                        *)
(***************************************************************************)

\* A closing channel only ever moves to closed.
LEMMA ClosingUnlessClosed ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ TypeOK
           /\ IsClosingChannel(chId)
           /\ [Next]_vars
           => \/ (IsClosingChannel(chId))'
              \/ (IsClosedChannel(chId))'
<1>1. ASSUME TypeOK, IsClosingChannel(chId), [Next]_vars
      PROVE  \/ (IsClosingChannel(chId))'
             \/ (IsClosedChannel(chId))'
  <2>1. CASE Next
    <3>1. CASE \/ NextSafeChannelOnly
               \/ NextSafeChannelCall
               \/ NextSafeRuntimeChannel
      BY <1>1, <3>1, SMTT(45)
      DEF NextSafeChannelOnly, NextSafeChannelCall, NextSafeRuntimeChannel,
          ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
          RuntimeBeginShutdown, L0!ChannelCreate, L0!ChannelStartClosing,
          L0!ChannelFinishClosing, L0!RuntimeBeginShutdown,
          L0!ChannelsOf, L0!CallsOf, RequestCancellationOfActiveCalls,
          IsClosingChannel, IsClosedChannel,
          TypeOK, L0!TypeOK, L0!ChannelStates
    <3>2. QED
      BY <1>1, <3>1, OnlyChannelStepsWriteChannelState, Zenon
      DEF IsClosingChannel, IsClosedChannel
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA ClosedSticky ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ TypeOK
           /\ IsClosedChannel(chId)
           /\ [Next]_vars
           => (IsClosedChannel(chId))'
<1>1. ASSUME TypeOK, IsClosedChannel(chId), [Next]_vars
      PROVE  (IsClosedChannel(chId))'
  <2>1. CASE Next
    <3>1. CASE \/ NextSafeChannelOnly
               \/ NextSafeChannelCall
               \/ NextSafeRuntimeChannel
      BY <1>1, <3>1, SMTT(45)
      DEF NextSafeChannelOnly, NextSafeChannelCall, NextSafeRuntimeChannel,
          ChannelCreate, ChannelStartClosing, ChannelFinishClosing,
          RuntimeBeginShutdown, L0!ChannelCreate, L0!ChannelStartClosing,
          L0!ChannelFinishClosing, L0!RuntimeBeginShutdown,
          L0!ChannelsOf, L0!CallsOf, RequestCancellationOfActiveCalls,
          IsClosingChannel, IsClosedChannel,
          TypeOK, L0!TypeOK, L0!ChannelStates
    <3>2. QED
      BY <1>1, <3>1, OnlyChannelStepsWriteChannelState, Zenon
      DEF IsClosedChannel
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* Every closing channel eventually closes, on the close and drain
\* fairness alone.
THEOREM ChannelEventuallyCloses ==
    ASSUME NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ [][Next]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           => ((IsClosingChannel(chId)) ~>
                   (IsClosedChannel(chId)))
<1>10. [](TypeOK /\ IsClosingChannel(chId) /\ [Next]_vars =>
              \/ (IsClosingChannel(chId))'
              \/ (IsClosedChannel(chId))')
    BY ClosingUnlessClosed, PTL
<1>11. [](TypeOK /\ IsClosingChannel(chId) /\
              (\A cId \in CallIds :
                   call_channel[cId] = chId => ~L0!IsActiveCall(cId))
              => ENABLED <<ChannelFinishClosing(chId)>>_vars)
    BY CFC1EnabledBridge, PTL
<1>12. [](TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
              (channel_state[chId] # "closing")')
    BY CFC1KillsClosing, PTL
<1>13. [](TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
              <<L0!ChannelFinishClosing(chId)>>_l0_vars)
    BY CFC1StepProjects, PTL
<1>140. TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
            (~(IsClosingChannel(chId)))'
    BY CFC1KillsClosing, Zenon
<1>14. [](TypeOK /\ <<ChannelFinishClosing(chId)>>_vars =>
              (~(IsClosingChannel(chId)))')
    BY <1>140, PTL
<1>1. ASSUME []IndInv,
             [][Next]_vars,
             WF_vars(ChannelFinishClosing(chId)),
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
      PROVE  (IsClosingChannel(chId)) ~>
                 (IsClosedChannel(chId))
  <2>1. []TypeOK
    BY <1>1, IndInvParts, PTL
  <2>2. /\ []IndInv
        /\ [][Next]_vars
        /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
        /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
        /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
        => []([](IsClosingChannel(chId)) =>
                  <>[](\A cId \in CallIds :
                           call_channel[cId] = chId =>
                               ~L0!IsActiveCall(cId)))
    BY AllCallsDrainForBoxed
  <2>3. []([](IsClosingChannel(chId)) =>
               <>[](\A cId \in CallIds :
                        call_channel[cId] = chId =>
                            ~L0!IsActiveCall(cId)))
    BY <1>1, <2>2, BoxedDCFairness, BoxedCBRFairness,
       BoxedEWFairness, BoxedWRFairness, PTL
  <2>4. ASSUME <>(IsClosingChannel(chId) /\
                      [](~(IsClosedChannel(chId))))
        PROVE  FALSE
    <3>1. <>[](IsClosingChannel(chId))
      BY <1>1, <2>1, <2>4, <1>10, PTL
    <3>2. <>[](\A cId \in CallIds :
                   call_channel[cId] = chId => ~L0!IsActiveCall(cId))
      BY <3>1, <2>3, PTL
    <3>3. QED
      BY <1>1, <2>1, <2>4, <3>1, <3>2, <1>11, <1>14, PTL
  <2>5. QED
    BY <2>4, PTL
<1>2. QED BY <1>1, PTL

\* Ownership by a non-running runtime is stable, and a stopping runtime
\* keeps no open channel.
LEMMA StoppingChannelsSettle ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ StrongInv
           /\ IsStoppingRuntime(rtId)
           /\ channel_runtime[chId] = rtId
           => channel_state[chId] \in {"closing", "closed"}
<1>1. QED
    BY NoneNotInRuntimeIds, SMT
    DEF StrongInv, L0!StrongInv, L0!StructuralInv,
        L0!ChannelLifecycleInv, L0!UsedChannels,
        L0!ChannelSentinelEquivalence,
        TypeOK, L0!TypeOK, L0!RuntimeStates, L0!ChannelStates

LEMMA OwnershipStableWhileStopping ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ TypeOK
           /\ IsStoppingRuntime(rtId)
           /\ channel_runtime[chId] # rtId
           /\ [Next]_vars
           => (channel_runtime[chId] # rtId)'
<1>1. ASSUME TypeOK, IsStoppingRuntime(rtId),
             channel_runtime[chId] # rtId, [Next]_vars
      PROVE  (channel_runtime[chId] # rtId)'
  <2>1. CASE Next
    <3>1. CASE \E ch \in ChannelIds, rt \in RuntimeIds :
               ChannelCreate(ch, rt)
      BY <1>1, <3>1, NoneNotInRuntimeIds, SMT
      DEF ChannelCreate, L0!ChannelCreate, StrongInv, L0!StrongInv,
          L0!StructuralInv, L0!ChannelSentinelEquivalence,
          L0!ChannelStates, IsStoppingRuntime, TypeOK, L0!TypeOK
    <3>2. QED
      BY <1>1, <3>1, OnlyChannelCreateWritesOwnership, Zenon
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA OwnedStable ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ TypeOK
           /\ StrongInv
           /\ channel_runtime[chId] = rtId
           /\ [Next]_vars
           => (channel_runtime[chId] = rtId)'
<1>1. ASSUME TypeOK, StrongInv, channel_runtime[chId] = rtId, [Next]_vars
      PROVE  (channel_runtime[chId] = rtId)'
  <2>1. CASE Next
    <3>1. CASE \E ch \in ChannelIds, rt \in RuntimeIds :
               ChannelCreate(ch, rt)
      BY <1>1, <3>1, NoneNotInRuntimeIds, SMT
      DEF ChannelCreate, L0!ChannelCreate, StrongInv, L0!StrongInv,
          L0!StructuralInv, L0!ChannelSentinelEquivalence,
          L0!ChannelStates, IsStoppingRuntime, TypeOK, L0!TypeOK
    <3>2. QED
      BY <1>1, <3>1, OnlyChannelCreateWritesOwnership, Zenon
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

\* While a runtime stays stopping and unemitted, each channel ends
\* owned-implies-closed, forever.
THEOREM ChannelSettlesForL1 ==
    ASSUME NEW rtId \in RuntimeIds, NEW chId \in ChannelIds
    PROVE  /\ []IndInv
           /\ []StrongInv
           /\ [][Next]_vars
           /\ WF_vars(ChannelFinishClosing(chId))
           /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           /\ [](IsStoppingRuntime(rtId))
           => <>[](channel_runtime[chId] = rtId =>
                       IsClosedChannel(chId))
<1>100. StrongInv /\ IsStoppingRuntime(rtId) /\
              channel_runtime[chId] = rtId =>
                  channel_state[chId] \in {"closing", "closed"}
    BY StoppingChannelsSettle, Zenon
<1>10. [](StrongInv /\ IsStoppingRuntime(rtId) /\
              channel_runtime[chId] = rtId =>
                  channel_state[chId] \in {"closing", "closed"})
    BY <1>100, PTL
<1>110. TypeOK /\ IsStoppingRuntime(rtId) /\
              channel_runtime[chId] # rtId /\ [Next]_vars =>
                  (channel_runtime[chId] # rtId)'
    BY OwnershipStableWhileStopping, Zenon
<1>11. [](TypeOK /\ IsStoppingRuntime(rtId) /\
              channel_runtime[chId] # rtId /\ [Next]_vars =>
                  (channel_runtime[chId] # rtId)')
    BY <1>110, PTL
<1>120. TypeOK /\ StrongInv /\ channel_runtime[chId] = rtId /\
              [Next]_vars => (channel_runtime[chId] = rtId)'
    BY OwnedStable, Zenon
<1>12. [](TypeOK /\ StrongInv /\ channel_runtime[chId] = rtId /\
              [Next]_vars =>
              (channel_runtime[chId] = rtId)')
    BY <1>120, PTL
<1>130. TypeOK /\ IsClosedChannel(chId) /\ [Next]_vars =>
              (IsClosedChannel(chId))'
    BY ClosedSticky, Zenon
<1>13. [](TypeOK /\ IsClosedChannel(chId) /\ [Next]_vars =>
              (IsClosedChannel(chId))')
    BY <1>130, PTL
<1>140. TypeOK /\ IsClosingChannel(chId) /\ [Next]_vars =>
              \/ (IsClosingChannel(chId))'
              \/ (IsClosedChannel(chId))'
    BY ClosingUnlessClosed, Zenon
<1>14. [](TypeOK /\ IsClosingChannel(chId) /\ [Next]_vars =>
              \/ (IsClosingChannel(chId))'
              \/ (IsClosedChannel(chId))')
    BY <1>140, PTL
<1>150. channel_state[chId] \in {"closing", "closed"} =>
              IsClosingChannel(chId) \/
              IsClosedChannel(chId)
    BY Zenon, Zenon
<1>15. [](channel_state[chId] \in {"closing", "closed"} =>
              IsClosingChannel(chId) \/
              IsClosedChannel(chId))
    BY <1>150, PTL
<1>1. ASSUME []IndInv,
             []StrongInv,
             [][Next]_vars,
             WF_vars(ChannelFinishClosing(chId)),
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId)),
             [](IsStoppingRuntime(rtId))
      PROVE  <>[](channel_runtime[chId] = rtId =>
                      IsClosedChannel(chId))
  <2>1. []TypeOK
    BY <1>1, IndInvParts, PTL
  <2>2. (IsClosingChannel(chId)) ~>
            (IsClosedChannel(chId))
    <3>1. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(ChannelFinishClosing(chId))
          /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
          /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
          /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
          /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
          => ((IsClosingChannel(chId)) ~>
                  (IsClosedChannel(chId)))
      BY ChannelEventuallyCloses, IsaT(600)
    <3>2. QED BY <1>1, <3>1, PTL
  <2>3. QED
    BY <1>1, <2>1, <2>2, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15, PTL
<1>2. QED BY <1>1, PTL

THEOREM AllChannelsSettleL1 ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  (\A chId \in ChannelIds :
                <>[](channel_runtime[chId] = rtId =>
                         IsClosedChannel(chId)))
           => <>[](\A chId \in ChannelIds :
                       channel_runtime[chId] = rtId =>
                           IsClosedChannel(chId))
<1>0. USE FiniteChannelIds DEF FiniteChannelIds
<1> DEFINE G(c) == channel_runtime[c] = rtId =>
                       IsClosedChannel(c)
           K(c) == <>[]G(c)
           I(T) == (\A chId \in T : K(chId)) =>
                       <>[](\A chId \in T : G(chId))
<1>1. I({})
  <2>1. \A chId \in {} : G(chId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET ChannelIds, NEW x \in ChannelIds \ T
       PROVE <>[](\A chId \in T : G(chId)) /\ <>[]G(x) =>
                 <>[](\A chId \in T \cup {x} : G(chId))
  <2>1. (\A chId \in T : G(chId)) /\ G(x) =>
            (\A chId \in T \cup {x} : G(chId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET ChannelIds, IsFiniteSet(T), I(T),
             NEW x \in ChannelIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A chId \in T \cup {x} : K(chId)) =>
            (\A chId \in T : K(chId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A chId \in T : G(chId)) /\ <>[]G(x) =>
            <>[](\A chId \in T \cup {x} : G(chId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(ChannelIds)
    BY <1>1, <1>2, FS_Induction, IsaM("blast")
<1>4. QED BY <1>3, Zenon DEF I

\* Once every owned channel is closed and the runtime stops, the release
\* guard holds: the calls are terminal by the invariant.
LEMMA ReadyFromParts ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ StrongInv
           /\ IsStoppingRuntime(rtId)
           /\ (\A chId \in ChannelIds :
                   channel_runtime[chId] = rtId =>
                       IsClosedChannel(chId))
           => ReleaseReady(rtId)
<1>0. SUFFICES ASSUME StrongInv,
                      IsStoppingRuntime(rtId),
                      \A chId \in ChannelIds :
                          channel_runtime[chId] = rtId =>
                              IsClosedChannel(chId)
               PROVE  ReleaseReady(rtId)
    OBVIOUS
<1>1. \A chId \in L0!ChannelsOf(rtId) : IsClosedChannel(chId)
    BY <1>0, Zenon DEF L0!ChannelsOf
<1>2. ASSUME NEW chId \in L0!ChannelsOf(rtId),
             NEW cId \in L0!CallsOf(chId)
      PROVE  call_state[cId] = "terminal"
  <2>1. IsClosedChannel(chId)
    BY <1>1, <1>2
  <2>20. L0!CallLifecycleInv /\ L0!TypeOK
    BY <1>0, Zenon
    DEF StrongInv, L0!StrongInv, L0!StructuralInv, TypeOK
  <2>2. ~L0!IsActiveCall(cId)
    <3>1. SUFFICES ASSUME L0!IsActiveCall(cId) PROVE FALSE
        OBVIOUS
    <3>2. cId \in L0!UsedCalls
      BY <1>2, <3>1, <2>20, SMTT(120)
      DEF L0!UsedCalls, L0!IsUnusedCall, L0!IsActiveCall,
          L0!ActiveCallStates, L0!CallsOf, L0!ChannelsOf, L0!TypeOK,
          L0!CallStates
    <3>3. call_channel[cId] \in L0!ActiveChannels
      BY <1>2, <3>1, <3>2, <2>20, Zenon DEF L0!CallLifecycleInv
    <3>4. call_channel[cId] = chId
      BY <1>2, Zenon DEF L0!CallsOf, L0!ChannelsOf
    <3>5. QED
      BY <2>1, <3>3, <3>4, SMTT(120)
      DEF L0!ActiveChannels, L0!ActiveChannelStates, L0!TypeOK,
          L0!ChannelsOf
  <2>30. L0!CallSentinelEquivalence /\ L0!TypeOK
    BY <1>0, Zenon
    DEF StrongInv, L0!StrongInv, L0!StructuralInv, TypeOK
  <2>3. ~L0!IsUnusedCall(cId)
    BY <1>2, <2>30, NoneNotInChannelIds, SMT
    DEF L0!CallSentinelEquivalence, L0!ChannelsOf, L0!CallsOf,
        L0!IsUnusedCall, L0!TypeOK
  <2>4. QED
    BY <1>2, <2>2, <2>3, <2>20, SMT
    DEF L0!IsActiveCall, L0!IsUnusedCall, L0!ActiveCallStates,
        L0!ChannelsOf, L0!CallsOf, L0!TypeOK, L0!CallStates
<1>3. QED
    BY <1>0, <1>1, <1>2, Zenon DEF ReleaseReady

\* A stopping runtime stays stopping until its event goes out.
LEMMA StoppingUnlessEmitted ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsStoppingRuntime(rtId)
           /\ ~IsShutdownEventEmitted(rtId)
           /\ [NextSafe]_vars
           => \/ (IsStoppingRuntime(rtId) /\
                      ~IsShutdownEventEmitted(rtId))'
              \/ (IsShutdownEventEmitted(rtId))'
<1>1. ASSUME TypeOK, IsStoppingRuntime(rtId),
             ~IsShutdownEventEmitted(rtId), [NextSafe]_vars
      PROVE  \/ (IsStoppingRuntime(rtId) /\
                     ~IsShutdownEventEmitted(rtId))'
             \/ (IsShutdownEventEmitted(rtId))'
  <2>1. CASE NextSafe
    <3>0. /\ runtime_state \in [RuntimeIds -> L0!RuntimeStates]
          /\ shutdown_event_emitted \in [RuntimeIds -> BOOLEAN]
      BY <1>1, Zenon DEF TypeOK, L0!TypeOK
\* NextSafe excludes failure, so runtime_state moves only in the runtime
\* family - and there, release is guarded on the event this call has not
\* seen, while create and begin-shutdown demand a state rtId is not in.
    <3>1. CASE NextSafeRuntimeOnly
      BY <1>1, <3>0, <3>1, SMT
      DEF NextSafeRuntimeOnly, RuntimeCreate, RuntimeRelease,
          L0!RuntimeCreate, L0!RuntimeRelease, IsStoppingRuntime, IsShutdownEventEmitted,
          L0!RuntimeStates, ffi_vars
    <3>2. CASE NextSafeRuntimeChannel
      BY <1>1, <3>0, <3>2, SMT
      DEF NextSafeRuntimeChannel, RuntimeBeginShutdown,
          L0!RuntimeBeginShutdown, IsStoppingRuntime, IsShutdownEventEmitted, L0!RuntimeStates
    <3>3. CASE \/ NextSafeChannelOnly
               \/ NextSafeChannelCall
               \/ NextSafeCallOnly
      <4>1. UNCHANGED <<runtime_state, shutdown_event_emitted,
                        shutdown_callback_running>>
        BY <3>3, ChannelAndCallStepsKeepRuntime
      <4>2. QED
        BY <1>1, <4>1, SMT DEF IsStoppingRuntime, IsShutdownEventEmitted
    <3>4. CASE NextSafeFfiOnly
      <4>1. UNCHANGED l0_vars
        BY <3>4, FfiOnlyStepsKeepL0
      <4>2. runtime_state' = runtime_state
        BY <4>1, SMT DEF l0_vars, L0!vars, L0!RuntimeVars
      <4>3. CASE \E rt \in RuntimeIds : EmitShutdownComplete(rt)
        <5>1. PICK rt \in RuntimeIds : EmitShutdownComplete(rt)
          BY <4>3
        <5>2. shutdown_event_emitted' =
                  [shutdown_event_emitted EXCEPT ![rt] = TRUE]
          BY <5>1, Zenon DEF EmitShutdownComplete
        <5>3. QED
          BY <1>1, <3>0, <4>2, <5>2, SMT DEF IsStoppingRuntime, IsShutdownEventEmitted
      <4>4. CASE \E rt \in RuntimeIds : ShutdownCallbackReturns(rt)
        <5>1. UNCHANGED shutdown_event_emitted
          BY <4>4, Zenon DEF ShutdownCallbackReturns
        <5>2. QED
          BY <1>1, <4>2, <5>1, Zenon DEF IsStoppingRuntime, IsShutdownEventEmitted
      <4>5. QED
        BY <1>1, <4>2, <4>3, <4>4,
           OnlyShutdownStepsWriteShutdownFlags, NextDecomposition, Zenon
        DEF IsStoppingRuntime, IsShutdownEventEmitted, NextByFootprint, NextSafe
    <3>5. QED
      BY <2>1, <3>1, <3>2, <3>3, <3>4 DEF NextSafe, NextSafeRefining
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

THEOREM BoxedCFCFairness ==
    (\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
    <=> [](\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
<1>1. [](\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
      <=> \A chId \in ChannelIds : [](WF_vars(ChannelFinishClosing(chId)))
    OBVIOUS
<1>2. ASSUME NEW chId \in ChannelIds
      PROVE [](WF_vars(ChannelFinishClosing(chId)))
            <=> WF_vars(ChannelFinishClosing(chId))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600), PTL

\* Under a stopping runtime every channel settles closed, jointly.
THEOREM AllChannelsSettleCollected ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ []StrongInv
           /\ [][Next]_vars
           /\ (\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
           /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           /\ [](IsStoppingRuntime(rtId))
           => <>[](\A chId \in ChannelIds :
                       channel_runtime[chId] = rtId =>
                           IsClosedChannel(chId))
<1>1. ASSUME []IndInv,
             []StrongInv,
             [][Next]_vars,
             \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)),
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId)),
             [](IsStoppingRuntime(rtId))
      PROVE  <>[](\A chId \in ChannelIds :
                      channel_runtime[chId] = rtId =>
                          IsClosedChannel(chId))
  <2>1. ASSUME NEW chId \in ChannelIds
        PROVE  <>[](channel_runtime[chId] = rtId =>
                        IsClosedChannel(chId))
    <3>1. /\ []IndInv
          /\ []StrongInv
          /\ [][Next]_vars
          /\ WF_vars(ChannelFinishClosing(chId))
          /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
          /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
          /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
          /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
          /\ [](IsStoppingRuntime(rtId))
          => <>[](channel_runtime[chId] = rtId =>
                      IsClosedChannel(chId))
      BY ChannelSettlesForL1, PTL
    <3>2. QED BY <1>1, <3>1, IsaT(600), PTL
  <2>2. \A chId \in ChannelIds :
            <>[](channel_runtime[chId] = rtId =>
                     IsClosedChannel(chId))
    BY <2>1
  <2>3. QED BY <2>2, AllChannelsSettleL1
<1>2. QED BY <1>1, PTL

THEOREM AllChannelsSettleBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []IndInv
           /\ []StrongInv
           /\ [][Next]_vars
           /\ [](\A chId \in ChannelIds :
                     WF_vars(ChannelFinishClosing(chId)))
           /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
           /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
           /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
           /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
           => []([](IsStoppingRuntime(rtId)) =>
                     <>[](\A chId \in ChannelIds :
                              channel_runtime[chId] = rtId =>
                                  IsClosedChannel(chId)))
<1>1. ASSUME []IndInv,
             []StrongInv,
             [][Next]_vars,
             [](\A chId \in ChannelIds :
                    WF_vars(ChannelFinishClosing(chId))),
             [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId))),
             [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))),
             [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId))),
             [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
      PROVE  []([](IsStoppingRuntime(rtId)) =>
                    <>[](\A chId \in ChannelIds :
                             channel_runtime[chId] = rtId =>
                                 IsClosedChannel(chId)))
  <2>1. [][]IndInv
    BY <1>1, PTL
  <2>2. [][]StrongInv
    BY <1>1, PTL
  <2>3. [][][Next]_vars
    BY <1>1, PTL
  <2>4. [][](\A chId \in ChannelIds :
                 WF_vars(ChannelFinishClosing(chId)))
    BY <1>1, BoxedCFCFairness, PTL
  <2>5. [][](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
    BY <1>1, BoxedDCFairness, PTL
  <2>6. [][](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
    BY <1>1, BoxedCBRFairness, PTL
  <2>7. [][](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
    BY <1>1, BoxedEWFairness, PTL
  <2>8. [][](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
    BY <1>1, BoxedWRFairness, PTL
  <2>9. /\ []IndInv
        /\ []StrongInv
        /\ [][Next]_vars
        /\ (\A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)))
        /\ (\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
        /\ (\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ (\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
        /\ (\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
        /\ [](IsStoppingRuntime(rtId))
        => <>[](\A chId \in ChannelIds :
                    channel_runtime[chId] = rtId =>
                        IsClosedChannel(chId))
    BY AllChannelsSettleCollected, PTL
  <2>10. QED
    BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, PTL
<1>2. QED BY <1>1

THEOREM ShutdownEmitProgressSafeFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ []StrongInv
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => ((IsStoppingRuntime(rtId)) ~>
                   IsShutdownEventEmitted(rtId))
<1>10. [](StrongInv => IndInv)
    BY StrongInvImpliesIndInv, PTL
<1>11. [](NextSafe => Next)
    BY SafeIsNext, PTL
<1>120. TypeOK /\ IsStoppingRuntime(rtId) /\
            ~IsShutdownEventEmitted(rtId) /\ [NextSafe]_vars =>
                \/ (IsStoppingRuntime(rtId) /\
                        ~IsShutdownEventEmitted(rtId))'
                \/ (IsShutdownEventEmitted(rtId))'
    BY StoppingUnlessEmitted, Zenon
<1>12. [](TypeOK /\ IsStoppingRuntime(rtId) /\
              ~IsShutdownEventEmitted(rtId) /\ [NextSafe]_vars =>
                  \/ (IsStoppingRuntime(rtId) /\
                          ~IsShutdownEventEmitted(rtId))'
                  \/ (IsShutdownEventEmitted(rtId))')
    BY <1>120, PTL
<1>130. StrongInv /\ IsStoppingRuntime(rtId) /\
            (\A chId \in ChannelIds :
                 channel_runtime[chId] = rtId =>
                     IsClosedChannel(chId))
            => ReleaseReady(rtId)
    BY ReadyFromParts, Zenon
<1>13. [](StrongInv /\ IsStoppingRuntime(rtId) /\
              (\A chId \in ChannelIds :
                   channel_runtime[chId] = rtId =>
                       IsClosedChannel(chId))
              => ReleaseReady(rtId))
    BY <1>130, PTL
<1>140. TypeOK /\ IsStoppingRuntime(rtId) /\
            ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
            => ENABLED <<EmitShutdownComplete(rtId)>>_vars
    BY EmitEnabledBridge, Zenon
<1>14. [](TypeOK /\ IsStoppingRuntime(rtId) /\
              ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
              => ENABLED <<EmitShutdownComplete(rtId)>>_vars)
    BY <1>140, PTL
<1>150. TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
            (IsShutdownEventEmitted(rtId))'
    BY EmitSets, Zenon
<1>15. [](TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
              (IsShutdownEventEmitted(rtId))')
    BY <1>150, PTL
<1>1. ASSUME []StrongInv,
             [][NextSafe]_vars,
             WF_vars(EmitShutdownComplete(rtId)),
             \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId)),
             \A cId \in CallIds : WF_vars(DeliverCancelled(cId)),
             \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)),
             \A cId \in CallIds : WF_vars(EmitWriteDone(cId)),
             \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
      PROVE  (IsStoppingRuntime(rtId)) ~>
                 IsShutdownEventEmitted(rtId)
  <2>1. []IndInv
    BY <1>1, <1>10, PTL
  <2>2. [][Next]_vars
    BY <1>1, <1>11, PTL
  <2>3. []TypeOK
    BY <2>1, IndInvParts, PTL
  <2>4. /\ []IndInv
        /\ []StrongInv
        /\ [][Next]_vars
        /\ [](\A chId \in ChannelIds :
                  WF_vars(ChannelFinishClosing(chId)))
        /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
        /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
        /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
        => []([](IsStoppingRuntime(rtId)) =>
                  <>[](\A chId \in ChannelIds :
                           channel_runtime[chId] = rtId =>
                               IsClosedChannel(chId)))
    BY AllChannelsSettleBoxed
  <2>5. []([](IsStoppingRuntime(rtId)) =>
               <>[](\A chId \in ChannelIds :
                        channel_runtime[chId] = rtId =>
                            IsClosedChannel(chId)))
    BY <1>1, <2>1, <2>2, <2>4, BoxedCFCFairness, BoxedDCFairness,
       BoxedCBRFairness, BoxedEWFairness, BoxedWRFairness, PTL
  <2>6. /\ []IndInv
        /\ [][Next]_vars
        /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
        => []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
    BY RuntimeQuiescesBoxed
  <2>7. []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
    BY <1>1, <2>1, <2>2, <2>6, BoxedCBRFairness, PTL
  <2>8. ASSUME <>(IsStoppingRuntime(rtId) /\
                      [](~IsShutdownEventEmitted(rtId)))
        PROVE  FALSE
    <3>1. <>[](IsStoppingRuntime(rtId) /\
                   ~IsShutdownEventEmitted(rtId))
      BY <1>1, <2>3, <2>8, <1>12, PTL
    <3>2. <>[](\A chId \in ChannelIds :
                   channel_runtime[chId] = rtId =>
                       IsClosedChannel(chId))
      BY <3>1, <2>5, PTL
    <3>3. <>[]ReleaseReady(rtId)
      BY <1>1, <3>1, <3>2, <1>13, PTL
    <3>4. <>[]IsRuntimeDrained(rtId)
      BY <3>3, <2>7, PTL
    <3>5. QED
      BY <1>1, <2>3, <2>8, <3>1, <3>4, <1>14, <1>15, PTL
  <2>9. QED
    BY <2>8, PTL
<1>2. QED BY <1>1, PTL

THEOREM ShutdownEmitFairnessRequirement ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ Init
           /\ [][NextSafe]_vars
           /\ WF_vars(EmitShutdownComplete(rtId))
           /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
           /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
           /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
           /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
           /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
           => ((IsStoppingRuntime(rtId)) ~>
                   IsShutdownEventEmitted(rtId))
<1>1. /\ Init
      /\ [][NextSafe]_vars
      => []StrongInv
    BY NominalBehaviorEstablishesStrongInv
<1>2. QED
    BY <1>1, ShutdownEmitProgressSafeFor, PTL

(***************************************************************************)
(* THE GUARANTEES UNDER SPEC                                               *)
(* The first three weaken the unconditional full-world drains; the        *)
(* shutdown one lifts through failure: failure is absorbing, so in the    *)
(* escape-free scenario health holds from the start and the safe chain    *)
(* replays.                                                                *)
(***************************************************************************)

THEOREM CancellationCompletesHolds == Spec => CancellationCompletes
<1>1. ASSUME Spec
      PROVE  CancellationCompletes
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. ASSUME NEW cId \in CallIds
        PROVE  (L0!IsActiveCall(cId) /\ IsCancelRequested(cId) /\
                    L0!NotFailed) ~>
                   (~L0!IsActiveCall(cId) \/ ~L0!NotFailed)
    <3>1. /\ WF_vars(DeliverCancelled(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
      BY <2>0, FairnessAtCall, IsaT(600), PTL
    <3>2. /\ []IndInv
          /\ [][Next]_vars
          /\ WF_vars(DeliverCancelled(cId))
          /\ WF_vars(DeliveryCallbackReturns(cId))
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          => ((L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
                  ~L0!IsActiveCall(cId))
      BY CancelledCallDies, PTL
    <3>3. (L0!IsActiveCall(cId) /\ IsCancelRequested(cId)) ~>
              ~L0!IsActiveCall(cId)
      BY <2>0, <2>1, <3>1, <3>2, PTL
    <3>4. QED BY <3>3, PTL
  <2>3. QED
    BY <2>2, IsaT(600), PTL DEF CancellationCompletes
<1>2. QED BY <1>1

THEOREM SendsEventuallyAcquittedHolds == Spec => SendsEventuallyAcquitted
<1>1. ASSUME Spec
      PROVE  SendsEventuallyAcquitted
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>10. [](TypeOK /\ FfiCallInv)
    BY <2>1, IndInvParts, PTL
  <2>2. ASSUME NEW cId \in CallIds, NEW k \in L0!PositiveNaturals
        PROVE  (HasAcceptedSendAt(cId, k) /\ L0!NotFailed) ~>
                   (IsSendAcquittedAt(cId, k) \/ ~L0!NotFailed)
    <3>1. /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
      BY <2>0, FairnessAtCall, IsaT(600), PTL
    <3>2. /\ [](TypeOK /\ FfiCallInv)
          /\ [][Next]_vars
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
          => ((HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k)))
      BY SendAcquittedFor, PTL
    <3>3. (HasAcceptedSendAt(cId, k)) ~> (IsSendAcquittedAt(cId, k))
      BY <2>0, <2>10, <3>1, <3>2, PTL
    <3>4. QED BY <3>3, PTL
  <2>3. QED
    BY <2>2, IsaT(600), PTL DEF SendsEventuallyAcquitted, SendAcquittedAt
<1>2. QED BY <1>1

THEOREM PayloadsEventuallyConsumedHolds == Spec => PayloadsEventuallyConsumed
<1>1. ASSUME Spec
      PROVE  PayloadsEventuallyConsumed
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>10. []TypeOK
    BY <2>1, IndInvParts, PTL
  <2>2. ASSUME NEW cId \in CallIds, NEW k \in PayloadIndices
        PROVE  (HostOwnsPayload(cId, k) /\ L0!NotFailed) ~>
                   (~HostOwnsPayload(cId, k) \/ ~L0!NotFailed)
    <3>1. WF_vars(HostConsumesEvent(cId))
      BY <2>0, FairnessAtCall, IsaT(600), PTL
    <3>2. /\ []TypeOK
          /\ [][Next]_vars
          /\ WF_vars(HostConsumesEvent(cId))
          => ((HostOwnsPayload(cId, k)) ~> (~HostOwnsPayload(cId, k)))
      BY PayloadConsumedFor, PTL
    <3>3. (HostOwnsPayload(cId, k)) ~> (~HostOwnsPayload(cId, k))
      BY <2>0, <2>10, <3>1, <3>2, PTL
    <3>4. QED BY <3>3, PTL
  <2>3. QED
    BY <2>2, IsaT(600), PTL DEF PayloadsEventuallyConsumed, PayloadConsumedAt
<1>2. QED BY <1>1

\* Failure is absorbing on the whole system.
LEMMA UnfailedSticky ==
    TypeOK /\ ~L0!NotFailed /\ [Next]_vars => (~L0!NotFailed)'
<1>1. ASSUME TypeOK, ~L0!NotFailed, [Next]_vars
      PROVE  (~L0!NotFailed)'
  <2>1. CASE Next
    BY <1>1, <2>1, FailedPersists
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars, L0!NotFailed
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA UmbrellaAt ==
    IndInv /\ L0!NotFailed => StrongInv
<1>1. QED
    BY Zenon DEF IndInv

\* The full-world stopping unless: safe steps keep it, the event may go
\* out, or some runtime fails.
LEMMA StoppingUnlessEmittedFull ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsStoppingRuntime(rtId)
           /\ ~IsShutdownEventEmitted(rtId)
           /\ [Next]_vars
           => \/ (IsStoppingRuntime(rtId) /\
                      ~IsShutdownEventEmitted(rtId))'
              \/ (IsShutdownEventEmitted(rtId))'
              \/ (~L0!NotFailed)'
<1>1. ASSUME TypeOK, IsStoppingRuntime(rtId),
             ~IsShutdownEventEmitted(rtId), [Next]_vars
      PROVE  \/ (IsStoppingRuntime(rtId) /\
                     ~IsShutdownEventEmitted(rtId))'
             \/ (IsShutdownEventEmitted(rtId))'
             \/ (~L0!NotFailed)'
  <2>1. CASE Next
\* Everything but a failure is the safe case already proved; a failure
\* is the third disjunct.
    <3>1. CASE NextSafe
      BY <1>1, <3>1, StoppingUnlessEmitted
    <3>2. CASE NextFail
      BY <1>1, <3>2, SMT
      DEF NextFail, RuntimeFail, L0!RuntimeFail, L0!NotFailed,
          L0!RuntimeStates, TypeOK, L0!TypeOK
    <3>3. CASE NextExplicitStutter
      BY <1>1, <3>3, SMT
      DEF NextExplicitStutter, RemainFailed, RemainReleased,
          L0!RemainFailed, L0!RemainReleased, L0!vars, l0_vars, ffi_vars,
          IsStoppingRuntime, IsShutdownEventEmitted
    <3>4. QED
      BY <2>1, <3>1, <3>2, <3>3, NextDecomposition DEF NextByFootprint
  <2>2. CASE UNCHANGED vars
    BY <1>1, <2>2, SMT DEF vars, l0_vars, L0!vars, ffi_vars
  <2>3. QED BY <1>1, <2>1, <2>2
<1>2. QED BY <1>1

LEMMA StoppingUnlessEmittedFullBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](TypeOK /\ IsStoppingRuntime(rtId) /\
                  ~IsShutdownEventEmitted(rtId) /\ [Next]_vars =>
                      \/ (IsStoppingRuntime(rtId) /\
                              ~IsShutdownEventEmitted(rtId))'
                      \/ (IsShutdownEventEmitted(rtId))'
                      \/ (~L0!NotFailed)')
<1>10. TypeOK /\ IsStoppingRuntime(rtId) /\
           ~IsShutdownEventEmitted(rtId) /\ [Next]_vars =>
               \/ (IsStoppingRuntime(rtId) /\
                       ~IsShutdownEventEmitted(rtId))'
               \/ (IsShutdownEventEmitted(rtId))'
               \/ (~L0!NotFailed)'
    BY StoppingUnlessEmittedFull, Zenon
<1>1. QED BY <1>10, PTL

LEMMA ReadyFromPartsBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](StrongInv /\ IsStoppingRuntime(rtId) /\
                  (\A chId \in ChannelIds :
                       channel_runtime[chId] = rtId =>
                           IsClosedChannel(chId))
                  => ReleaseReady(rtId))
<1>10. StrongInv /\ IsStoppingRuntime(rtId) /\
           (\A chId \in ChannelIds :
                channel_runtime[chId] = rtId =>
                    IsClosedChannel(chId))
           => ReleaseReady(rtId)
    BY ReadyFromParts, Zenon
<1>1. QED BY <1>10, PTL

LEMMA EmitEnabledBridgeBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](TypeOK /\ IsStoppingRuntime(rtId) /\
                  ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
                  => ENABLED <<EmitShutdownComplete(rtId)>>_vars)
<1>10. TypeOK /\ IsStoppingRuntime(rtId) /\
           ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
           => ENABLED <<EmitShutdownComplete(rtId)>>_vars
    BY EmitEnabledBridge, Zenon
<1>1. QED BY <1>10, PTL

LEMMA EmitSetsBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
                  (IsShutdownEventEmitted(rtId))')
<1>10. TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
           (IsShutdownEventEmitted(rtId))'
    BY EmitSets, Zenon
<1>1. QED BY <1>10, PTL

THEOREM ShutdownEventEmittedHolds == Spec => ShutdownEventEmitted
<1>1. ASSUME Spec
      PROVE  ShutdownEventEmitted
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. []TypeOK
    BY <2>1, IndInvParts, PTL
  <2>3. ASSUME NEW rtId \in RuntimeIds
        PROVE  (IsStoppingRuntime(rtId)) ~>
                   (IsShutdownEventEmitted(rtId) \/ ~L0!NotFailed)
    <3>0. /\ WF_vars(EmitShutdownComplete(rtId))
          /\ \A chId \in ChannelIds : WF_vars(ChannelFinishClosing(chId))
          /\ \A cId \in CallIds : WF_vars(DeliverCancelled(cId))
          /\ \A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId))
          /\ \A cId \in CallIds : WF_vars(EmitWriteDone(cId))
          /\ \A cId \in CallIds : WF_vars(WriteDoneReturns(cId))
      BY <2>0, FairnessAtRuntime, FairnessEverywhere, IsaT(600), PTL
    <3>10. [](TypeOK /\ ~L0!NotFailed /\ [Next]_vars =>
                  (~L0!NotFailed)')
      BY UnfailedSticky, PTL
    <3>11. [](IndInv /\ L0!NotFailed => StrongInv)
      BY UmbrellaAt, PTL
    <3>12. [](TypeOK /\ IsStoppingRuntime(rtId) /\
                  ~IsShutdownEventEmitted(rtId) /\ [Next]_vars =>
                      \/ (IsStoppingRuntime(rtId) /\
                              ~IsShutdownEventEmitted(rtId))'
                      \/ (IsShutdownEventEmitted(rtId))'
                      \/ (~L0!NotFailed)')
      BY StoppingUnlessEmittedFullBoxed
    <3>13. [](StrongInv /\ IsStoppingRuntime(rtId) /\
                  (\A chId \in ChannelIds :
                       channel_runtime[chId] = rtId =>
                           IsClosedChannel(chId))
                  => ReleaseReady(rtId))
      BY ReadyFromPartsBoxed
    <3>14. [](TypeOK /\ IsStoppingRuntime(rtId) /\
                  ~IsShutdownEventEmitted(rtId) /\ IsRuntimeDrained(rtId)
                  => ENABLED <<EmitShutdownComplete(rtId)>>_vars)
      BY EmitEnabledBridgeBoxed
    <3>15. [](TypeOK /\ <<EmitShutdownComplete(rtId)>>_vars =>
                  (IsShutdownEventEmitted(rtId))')
      BY EmitSetsBoxed
    <3>1. ASSUME <>(IsStoppingRuntime(rtId) /\
                        [](~IsShutdownEventEmitted(rtId) /\
                               L0!NotFailed))
          PROVE  FALSE
      <4>1. []L0!NotFailed
        BY <2>0, <2>2, <3>1, <3>10, PTL
      <4>2. []StrongInv
        BY <2>1, <4>1, <3>11, PTL
      <4>3. <>[](IsStoppingRuntime(rtId) /\
                     ~IsShutdownEventEmitted(rtId))
        BY <2>0, <2>2, <3>1, <3>12, <4>1, PTL
      <4>4. /\ []IndInv
            /\ []StrongInv
            /\ [][Next]_vars
            /\ [](\A chId \in ChannelIds :
                      WF_vars(ChannelFinishClosing(chId)))
            /\ [](\A cId \in CallIds : WF_vars(DeliverCancelled(cId)))
            /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
            /\ [](\A cId \in CallIds : WF_vars(EmitWriteDone(cId)))
            /\ [](\A cId \in CallIds : WF_vars(WriteDoneReturns(cId)))
            => []([](IsStoppingRuntime(rtId)) =>
                      <>[](\A chId \in ChannelIds :
                               channel_runtime[chId] = rtId =>
                                   IsClosedChannel(chId)))
        BY AllChannelsSettleBoxed
      <4>5. []([](IsStoppingRuntime(rtId)) =>
                   <>[](\A chId \in ChannelIds :
                            channel_runtime[chId] = rtId =>
                                IsClosedChannel(chId)))
        BY <2>0, <2>1, <3>0, <4>2, <4>4, BoxedCFCFairness,
           BoxedDCFairness, BoxedCBRFairness, BoxedEWFairness,
           BoxedWRFairness, PTL
      <4>6. <>[](\A chId \in ChannelIds :
                     channel_runtime[chId] = rtId =>
                         IsClosedChannel(chId))
        BY <4>3, <4>5, PTL
      <4>7. /\ []IndInv
            /\ [][Next]_vars
            /\ [](\A cId \in CallIds : WF_vars(DeliveryCallbackReturns(cId)))
            => []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
        BY RuntimeQuiescesBoxed
      <4>8. []([](ReleaseReady(rtId)) => <>[]IsRuntimeDrained(rtId))
        BY <2>0, <2>1, <3>0, <4>7, BoxedCBRFairness, PTL
      <4>9. <>[]ReleaseReady(rtId)
        BY <4>2, <4>3, <4>6, <3>13, PTL
      <4>10. <>[]IsRuntimeDrained(rtId)
        BY <4>8, <4>9, PTL
      <4>11. QED
        BY <2>0, <2>2, <3>0, <3>1, <4>3, <4>10, <3>14, <3>15, PTL
    <3>2. QED
      BY <3>1, PTL
  <2>4. QED
    BY <2>3, IsaT(600), PTL DEF ShutdownEventEmitted
<1>2. QED BY <1>1

\* The three callback-return guarantees.  Each is one step of the fairness
\* it rests on, and each turns an obligation the ABI imposes on the host
\* into something named that the manifest can check.
\* The three clearing facts, each one line off a transfer lemma.
LEMMA DeliveryReturnClears ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
               (~IsDeliveryCallbackRunning(cId))'
<1>1. QED
    BY DeliveryReturnTransfers, Zenon

LEMMA WriteDoneReturnClears ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ <<WriteDoneReturns(cId)>>_vars =>
               (~IsWriteDoneCallbackRunning(cId))'
<1>1. QED
    BY WriteDoneReturnsTransfers, Zenon

\* The three action facts are validities, so they are proved out here where
\* no temporal hypothesis is in scope - PTL cannot box a fact that holds
\* only of the behaviour under Spec.
THEOREM DeliveryCallbacksReturnHolds == Spec => DeliveryCallbacksReturn
<1>0. ASSUME NEW cId \in CallIds
      PROVE  /\ [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
                   ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
             /\ [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
                   (~IsDeliveryCallbackRunning(cId))')
             /\ [](TypeOK /\ IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
                   ~<<DeliveryCallbackReturns(cId)>>_vars => (IsDeliveryCallbackRunning(cId))')
  <2>1. TypeOK /\ IsDeliveryCallbackRunning(cId) => ENABLED <<DeliveryCallbackReturns(cId)>>_vars
    BY DeliveryCallbackReturnsEnabled, Zenon
  <2>2. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars => (~IsDeliveryCallbackRunning(cId))'
    BY DeliveryReturnClears, Zenon
  <2>3. TypeOK /\ IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
            ~<<DeliveryCallbackReturns(cId)>>_vars => (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>1. ASSUME Spec, NEW cId \in CallIds
      PROVE  IsDeliveryCallbackRunning(cId) ~> ~IsDeliveryCallbackRunning(cId)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []TypeOK
    BY <2>0, BehaviorEstablishesIndInv, IndInvParts, PTL
  <2>2. WF_vars(DeliveryCallbackReturns(cId))
    BY <2>0, FairnessAtCall, IsaT(600), PTL
  <2>3. QED
    BY <1>0, <2>0, <2>1, <2>2, PTL
<1>2. QED BY <1>1, Zenon DEF DeliveryCallbacksReturn

\* The three action facts are validities, so they are proved out here where
\* no temporal hypothesis is in scope - PTL cannot box a fact that holds
\* only of the behaviour under Spec.
THEOREM WriteDoneCallbacksReturnHolds == Spec => WriteDoneCallbacksReturn
<1>0. ASSUME NEW cId \in CallIds
      PROVE  /\ [](TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
                   ENABLED <<WriteDoneReturns(cId)>>_vars)
             /\ [](TypeOK /\ <<WriteDoneReturns(cId)>>_vars =>
                   (~IsWriteDoneCallbackRunning(cId))')
             /\ [](TypeOK /\ IsWriteDoneCallbackRunning(cId) /\ [Next]_vars /\
                   ~<<WriteDoneReturns(cId)>>_vars => (IsWriteDoneCallbackRunning(cId))')
  <2>1. TypeOK /\ IsWriteDoneCallbackRunning(cId) => ENABLED <<WriteDoneReturns(cId)>>_vars
    BY WriteDoneReturnsEnabled, Zenon
  <2>2. TypeOK /\ <<WriteDoneReturns(cId)>>_vars => (~IsWriteDoneCallbackRunning(cId))'
    BY WriteDoneReturnClears, Zenon
  <2>3. TypeOK /\ IsWriteDoneCallbackRunning(cId) /\ [Next]_vars /\
            ~<<WriteDoneReturns(cId)>>_vars => (IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>1. ASSUME Spec, NEW cId \in CallIds
      PROVE  IsWriteDoneCallbackRunning(cId) ~> ~IsWriteDoneCallbackRunning(cId)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []TypeOK
    BY <2>0, BehaviorEstablishesIndInv, IndInvParts, PTL
  <2>2. WF_vars(WriteDoneReturns(cId))
    BY <2>0, FairnessAtCall, IsaT(600)
  <2>3. QED
    BY <1>0, <2>0, <2>1, <2>2, PTL
<1>2. QED BY <1>1, Zenon DEF WriteDoneCallbacksReturn

\* The three action facts are validities, so they are proved out here where
\* no temporal hypothesis is in scope - PTL cannot box a fact that holds
\* only of the behaviour under Spec.
LEMMA RRCREnabled ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) =>
               ENABLED <<ResourcesReleasedCallbackReturns(rtId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF ResourcesReleasedCallbackReturns, vars, l0_vars, L0!vars, ffi_vars,
        TypeOK, L0!TypeOK

LEMMA RRCRClears ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
               (~IsResourcesReleasedCallbackRunning(rtId))'
<1>1. QED
    BY SMT DEF ResourcesReleasedCallbackReturns, vars, l0_vars,
        L0!vars, ffi_vars, TypeOK, L0!TypeOK

LEMMA ReleaseRunningStable ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ TypeOK
           /\ IsResourcesReleasedCallbackRunning(rtId)
           /\ [Next]_vars
           /\ ~<<ResourcesReleasedCallbackReturns(rtId)>>_vars
           => (IsResourcesReleasedCallbackRunning(rtId))'
<1>1. ASSUME TypeOK, IsResourcesReleasedCallbackRunning(rtId), [Next]_vars,
             ~<<ResourcesReleasedCallbackReturns(rtId)>>_vars
      PROVE  (IsResourcesReleasedCallbackRunning(rtId))'
  <2>0. ~ResourcesReleasedCallbackReturns(rtId)
    BY <1>1, SMT
    DEF ResourcesReleasedCallbackReturns, vars, l0_vars, L0!vars, ffi_vars,
        TypeOK, L0!TypeOK
  <2>1. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    BY <1>1, <2>1, SMT
    DEF EmitResourcesReleased, IsResourcesReleasedCallbackRunning,
        TypeOK, L0!TypeOK
  <2>2. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    BY <1>1, <2>0, <2>2, SMT
    DEF ResourcesReleasedCallbackReturns,
        IsResourcesReleasedCallbackRunning, TypeOK, L0!TypeOK
  <2>3. CASE UNCHANGED <<resources_released_emitted,
                         resources_released_callback_running>>
    BY <1>1, <2>3, Zenon DEF IsResourcesReleasedCallbackRunning
  <2>4. QED
    BY <1>1, <2>1, <2>2, <2>3, OnlyReleaseStepsWriteReleaseFlags, Zenon
<1>2. QED BY <1>1

THEOREM ResourcesReleasedCallbacksReturnHolds ==
    Spec => ResourcesReleasedCallbacksReturn
<1>0. ASSUME NEW rtId \in RuntimeIds
      PROVE  /\ [](TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) =>
                   ENABLED <<ResourcesReleasedCallbackReturns(rtId)>>_vars)
             /\ [](TypeOK /\
                   <<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
                       (~IsResourcesReleasedCallbackRunning(rtId))')
             /\ [](TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) /\
                   [Next]_vars /\
                   ~<<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
                       (IsResourcesReleasedCallbackRunning(rtId))')
  <2>1. TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) =>
            ENABLED <<ResourcesReleasedCallbackReturns(rtId)>>_vars
    BY RRCREnabled, Zenon
  <2>2. TypeOK /\ <<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
            (~IsResourcesReleasedCallbackRunning(rtId))'
    BY RRCRClears, Zenon
  <2>3. TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) /\ [Next]_vars /\
            ~<<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
                (IsResourcesReleasedCallbackRunning(rtId))'
    BY ReleaseRunningStable, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>1. ASSUME Spec, NEW rtId \in RuntimeIds
      PROVE  IsResourcesReleasedCallbackRunning(rtId) ~>
                 ~IsResourcesReleasedCallbackRunning(rtId)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []TypeOK
    BY <2>0, BehaviorEstablishesIndInv, IndInvParts, PTL
  <2>2. WF_vars(ResourcesReleasedCallbackReturns(rtId))
    BY <2>0, FairnessAtRuntime, IsaT(600), PTL
  <2>3. QED
    BY <1>0, <2>0, <2>1, <2>2, PTL
<1>2. QED BY <1>1, Zenon DEF ResourcesReleasedCallbacksReturn

THEOREM ShutdownCallbacksReturnHolds == Spec => ShutdownCallbacksReturn
<1>0. ASSUME NEW rtId \in RuntimeIds
      PROVE  /\ [](TypeOK /\ IsShutdownCallbackRunning(rtId) =>
                   ENABLED <<ShutdownCallbackReturns(rtId)>>_vars)
             /\ [](TypeOK /\ <<ShutdownCallbackReturns(rtId)>>_vars =>
                   (~IsShutdownCallbackRunning(rtId))')
             /\ [](TypeOK /\ IsShutdownCallbackRunning(rtId) /\ [Next]_vars /\
                   ~<<ShutdownCallbackReturns(rtId)>>_vars => (IsShutdownCallbackRunning(rtId))')
  <2>1. TypeOK /\ IsShutdownCallbackRunning(rtId) => ENABLED <<ShutdownCallbackReturns(rtId)>>_vars
    BY SCREnabled, Zenon
  <2>2. TypeOK /\ <<ShutdownCallbackReturns(rtId)>>_vars => (~IsShutdownCallbackRunning(rtId))'
    BY SCRClears, Zenon
  <2>3. TypeOK /\ IsShutdownCallbackRunning(rtId) /\ [Next]_vars /\
            ~<<ShutdownCallbackReturns(rtId)>>_vars => (IsShutdownCallbackRunning(rtId))'
    BY RunningStable, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>1. ASSUME Spec, NEW rtId \in RuntimeIds
      PROVE  IsShutdownCallbackRunning(rtId) ~> ~IsShutdownCallbackRunning(rtId)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []TypeOK
    BY <2>0, BehaviorEstablishesIndInv, IndInvParts, PTL
  <2>2. WF_vars(ShutdownCallbackReturns(rtId))
    BY <2>0, FairnessAtRuntime, IsaT(600)
  <2>3. QED
    BY <1>0, <2>0, <2>1, <2>2, PTL
<1>2. QED BY <1>1, Zenon DEF ShutdownCallbacksReturn

\* Enabledness and effect of the two buffer steps, and the frames that
\* keep a buffer where it is until its own step fires.  A buffer's state
\* is written only by a step naming that buffer, or by a send committing
\* it - and a send moves it to the same place a return would.
LEMMA ReturnBufferEnabled ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  TypeOK /\ LentCountMatchesBufferStates /\
               IsLentBuffer(cId, b) =>
                   ENABLED <<HostReturnsBuffer(cId, b)>>_vars
<1>1. SUFFICES ASSUME TypeOK, LentCountMatchesBufferStates,
                      IsLentBuffer(cId, b)
               PROVE  ENABLED <<HostReturnsBuffer(cId, b)>>_vars
    OBVIOUS
\* The counted guard follows from the named one through the bridge.
<1>2. HostHoldsSomeBuffer(cId)
    BY <1>1, NoLentBufferWhenCountIsZero, SMT
    DEF HostHoldsSomeBuffer, HostHoldsNoBuffer, TypeOK, L0!TypeOK
<1>3. QED
    BY <1>1, <1>2, ExpandENABLED, SMT
    DEF HostReturnsBuffer, IsLentBuffer, HostHoldsSomeBuffer,
        TypeOK, L0!TypeOK, l0_vars, L0!vars, vars, ffi_vars

\* Being returned is not enough to enable the release: the guard also asks
\* that the send this buffer carries be acquitted, which is what stops the
\* bytes going while the transport may still be reading them.
\* The lend is enabled wherever the host may ask and the budget has room for
\* the message's own size.  Taken at that size, the two budget guards are the
\* same fact twice: CoversMessage is reflexive there, and IsSendableMessage is
\* the room hypothesis read against MessageWithinCeiling's bound.
LEMMA LendEnabled ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW msg \in Messages,
           IsSendableMessage(msg)
    PROVE  TypeOK /\ ContemplatesLend(cId) /\ HasFreeSendSlot(cId)
               /\ IsFreshBuffer(cId, b)
               /\ IsMemoryAvailable(MessageLength[msg]) =>
                   ENABLED <<LendForMessage(cId, b, msg)>>_vars
<1>0. SUFFICES ASSUME TypeOK, ContemplatesLend(cId), HasFreeSendSlot(cId),
                      IsFreshBuffer(cId, b),
                      IsMemoryAvailable(MessageLength[msg])
               PROVE  ENABLED <<LendForMessage(cId, b, msg)>>_vars
    OBVIOUS
\* The step changes the state, said without a prime so it survives into the
\* expanded ENABLED as a hypothesis: the buffer leaves "none".
<1>05. [buffer_state EXCEPT ![cId][b] = "lent"] # buffer_state
  <2>1. buffer_state[cId][b] = "none"
    BY <1>0, Zenon DEF IsFreshBuffer
  <2>2. [buffer_state EXCEPT ![cId][b] = "lent"][cId][b] = "lent"
    BY <1>0, SMT DEF TypeOK, L0!TypeOK
  <2>3. QED
    BY <2>1, <2>2, Zenon
<1>1. QED
    BY <1>0, <1>05, MessageLengthIsNat, ExpandENABLED, SMTT(120)
    DEF LendForMessage, LendSendBuffer, ContemplatesLend, HasFreeSendSlot,
        IsFreshBuffer, IsMemoryAvailable, CoversMessage, CoversRequest,
        IsSendableMessage, IsLendable, Sizes, MessageLengthIsNat,
        l0_vars, L0!vars, vars, ffi_vars

LEMMA FreeBufferEnabled ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  TypeOK /\ IsReturnedBuffer(cId, b) /\
               CarriesNoUnacquittedSend(cId, b) =>
                   ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
\* The hypotheses come off first: the prover then faces the existential
\* alone, and the witness it needs for the budget is the first disjunct.
<1>0. SUFFICES ASSUME TypeOK, IsReturnedBuffer(cId, b),
                      CarriesNoUnacquittedSend(cId, b)
               PROVE  ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
    OBVIOUS
\* The step changes the state, said without a prime so it survives into the
\* expanded ENABLED as a hypothesis.  Without it the solver has to rebuild the
\* nested EXCEPT read-back inside an existential over every primed variable,
\* and it does not: with the budget unconstrained it took the free route of
\* varying the budget instead, which is why this only became necessary once
\* the free started writing it.
<1>05. [buffer_state EXCEPT ![cId][b] = "freed"] # buffer_state
  <2>1. buffer_state[cId][b] = "returned"
    BY <1>0, Zenon DEF IsReturnedBuffer
  <2>2. [buffer_state EXCEPT ![cId][b] = "freed"][cId][b] = "freed"
    BY <1>0, SMT DEF TypeOK, L0!TypeOK
  <2>3. QED
    BY <2>1, <2>2, Zenon
\* AnotherBufferOutstanding stays opaque here: the witness lives in the other
\* branch of the budget clause, and unfolding this one would only hand the
\* solver an existential it has no reason to satisfy.
<1>1. QED
    BY <1>0, <1>05, ExpandENABLED, SMTT(120)
    DEF FreeReturnedBuffer, IsReturnedBuffer, CarriesNoUnacquittedSend,
        l0_vars, L0!vars, vars, ffi_vars

\* The entry of one buffer of one call moves only under a step that names
\* it: lending it, giving it back, releasing it, or a send committing it.
\* The send is the awkward one, because it picks its buffer existentially -
\* so what has to be excluded is not every send on the call but a send that
\* could have committed this buffer, which is to say the buffer being lent.
LEMMA BufferEntryFrozen ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds,
           NEW b \in BufferIds,
           \A msg \in Messages, ch \in Sizes : ~LendSendBuffer(cId, b, msg, ch),
           ~HostReturnsBuffer(cId, b),
           ~FreeReturnedBuffer(cId, b),
           \/ (\A m \in Messages, bs \in BufferIds :
                  ~SendMessage(cId, m, bs))
           \/ ~IsLentBuffer(cId, b)
    PROVE  (buffer_state[cId][b])' = buffer_state[cId][b]
<1>1. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>1, SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, TypeOK, L0!TypeOK
<1>2. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <1>2, SMT DEF HostReturnsBuffer, TypeOK, L0!TypeOK
<1>3. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <1>3, SMT DEF FreeReturnedBuffer, TypeOK, L0!TypeOK
<1>4. CASE \E d \in CallIds, m \in Messages,
              bs \in BufferIds : SendMessage(d, m, bs)
  <2>1. PICK d \in CallIds, m \in Messages, bs \in BufferIds :
              SendMessage(d, m, bs)
    BY <1>4
  <2>2. PICK e \in BufferIds :
            /\ IsLentBuffer(d, e)
            /\ buffer_state' = [buffer_state EXCEPT ![d][e] = "returned"]
    BY <2>1, Zenon DEF SendMessage
\* Either no send on this call at all, or this buffer is not lent - and
\* the committed one is, so it is not this one.
  <2>3. d # cId \/ e # b
    BY <2>1, <2>2, Zenon DEF IsLentBuffer
  <2>4. QED
    BY <2>2, <2>3, SMT DEF TypeOK, L0!TypeOK
<1>5. CASE UNCHANGED buffer_state
    BY <1>5, Zenon
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5, OnlyBufferStepsWriteBufferStates, Zenon

LEMMA BufferStateFrame ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  TypeOK /\ [Next]_vars =>
               /\ (IsLentBuffer(cId, b) /\
                      ~<<HostReturnsBuffer(cId, b)>>_vars /\
                      (\A m \in Messages, bs \in BufferIds :
                          ~<<SendMessage(cId, m, bs)>>_vars) =>
                          (IsLentBuffer(cId, b))')
               /\ (IsReturnedBuffer(cId, b) /\
                      ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                          (IsReturnedBuffer(cId, b))')
               /\ (IsFreedBuffer(cId, b) => (IsFreedBuffer(cId, b))')
<1>1. SUFFICES ASSUME TypeOK, [Next]_vars
               PROVE  /\ (IsLentBuffer(cId, b) /\
                             ~<<HostReturnsBuffer(cId, b)>>_vars /\
                             (\A m \in Messages, bs \in BufferIds :
                                  ~<<SendMessage(cId, m, bs)>>_vars) =>
                                 (IsLentBuffer(cId, b))')
                      /\ (IsReturnedBuffer(cId, b) /\
                             ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                                 (IsReturnedBuffer(cId, b))')
                      /\ (IsFreedBuffer(cId, b) =>
                                 (IsFreedBuffer(cId, b))')
    OBVIOUS
\* The three states are exclusive, so each antecedent rules out the steps
\* that would need a different one.
<1>2. ASSUME IsLentBuffer(cId, b),
             ~<<HostReturnsBuffer(cId, b)>>_vars,
             \A m \in Messages, bs \in BufferIds :
                ~<<SendMessage(cId, m, bs)>>_vars
      PROVE  (IsLentBuffer(cId, b))'
  <2>1. ~HostReturnsBuffer(cId, b)
    BY <1>1, <1>2, BufferSubscriptCollapses
  <2>2. \A m \in Messages, bs \in BufferIds :
           ~SendMessage(cId, m, bs)
    <3>1. ASSUME NEW m \in Messages, NEW bs \in BufferIds
          PROVE  <<SendMessage(cId, m, bs)>>_vars <=> SendMessage(cId, m, bs)
      BY <1>1, SendSubscriptCollapses
    <3>2. QED BY <1>2, <3>1
  <2>3. /\ \A msg \in Messages, ch \in Sizes : ~LendSendBuffer(cId, b, msg, ch)
        /\ ~FreeReturnedBuffer(cId, b)
    BY <1>2, Zenon
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, FreeReturnedBuffer, IsFreshBuffer,
        IsReturnedBuffer, IsLentBuffer
  <2>4. QED
    BY <1>1, <1>2, <2>1, <2>2, <2>3, BufferEntryFrozen, Zenon
    DEF IsLentBuffer
<1>3. ASSUME IsReturnedBuffer(cId, b),
             ~<<FreeReturnedBuffer(cId, b)>>_vars
      PROVE  (IsReturnedBuffer(cId, b))'
  <2>1. ~FreeReturnedBuffer(cId, b)
    BY <1>1, <1>3, BufferSubscriptCollapses
  <2>2. /\ \A msg \in Messages, ch \in Sizes : ~LendSendBuffer(cId, b, msg, ch)
        /\ ~HostReturnsBuffer(cId, b)
        /\ ~IsLentBuffer(cId, b)
    BY <1>3, Zenon
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, IsFreshBuffer,
        IsReturnedBuffer, IsLentBuffer
  <2>3. QED
    BY <1>1, <1>3, <2>1, <2>2, BufferEntryFrozen, Zenon
    DEF IsReturnedBuffer
<1>4. ASSUME IsFreedBuffer(cId, b)
      PROVE  (IsFreedBuffer(cId, b))'
  <2>1. /\ \A msg \in Messages, ch \in Sizes : ~LendSendBuffer(cId, b, msg, ch)
        /\ ~HostReturnsBuffer(cId, b)
        /\ ~FreeReturnedBuffer(cId, b)
        /\ ~IsLentBuffer(cId, b)
    BY <1>4, Zenon
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, HostReturnsBuffer, FreeReturnedBuffer,
        IsFreshBuffer, IsReturnedBuffer, IsLentBuffer, IsFreedBuffer
  <2>2. QED
    BY <1>1, <1>4, <2>1, BufferEntryFrozen, Zenon DEF IsFreedBuffer
<1>5. QED
    BY <1>1, <1>2, <1>3, <1>4, Zenon

\* The first rung in the shape a fairness argument needs: a lent buffer
\* either stays lent or is already returned.  The send is why the second
\* disjunct is there - committing the buffer returns it just as
\* ak_return_call_buffer would, so a send cannot break the rung, only
\* finish it early.
LEMMA LentStaysOrIsReturned ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  TypeOK /\ IsLentBuffer(cId, b) /\ [Next]_vars /\
               ~<<HostReturnsBuffer(cId, b)>>_vars =>
                   (IsLentBuffer(cId, b))' \/ (IsReturnedBuffer(cId, b))'
<1>1. SUFFICES ASSUME TypeOK, IsLentBuffer(cId, b), [Next]_vars,
                      ~<<HostReturnsBuffer(cId, b)>>_vars
               PROVE  (IsLentBuffer(cId, b))' \/ (IsReturnedBuffer(cId, b))'
    OBVIOUS
<1>2. ~HostReturnsBuffer(cId, b)
    BY <1>1, BufferSubscriptCollapses
<1>3. /\ \A msg \in Messages, ch \in Sizes : ~LendSendBuffer(cId, b, msg, ch)
      /\ ~FreeReturnedBuffer(cId, b)
    BY <1>1, Zenon
    DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, FreeReturnedBuffer, IsFreshBuffer,
        IsReturnedBuffer, IsLentBuffer
<1>4. CASE \E d \in CallIds, m \in Messages,
              bs \in BufferIds : SendMessage(d, m, bs)
  <2>1. PICK d \in CallIds, m \in Messages, bs \in BufferIds :
              SendMessage(d, m, bs)
    BY <1>4
  <2>2. PICK e \in BufferIds :
            /\ IsLentBuffer(d, e)
            /\ buffer_state' = [buffer_state EXCEPT ![d][e] = "returned"]
    BY <2>1, Zenon DEF SendMessage
\* Either this send committed this buffer, and it is returned, or it
\* committed another and this one has not moved.
  <2>3. CASE d = cId /\ e = b
    BY <1>1, <2>2, <2>3, SMT DEF IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>4. CASE ~(d = cId /\ e = b)
    BY <1>1, <2>2, <2>4, SMT DEF IsLentBuffer, TypeOK, L0!TypeOK
  <2>5. QED BY <2>3, <2>4
<1>5. CASE \A d \in CallIds, m \in Messages, bs \in BufferIds : ~SendMessage(d, m, bs)
  <2>1. (buffer_state[cId][b])' = buffer_state[cId][b]
    BY <1>1, <1>2, <1>3, <1>5, BufferEntryFrozen, Zenon
  <2>2. QED BY <1>1, <2>1, Zenon DEF IsLentBuffer
<1>6. QED BY <1>4, <1>5

\* Off the lent state the index a buffer carries cannot move: only a send
\* writes it, and a send writes the buffer it commits, which is lent.
LEMMA BufferSendFrozenOffLent ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds, NEW b \in BufferIds,
           ~IsLentBuffer(cId, b)
    PROVE  (buffer_send[cId][b])' = buffer_send[cId][b]
<1>0. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
               SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>1
  <2>2. IsLentBuffer(d, e)
    BY <2>1, Zenon DEF SendMessage
  <2>3. ~(cId = d /\ b = e)
    BY <2>2, Zenon
  <2>4. QED
    BY <1>0, <2>1, <2>3, SendMovesOneEntry, SMT
<1>2. CASE UNCHANGED buffer_send
    BY <1>2, Zenon
<1>3. QED
    BY <1>1, <1>2, OnlySendMessageWritesBufferSend, Zenon

\* The count of acquittals never falls: only one step writes it, and that
\* step increments.
LEMMA WriteDonesOnlyGrow ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds
    PROVE  /\ (write_dones_emitted[cId])' \in Nat
           /\ (write_dones_emitted[cId])' >= write_dones_emitted[cId]
<1>0. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
\* EmitMovesOneCount is stated for one entry of one call, so it wants a
\* buffer to name; any will do, and the identity space is nonempty.
<1>01. PICK y \in BufferIds : y = y
    BY BufferIdsAreAFiniteNonemptySet DEF BufferIdsAreAFiniteNonemptySet
<1>1. CASE \E c \in CallIds : EmitWriteDone(c)
  <2>1. PICK c \in CallIds : EmitWriteDone(c)
    BY <1>1
  <2>2. QED
    BY <1>0, <1>01, <2>1, EmitMovesOneCount
<1>2. CASE UNCHANGED write_dones_emitted
    BY <1>0, <1>2, SMT
<1>3. QED
    BY <1>1, <1>2, EveryStepEitherEmitsOrKeepsWriteDones, Zenon

\* So the bytes of a buffer, once free to go, stay free to go: the index is
\* frozen off the lent state and the count it is compared against only grows.
LEMMA AcquittalPersistsOffLent ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds, NEW b \in BufferIds,
           ~IsLentBuffer(cId, b), CarriesNoUnacquittedSend(cId, b)
    PROVE  (CarriesNoUnacquittedSend(cId, b))'
<1>0. /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. (buffer_send[cId][b])' = buffer_send[cId][b]
    BY BufferSendFrozenOffLent
<1>2. /\ (write_dones_emitted[cId])' \in Nat
      /\ (write_dones_emitted[cId])' >= write_dones_emitted[cId]
    BY WriteDonesOnlyGrow
<1>3. QED
    BY <1>0, <1>1, <1>2, SMT DEF CarriesNoUnacquittedSend

\* The debt a returned buffer carries is bounded by the send window, and the
\* window is bounded by a constant.  This is what lets the induction below be
\* instantiated at all: the index a buffer carries grows without bound over
\* the life of a call, but its distance from the acquittals does not.
LEMMA ReturnedBufferDebtBounded ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  TypeOK /\ FfiCallInv /\ BufferStateInv =>
               buffer_send[cId][b] - WriteDonesReturned(cId)
                   <= MaxSendsInFlight + 1
<1>1. SUFFICES ASSUME TypeOK, FfiCallInv, BufferStateInv
               PROVE  buffer_send[cId][b] - WriteDonesReturned(cId)
                          <= MaxSendsInFlight + 1
    OBVIOUS
<1>2. /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ write_dones_emitted \in [CallIds -> Nat]
      /\ buffers_held_by_host \in [CallIds -> Nat]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ Len(submitted[cId]) \in Nat
    BY <1>1, TypeOKSplit, LenProperties, Zenon
    DEF TypeOK, L0!TypeOK, BufferTypes
<1>3. buffer_send[cId][b] <= Len(submitted[cId])
    BY <1>1, Zenon DEF BufferStateInv, BufferSendIndicesExist
<1>4. SendWindowOccupancy(cId) <= MaxSendsInFlight
    BY <1>1, Zenon DEF FfiCallInv, SendsInFlightWithinLimit
<1>5. QED
    BY <1>1, <1>2, <1>3, <1>4, MaxSendsInFlightIsPositive, SMT
    DEF SendWindowOccupancy, WriteDonesReturned,
        MaxSendsInFlightIsPositive

\* The acquittal of the send a buffer carries, read at the index the buffer
\* records.  A state formula, so it may be instantiated at that index even
\* though the index is not a constant.
BufferSendAcquitted(cId, b) == IsSendAcquittedAt(cId, buffer_send[cId][b])

\* A returned buffer stays returned until it is freed: the only step that
\* moves the entry off "returned" is the release, and lending asks for
\* "none".
LEMMA ReturnedBufferStaysOrIsFreed ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds, NEW b \in BufferIds,
           IsReturnedBuffer(cId, b)
    PROVE  (IsReturnedBuffer(cId, b))' \/ (IsFreedBuffer(cId, b))'
<1>0. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. ~IsLentBuffer(cId, b) /\ ~IsFreshBuffer(cId, b)
    BY <1>0, SMT DEF IsReturnedBuffer, IsLentBuffer, IsFreshBuffer
<1>2. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
  <2>1. PICK d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>2
  <2>2. ~(cId = d /\ b = e)
    BY <1>1, <2>1, Zenon DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer
  <2>3. QED
    BY <1>0, <2>1, <2>2, LendMovesOneEntry, SMT
    DEF IsReturnedBuffer, IsFreedBuffer
<1>3. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <1>0, <1>3, ReturnMovesOneEntry, SMT
    DEF IsReturnedBuffer, IsFreedBuffer
<1>4. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <1>0, <1>4, FreeMovesOneEntry, SMT
    DEF IsReturnedBuffer, IsFreedBuffer
<1>5. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
               SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>5
  <2>2. ~(cId = d /\ b = e)
    BY <1>1, <2>1, Zenon DEF SendMessage
  <2>3. QED
    BY <1>0, <2>1, <2>2, SendMovesOneEntry, SMT
    DEF IsReturnedBuffer, IsFreedBuffer
<1>6. CASE UNCHANGED buffer_state
    BY <1>6, Zenon DEF IsReturnedBuffer, IsFreedBuffer
<1>7. QED
    BY <1>2, <1>3, <1>4, <1>5, <1>6,
       OnlyBufferStepsWriteBufferStates, Zenon

\* The rung bridges at the index a returned buffer carries.  Each is the
\* k-indexed fact instantiated there, which is legal because each is a state
\* formula: the index is read in the state the formula is evaluated at.
LEMMA ReturnedBufferRungBridges ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ (TypeOK /\ BufferStateInv /\
                   ~BufferSendAcquitted(cId, b) /\
                   ~IsWriteDoneCallbackRunning(cId) =>
                       IsAwaitingWriteDone(cId))
           /\ (TypeOK /\ BufferStateInv =>
                   HasAcceptedSendAt(cId, buffer_send[cId][b]))
\* TypeOK is a hypothesis of each implication rather than of the lemma, so
\* the typing has to be stated the same way.
<1>0. TypeOK =>
          /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
          /\ submitted \in [CallIds -> Seq(Messages)]
          /\ Len(submitted[cId]) \in Nat
    BY TypeOKSplit, LenProperties, Zenon
    DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. TypeOK /\ BufferStateInv =>
          buffer_send[cId][b] <= Len(submitted[cId])
    BY Zenon DEF BufferStateInv, BufferSendIndicesExist
<1>2. QED
    BY <1>0, <1>1, SMT
    DEF BufferSendAcquitted, IsSendAcquittedAt, HasAcceptedSendAt,
        WriteDonesReturned, IsAwaitingWriteDone, IsWriteDoneCallbackRunning,
        TypeOK, L0!TypeOK

\* One rung of the buffer drain: the same pair of actions as the send ladder
\* and the same measure, read at the index the buffer carries.  That index is
\* frozen while the buffer is given back, so the pair still moves the measure
\* down by one.  Being freed is a way out rather than a failure: the release
\* guard is the very thing this ladder is establishing.
THEOREM ReturnedBufferDescent ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => ((/\ IsReturnedBuffer(cId, b)
                /\ ~BufferSendAcquitted(cId, b)
                /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1)
               ~> (\/ BufferSendAcquitted(cId, b)
                   \/ IsFreedBuffer(cId, b)
                   \/ /\ IsReturnedBuffer(cId, b)
                      /\ ~BufferSendAcquitted(cId, b)
                      /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n))
<1>100. TypeOK /\ BufferStateInv /\ ~BufferSendAcquitted(cId, b) /\
            ~IsWriteDoneCallbackRunning(cId) => IsAwaitingWriteDone(cId)
    BY ReturnedBufferRungBridges, Zenon
<1>10. [](TypeOK /\ BufferStateInv /\ ~BufferSendAcquitted(cId, b) /\
              ~IsWriteDoneCallbackRunning(cId) => IsAwaitingWriteDone(cId))
    BY <1>100, PTL
<1>110. TypeOK /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) =>
                ENABLED <<EmitWriteDone(cId)>>_vars
    BY EmitWriteDoneEnabled, Zenon
<1>11. [](TypeOK /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) =>
                  ENABLED <<EmitWriteDone(cId)>>_vars)
    BY <1>110, PTL
<1>120. TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
            ENABLED <<WriteDoneReturns(cId)>>_vars
    BY WriteDoneReturnsEnabled, Zenon
<1>12. [](TypeOK /\ IsWriteDoneCallbackRunning(cId) =>
              ENABLED <<WriteDoneReturns(cId)>>_vars)
    BY <1>120, PTL
<1>130. TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
            (IsWriteDoneCallbackRunning(cId))'
    BY DrainStepEffects, Zenon
<1>13. [](TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
              (IsWriteDoneCallbackRunning(cId))')
    BY <1>130, PTL
\* The acquittal callback returning is the step that moves the measure: it
\* leaves the buffer alone and raises the returned count by one.
<1>140. TypeOK /\ IsReturnedBuffer(cId, b) /\
            buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1 /\
            <<WriteDoneReturns(cId)>>_vars =>
                (\/ BufferSendAcquitted(cId, b)
                 \/ IsFreedBuffer(cId, b)
                 \/ /\ IsReturnedBuffer(cId, b)
                    /\ ~BufferSendAcquitted(cId, b)
                    /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n)'
    BY DrainStepEffects, SMT
    DEF WriteDoneReturns, BufferSendAcquitted, IsSendAcquittedAt,
        IsReturnedBuffer, IsFreedBuffer, TypeOK, L0!TypeOK,
        vars, l0_vars, L0!vars, ffi_vars
<1>14. [](TypeOK /\ IsReturnedBuffer(cId, b) /\
              buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1 /\
              <<WriteDoneReturns(cId)>>_vars =>
                  (\/ BufferSendAcquitted(cId, b)
                   \/ IsFreedBuffer(cId, b)
                   \/ /\ IsReturnedBuffer(cId, b)
                      /\ ~BufferSendAcquitted(cId, b)
                      /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n)')
    BY <1>140, PTL
<1>150. TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
            ~IsWriteDoneCallbackRunning(cId) /\
            ~<<EmitWriteDone(cId)>>_vars =>
                (IsAwaitingWriteDone(cId) /\
                     ~IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>15. [](TypeOK /\ [Next]_vars /\ IsAwaitingWriteDone(cId) /\
              ~IsWriteDoneCallbackRunning(cId) /\
              ~<<EmitWriteDone(cId)>>_vars =>
                  (IsAwaitingWriteDone(cId) /\
                       ~IsWriteDoneCallbackRunning(cId))')
    BY <1>150, PTL
<1>160. TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
            ~<<WriteDoneReturns(cId)>>_vars =>
                (IsWriteDoneCallbackRunning(cId))'
    BY SlotFrame, Zenon
<1>16. [](TypeOK /\ [Next]_vars /\ IsWriteDoneCallbackRunning(cId) /\
              ~<<WriteDoneReturns(cId)>>_vars =>
                  (IsWriteDoneCallbackRunning(cId))')
    BY <1>160, PTL
\* Any other step freezes the measure, because it freezes both of its terms.
<1>170. TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) /\
            ~<<WriteDoneReturns(cId)>>_vars /\
            buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1 =>
                (buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1)'
  <2>1. SUFFICES ASSUME TypeOK, [Next]_vars, IsReturnedBuffer(cId, b),
                        ~<<WriteDoneReturns(cId)>>_vars
                 PROVE  /\ (buffer_send[cId][b])' = buffer_send[cId][b]
                        /\ (WriteDonesReturned(cId))' = WriteDonesReturned(cId)
    OBVIOUS
  <2>2. ~IsLentBuffer(cId, b)
    BY <2>1, SMT DEF IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>3. (buffer_send[cId][b])' = buffer_send[cId][b]
    BY <2>1, <2>2, BufferSendFrozenOffLent
  <2>4. QED
    BY <2>1, <2>3, ReturnedFrameUnlessWDR
<1>17. [](TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) /\
              ~<<WriteDoneReturns(cId)>>_vars /\
              buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1 =>
                  (buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1)')
    BY <1>170, PTL
<1>180. TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) =>
            (IsReturnedBuffer(cId, b))' \/ (IsFreedBuffer(cId, b))'
    BY ReturnedBufferStaysOrIsFreed, Zenon
<1>18. [](TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) =>
              (IsReturnedBuffer(cId, b))' \/ (IsFreedBuffer(cId, b))')
    BY <1>180, PTL
<1>190. TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) /\
            ~BufferSendAcquitted(cId, b) /\
            ~<<WriteDoneReturns(cId)>>_vars =>
                (~BufferSendAcquitted(cId, b))'
  <2>1. SUFFICES ASSUME TypeOK, [Next]_vars, IsReturnedBuffer(cId, b),
                        ~BufferSendAcquitted(cId, b),
                        ~<<WriteDoneReturns(cId)>>_vars
                 PROVE  (~BufferSendAcquitted(cId, b))'
    OBVIOUS
  <2>2. ~IsLentBuffer(cId, b)
    BY <2>1, SMT DEF IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>3. (buffer_send[cId][b])' = buffer_send[cId][b]
    BY <2>1, <2>2, BufferSendFrozenOffLent
  <2>4. (WriteDonesReturned(cId))' = WriteDonesReturned(cId)
    BY <2>1, ReturnedFrameUnlessWDR
  <2>5. QED
    BY <2>1, <2>3, <2>4, SMT
    DEF BufferSendAcquitted, IsSendAcquittedAt, TypeOK, L0!TypeOK
<1>19. [](TypeOK /\ [Next]_vars /\ IsReturnedBuffer(cId, b) /\
              ~BufferSendAcquitted(cId, b) /\
              ~<<WriteDoneReturns(cId)>>_vars =>
                  (~BufferSendAcquitted(cId, b))')
    BY <1>190, PTL
<1>200. TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
            ~<<WriteDoneReturns(cId)>>_vars
    BY SMT DEF EmitWriteDone, WriteDoneReturns, TypeOK, L0!TypeOK,
        vars, l0_vars, L0!vars, ffi_vars
<1>20. [](TypeOK /\ <<EmitWriteDone(cId)>>_vars =>
              ~<<WriteDoneReturns(cId)>>_vars)
    BY <1>200, PTL
<1>1. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  (/\ IsReturnedBuffer(cId, b)
              /\ ~BufferSendAcquitted(cId, b)
              /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1)
             ~> (\/ BufferSendAcquitted(cId, b)
                 \/ IsFreedBuffer(cId, b)
                 \/ /\ IsReturnedBuffer(cId, b)
                    /\ ~BufferSendAcquitted(cId, b)
                    /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n)
  <2>1. []TypeOK
    BY <1>1, PTL
  <2>2. []BufferStateInv
    BY <1>1, PTL
  <2>3. QED
    BY <1>1, <2>1, <2>2, <1>10, <1>11, <1>12, <1>13, <1>14, <1>15,
       <1>16, <1>17, <1>18, <1>19, <1>20, PTL
<1>2. QED BY <1>1, PTL

\* The bound splits of the buffer ladder, standalone: the induction hypothesis
\* in the consumer's scope bans necessitation there.
LEMMA ReturnedBufferBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                     ~BufferSendAcquitted(cId, b) /\
                     buffer_send[cId][b] - WriteDonesReturned(cId) <= n + 1
                         => \/ buffer_send[cId][b] - WriteDonesReturned(cId)
                                   <= n
                            \/ buffer_send[cId][b] - WriteDonesReturned(cId)
                                   = n + 1)
           /\ [](TypeOK /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n
                     => buffer_send[cId][b] - WriteDonesReturned(cId) <= n)
           /\ [](TypeOK /\ BufferSendAcquitted(cId, b) =>
                     CarriesNoUnacquittedSend(cId, b))
<1>1. TypeOK /\ IsReturnedBuffer(cId, b) /\
          ~BufferSendAcquitted(cId, b) /\
          buffer_send[cId][b] - WriteDonesReturned(cId) <= n + 1
              => \/ buffer_send[cId][b] - WriteDonesReturned(cId) <= n
                 \/ buffer_send[cId][b] - WriteDonesReturned(cId) = n + 1
    BY SMT DEF TypeOK, L0!TypeOK, WriteDonesReturned
<1>2. TypeOK /\ buffer_send[cId][b] - WriteDonesReturned(cId) = n
          => buffer_send[cId][b] - WriteDonesReturned(cId) <= n
    BY SMT DEF TypeOK, L0!TypeOK, WriteDonesReturned
\* An acquitted send is a send whose bytes may go: the returned count never
\* exceeds the emitted one.
<1>3. TypeOK /\ BufferSendAcquitted(cId, b) =>
          CarriesNoUnacquittedSend(cId, b)
    BY SMT
    DEF BufferSendAcquitted, IsSendAcquittedAt, WriteDonesReturned,
        CarriesNoUnacquittedSend, TypeOK, L0!TypeOK
<1>4. QED BY <1>1, <1>2, <1>3, PTL

\* The ladder: the debt a returned buffer carries falls to nothing, so its
\* bytes become free to release.  It is instantiated at MaxSendsInFlight + 1,
\* which ReturnedBufferDebtBounded puts above the measure - and a constant
\* bound is what makes any instantiation possible, the index a buffer carries
\* not being one.
THEOREM ReturnedBufferAcquits ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ [][Next]_vars
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => (IsReturnedBuffer(cId, b) ~>
                   (CarriesNoUnacquittedSend(cId, b) \/
                        IsFreedBuffer(cId, b)))
<1>0. MaxSendsInFlight + 1 \in Nat
    BY MaxSendsInFlightIsPositive, SMT DEF MaxSendsInFlightIsPositive
<1> DEFINE P == IsReturnedBuffer(cId, b) /\ ~BufferSendAcquitted(cId, b)
           M == buffer_send[cId][b] - WriteDonesReturned(cId)
           G == CarriesNoUnacquittedSend(cId, b) \/ IsFreedBuffer(cId, b)
<1>1. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             [][Next]_vars,
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  IsReturnedBuffer(cId, b) ~> G
  <2> DEFINE Ind(n) == (P /\ M <= n) ~> G
\* Rung zero is empty: a measure at or below zero is an acquitted send, and
\* the ladder only ever stands on an unacquitted one.
  <2>2. Ind(0)
    <3>1. TypeOK => ~(P /\ M <= 0)
      BY SMT DEF BufferSendAcquitted, IsSendAcquittedAt, WriteDonesReturned,
          TypeOK, L0!TypeOK
    <3>2. QED BY <1>1, <3>1, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE Ind(n + 1)
    <3>1. (P /\ M = n + 1) ~>
              (\/ BufferSendAcquitted(cId, b)
               \/ IsFreedBuffer(cId, b)
               \/ (P /\ M = n))
      BY <1>1, <2>3, ReturnedBufferDescent, PTL
    <3>15. [](TypeOK /\ BufferSendAcquitted(cId, b) =>
                  CarriesNoUnacquittedSend(cId, b))
      BY <2>3, ReturnedBufferBoundSplit
    <3>2. [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                 ~BufferSendAcquitted(cId, b) /\
                 buffer_send[cId][b] - WriteDonesReturned(cId) <= n + 1
                     => \/ buffer_send[cId][b] - WriteDonesReturned(cId) <= n
                        \/ buffer_send[cId][b] - WriteDonesReturned(cId)
                               = n + 1)
      BY <2>3, ReturnedBufferBoundSplit
    <3>3. [](TypeOK /\ M = n => M <= n)
      BY <2>3, ReturnedBufferBoundSplit
    <3>4. []TypeOK
      BY <1>1, PTL
    <3>5. QED BY <2>3, <3>1, <3>15, <3>2, <3>3, <3>4, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(600)
  <2>5. Ind(MaxSendsInFlight + 1)
    BY <1>0, <2>4
\* Entering the ladder: the debt is under the constant bound, and a buffer
\* whose send is already acquitted is past the goal.
  <2>6. [](TypeOK /\ FfiCallInv /\ BufferStateInv =>
               (IsReturnedBuffer(cId, b) =>
                    (P /\ M <= MaxSendsInFlight + 1) \/ G))
    <3>1. TypeOK /\ FfiCallInv /\ BufferStateInv =>
              (IsReturnedBuffer(cId, b) =>
                   (P /\ M <= MaxSendsInFlight + 1) \/ G)
      BY ReturnedBufferDebtBounded, SMT
      DEF BufferSendAcquitted, IsSendAcquittedAt, WriteDonesReturned,
          CarriesNoUnacquittedSend, TypeOK, L0!TypeOK
    <3>2. QED BY <3>1, PTL
  <2>7. [](TypeOK /\ FfiCallInv /\ BufferStateInv)
    BY <1>1, PTL
  <2>8. QED BY <1>1, <2>5, <2>6, <2>7, PTL DEF Ind
<1>2. QED BY <1>1, PTL

\* A cited lemma may be necessitated where a step proved under a temporal
\* hypothesis may not, which is the only reason this is a lemma.
LEMMA IndInvBufferParts ==
    IndInv => TypeOK /\ FfiCallInv /\ BufferStateInv /\
                  LentCountMatchesBufferStates
<1>1. QED
    BY DEF IndInv, BufferStateInv

\* Freed is where a buffer identity ends: lending asks for "none", giving
\* back and committing ask for "lent", and releasing asks for "returned".
LEMMA FreedStaysFreed ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds, NEW b \in BufferIds,
           IsFreedBuffer(cId, b)
    PROVE  (IsFreedBuffer(cId, b))'
<1>0. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. /\ ~IsLentBuffer(cId, b)
      /\ ~IsFreshBuffer(cId, b)
      /\ ~IsReturnedBuffer(cId, b)
    BY <1>0, SMT
    DEF IsFreedBuffer, IsLentBuffer, IsFreshBuffer, IsReturnedBuffer
<1>2. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
  <2>1. PICK d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>2
  <2>2. ~(cId = d /\ b = e)
    BY <1>1, <2>1, Zenon DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsFreshBuffer
  <2>3. QED
    BY <1>0, <2>1, <2>2, LendMovesOneEntry, SMT DEF IsFreedBuffer
<1>3. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
  <2>1. PICK d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <1>3
  <2>2. ~(cId = d /\ b = e)
    BY <1>1, <2>1, Zenon DEF HostReturnsBuffer
  <2>3. QED
    BY <1>0, <2>1, <2>2, ReturnMovesOneEntry, SMT DEF IsFreedBuffer
<1>4. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <1>0, <1>4, FreeMovesOneEntry, SMT DEF IsFreedBuffer
<1>5. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
               SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>5
  <2>2. ~(cId = d /\ b = e)
    BY <1>1, <2>1, Zenon DEF SendMessage
  <2>3. QED
    BY <1>0, <2>1, <2>2, SendMovesOneEntry, SMT DEF IsFreedBuffer
<1>6. CASE UNCHANGED buffer_state
    BY <1>6, Zenon DEF IsFreedBuffer
<1>7. QED
    BY <1>2, <1>3, <1>4, <1>5, <1>6,
       OnlyBufferStepsWriteBufferStates, Zenon

\* Every lent buffer is given back and then released.  Two rungs: the host
\* returns it, which is its obligation, and the runtime frees it, which is
\* the runtime's.  The replay buffer lives in the gap.
\* Every fact of both rungs is a validity, so it is proved and boxed here
\* where no temporal hypothesis is in scope: PTL cannot box a fact that
\* holds only of the behaviour under Spec.
THEOREM BufferEventuallyFreedHolds == Spec => BufferEventuallyFreed
<1>0. ASSUME NEW cId \in CallIds, NEW b \in BufferIds
      PROVE  /\ [](TypeOK /\ LentCountMatchesBufferStates /\
                         IsLentBuffer(cId, b) =>
                             ENABLED <<HostReturnsBuffer(cId, b)>>_vars
                   )
             /\ [](TypeOK /\ <<HostReturnsBuffer(cId, b)>>_vars =>
                         (IsReturnedBuffer(cId, b))'
                   )
             /\ [](TypeOK /\ IsLentBuffer(cId, b) /\ [Next]_vars /\
                         ~<<HostReturnsBuffer(cId, b)>>_vars =>
                             (IsLentBuffer(cId, b))' \/
                                 (IsReturnedBuffer(cId, b))'
                   )
\* The second rung stands on the acquittal as well as on the state: the
\* release guard asks for both, so the WF1 has to carry both.
             /\ [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                         CarriesNoUnacquittedSend(cId, b) =>
                             ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
                   )
             /\ [](TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
                         (IsFreedBuffer(cId, b))'
                   )
             /\ [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                         CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
                         ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                             (IsReturnedBuffer(cId, b) /\
                                  CarriesNoUnacquittedSend(cId, b))'
                   )
             /\ [](TypeOK => (IsFreedBuffer(cId, b) =>
                         ~IsReturnedBuffer(cId, b))
                   )
             /\ [](TypeOK /\ [Next]_vars /\
                         (IsReturnedBuffer(cId, b) \/
                              IsFreedBuffer(cId, b))
                             => (IsReturnedBuffer(cId, b) \/
                                     IsFreedBuffer(cId, b))'
                   )
  <2>1. TypeOK /\ LentCountMatchesBufferStates /\
          IsLentBuffer(cId, b) =>
              ENABLED <<HostReturnsBuffer(cId, b)>>_vars
    BY ReturnBufferEnabled, Zenon
  <2>2. TypeOK /\ <<HostReturnsBuffer(cId, b)>>_vars =>
          (IsReturnedBuffer(cId, b))'
    BY SMT DEF HostReturnsBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>3. TypeOK /\ IsLentBuffer(cId, b) /\ [Next]_vars /\
          ~<<HostReturnsBuffer(cId, b)>>_vars =>
              (IsLentBuffer(cId, b))' \/ (IsReturnedBuffer(cId, b))'
    BY LentStaysOrIsReturned, Zenon
  <2>4. TypeOK /\ IsReturnedBuffer(cId, b) /\
          CarriesNoUnacquittedSend(cId, b) =>
              ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
    BY FreeBufferEnabled, Zenon
  <2>5. TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
          (IsFreedBuffer(cId, b))'
    BY SMT DEF FreeReturnedBuffer, IsFreedBuffer, TypeOK, L0!TypeOK
  <2>6. TypeOK /\ IsReturnedBuffer(cId, b) /\
          CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
          ~<<FreeReturnedBuffer(cId, b)>>_vars =>
              (IsReturnedBuffer(cId, b) /\
                   CarriesNoUnacquittedSend(cId, b))'
    <3>1. SUFFICES ASSUME TypeOK, IsReturnedBuffer(cId, b),
                          CarriesNoUnacquittedSend(cId, b), [Next]_vars,
                          ~<<FreeReturnedBuffer(cId, b)>>_vars
                   PROVE  (IsReturnedBuffer(cId, b) /\
                               CarriesNoUnacquittedSend(cId, b))'
      OBVIOUS
    <3>2. ~IsLentBuffer(cId, b)
      BY <3>1, SMT DEF IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
    <3>3. (IsReturnedBuffer(cId, b))'
      BY <3>1, BufferStateFrame, Zenon
    <3>4. QED
      BY <3>1, <3>2, <3>3, AcquittalPersistsOffLent
\* Freed and returned are two states, so reaching one leaves the other.
  <2>65. TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b))
    BY SMT DEF IsFreedBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
\* Once given back the buffer is returned or freed for good, which is what
\* lets the acquittal step and the release step be chained.
  <2>66. TypeOK /\ [Next]_vars /\
           (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b)) =>
               (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b))'
    BY ReturnedBufferStaysOrIsFreed, FreedStaysFreed, Zenon
  <2>7. QED
    BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>65, <2>66, PTL
<1>01. IndInv => LentCountMatchesBufferStates
    BY Zenon DEF IndInv, BufferStateInv
<1>1. ASSUME Spec, NEW cId \in CallIds, NEW b \in BufferIds
      PROVE  IsLentBuffer(cId, b) ~> IsFreedBuffer(cId, b)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. []TypeOK
    BY <2>1, IndInvParts, PTL
  <2>3. []LentCountMatchesBufferStates
    BY <2>1, <1>01, PTL
  <2>4. /\ WF_vars(HostReturnsBuffer(cId, b))
        /\ WF_vars(FreeReturnedBuffer(cId, b))
    BY <2>0, FairnessAtBuffer, IsaT(600)
  <2>45. /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
         /\ WF_vars(EmitWriteDone(cId))
         /\ WF_vars(WriteDoneReturns(cId))
    <3>1. [](TypeOK /\ FfiCallInv /\ BufferStateInv)
      BY <2>1, IndInvBufferParts, PTL
    <3>2. QED BY <2>0, <3>1, FairnessAtCall, IsaT(600)
  <2>5. IsLentBuffer(cId, b) ~> IsReturnedBuffer(cId, b)
    BY <1>0, <2>0, <2>2, <2>3, <2>4, PTL
\* The second rung is two steps now: the acquittal comes first, because the
\* release will not fire without it.
  <2>55. IsReturnedBuffer(cId, b) ~>
             (CarriesNoUnacquittedSend(cId, b) \/ IsFreedBuffer(cId, b))
    BY <2>0, <2>45, ReturnedBufferAcquits, PTL
  <2>56. (IsReturnedBuffer(cId, b) /\ CarriesNoUnacquittedSend(cId, b))
             ~> IsFreedBuffer(cId, b)
    BY <1>0, <2>0, <2>2, <2>4, PTL
  <2>6. IsReturnedBuffer(cId, b) ~> IsFreedBuffer(cId, b)
    BY <1>0, <2>0, <2>2, <2>55, <2>56, PTL
  <2>7. QED
    BY <2>5, <2>6, PTL
<1>2. QED BY <1>1, Zenon DEF BufferEventuallyFreed

\* Reclamation is enabled exactly when the debt is settled: the call over,
\* no payload owed, no buffer out, and the runtime still there.  The last
\* conjunct is what the destruction escape in the property answers.
\* The seven conjuncts of the guard, in one place: a terminal call that owes
\* nothing is reclaimable.  The last two are the arena's, and they are the
\* reason this lemma is not simply about the debt counters.
LEMMA ReleaseEnabled ==
    ASSUME NEW cId \in CallIds
    PROVE  TypeOK /\ L0!IsTerminalCall(cId) /\ ~IsHandleReleased(cId) /\
               ~IsRuntimeOfCallDestroyed(cId) /\ HostOwnsNoPayload(cId) /\
               HostHoldsNoBuffer(cId) /\ ~IsDeliveryCallbackRunning(cId) /\
               (\A b \in BufferIds : ~IsReturnedBuffer(cId, b)) =>
                   ENABLED <<ReleaseCallHandle(cId)>>_vars
<1>1. SUFFICES ASSUME TypeOK, L0!IsTerminalCall(cId), ~IsHandleReleased(cId),
                      ~IsRuntimeOfCallDestroyed(cId), HostOwnsNoPayload(cId),
                      HostHoldsNoBuffer(cId), ~IsDeliveryCallbackRunning(cId),
                      \A b \in BufferIds : ~IsReturnedBuffer(cId, b)
               PROVE  ENABLED <<ReleaseCallHandle(cId)>>_vars
    OBVIOUS
<1>2. ~L0!IsUnusedCall(cId) /\ ~L0!IsActiveCall(cId)
    BY <1>1, SMT
    DEF L0!IsTerminalCall, L0!IsUnusedCall, L0!IsActiveCall,
        L0!ActiveCallStates, TypeOK, L0!TypeOK, L0!CallStates
\* The step changes the state, said without a prime so it survives into the
\* expanded ENABLED as a hypothesis - the release latches the flag.
<1>25. [handle_released EXCEPT ![cId] = TRUE] # handle_released
  <2>1. handle_released[cId] = FALSE
    BY <1>1, SMT DEF IsHandleReleased, TypeOK, L0!TypeOK
  <2>2. [handle_released EXCEPT ![cId] = TRUE][cId] = TRUE
    BY <1>1, SMT DEF TypeOK, L0!TypeOK
  <2>3. QED BY <2>1, <2>2, Zenon
<1>3. QED
    BY <1>1, <1>2, <1>25, ExpandENABLED, SMTT(120)
    DEF ReleaseCallHandle, IsHandleReleased, HostOwnsNoPayload,
        HostHoldsNoBuffer, OwedPayloads, IsDeliveryCallbackRunning,
        IsReturnedBuffer,
        l0_vars, L0!vars, vars, ffi_vars

\* On a terminal call nothing is lent, delivered, or started: every step
\* that would do so needs the call active.  So the three components of the
\* debt only ever fall, and each falls under one action that carries
\* fairness.
LEMMA TerminalCallDebtOnlyFalls ==
    ASSUME NEW cId \in CallIds, TypeOK, L0!IsTerminalCall(cId),
           [Next]_vars
    PROVE  /\ \A b \in BufferIds :
                 ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))'
           /\ (~IsDeliveryCallbackRunning(cId) =>
                 (~IsDeliveryCallbackRunning(cId))')
           /\ (events_delivered[cId])' = events_delivered[cId]
<1>1. ~L0!IsActiveCall(cId)
    BY SMT
    DEF L0!IsTerminalCall, L0!IsActiveCall, L0!ActiveCallStates,
        TypeOK, L0!TypeOK, L0!CallStates
\* Lending is the only step that makes a buffer lent, and it needs an
\* active call.
<1>2. \A b \in BufferIds, msg \in Messages, ch \in Sizes :
                 ~LendSendBuffer(cId, b, msg, ch)
    BY <1>1, Zenon DEF LendSendBuffer
<1>3. \A b \in BufferIds :
          ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))'
  <2>1. SUFFICES ASSUME NEW b \in BufferIds, ~IsLentBuffer(cId, b)
                 PROVE  (~IsLentBuffer(cId, b))'
    OBVIOUS
  <2>2. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>2, <2>1, <2>2, SMT DEF LendSendBuffer, RefuseLendTooLarge,
        RefuseLendForSlot, RefuseLendForBudget, IsLentBuffer, TypeOK, L0!TypeOK
  <2>3. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <2>1, <2>3, SMT DEF HostReturnsBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>4. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <2>1, <2>4, SMT DEF FreeReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>5. CASE \E d \in CallIds, m \in Messages,
                bs \in BufferIds : SendMessage(d, m, bs)
    BY <2>1, <2>5, SMT DEF SendMessage, IsLentBuffer, TypeOK, L0!TypeOK
  <2>6. CASE UNCHANGED buffer_state
    BY <2>1, <2>6, Zenon DEF IsLentBuffer
  <2>7. QED
    BY <2>2, <2>3, <2>4, <2>5, <2>6,
       OnlyBufferStepsWriteBufferStates, Zenon
\* A delivery is what puts a callback on the stack, and it needs an active
\* call too; so does anything that appends an event.
\* Appending an event needs an active call, so the trace is frozen.
<1>4. (events_delivered[cId])' = events_delivered[cId]
  <2>1. CASE \E d \in CallIds : DeliverInitialMetadata(d)
    BY <1>1, <2>1, SMT
    DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
  <2>2. CASE \E d \in CallIds : DeliverMessage(d)
    BY <1>1, <2>2, SMT
    DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
  <2>3. CASE \E d \in CallIds : DeliverStatus(d)
    BY <1>1, <2>3, SMT
    DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
  <2>4. CASE \E d \in CallIds : DeliverCancelled(d)
    BY <1>1, <2>4, SMT
    DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost,
        L0!IsActiveCall, L0!ActiveCallStates, TypeOK, L0!TypeOK
  <2>5. QED
    BY <2>1, <2>2, <2>3, <2>4,
       EveryStepEitherDeliversOrKeepsEvents, Zenon
\* And a callback goes on the stack only in HandPayloadToHost, which the
\* four deliveries carry and which needs an active call.
<1>45. ~IsDeliveryCallbackRunning(cId) =>
           (~IsDeliveryCallbackRunning(cId))'
  <2>0. SUFFICES ASSUME ~IsDeliveryCallbackRunning(cId)
                 PROVE  (~IsDeliveryCallbackRunning(cId))'
    OBVIOUS
  <2>1. CASE \/ NextSafeRuntimeOnly
             \/ NextSafeRuntimeChannel
             \/ NextSafeChannelOnly
             \/ NextSafeChannelCall
    <3>1. UNCHANGED delivery_callback_running
      BY <2>1, RuntimeAndChannelStepsKeepDeliveryFlags
    <3>2. QED BY <2>0, <3>1, Zenon DEF IsDeliveryCallbackRunning
  <2>2. CASE NextSafeFfiOnly
    BY <2>0, <2>2, FfiOnlyStepsNeverStartDelivery
  <2>3. CASE NextSafeCallOnly
    <3>1. CASE \/ \E c \in CallIds, ch \in ChannelIds : CallStart(c, ch)
               \/ \E c \in CallIds, m \in Messages,
                     bs \in BufferIds : SendMessage(c, m, bs)
               \/ \E c \in CallIds : EndSend(c)
               \/ \E c \in CallIds : NetworkSend(c)
               \/ \E c \in CallIds, m \in Messages : NetworkReceive(c, m)
               \/ \E c \in CallIds : ReceiveStatus(c)
      <4>1. UNCHANGED delivery_callback_running
        BY <3>1, SMT
        DEF CallStart, SendMessage, EndSend, NetworkSend, NetworkReceive,
            ReceiveStatus, ffi_vars
      <4>2. QED BY <2>0, <4>1, Zenon DEF IsDeliveryCallbackRunning
    <3>2. CASE \E d \in CallIds : DeliverInitialMetadata(d)
      BY <1>1, <2>0, <3>2, SMT
      DEF DeliverInitialMetadata, L0!DeliverInitialMetadata, HandPayloadToHost,
          IsDeliveryCallbackRunning, L0!IsActiveCall, L0!ActiveCallStates,
          TypeOK, L0!TypeOK
    <3>3. CASE \E d \in CallIds : DeliverMessage(d)
      BY <1>1, <2>0, <3>3, SMT
      DEF DeliverMessage, L0!DeliverMessage, HandPayloadToHost,
          IsDeliveryCallbackRunning, L0!IsActiveCall, L0!ActiveCallStates,
          TypeOK, L0!TypeOK
    <3>4. CASE \E d \in CallIds : DeliverStatus(d)
      BY <1>1, <2>0, <3>4, SMT
      DEF DeliverStatus, L0!DeliverStatus, HandPayloadToHost,
          IsDeliveryCallbackRunning, L0!IsActiveCall, L0!ActiveCallStates,
          TypeOK, L0!TypeOK
    <3>5. CASE \E d \in CallIds : DeliverCancelled(d)
      BY <1>1, <2>0, <3>5, SMT
      DEF DeliverCancelled, L0!CallCancel, HandPayloadToHost,
          IsDeliveryCallbackRunning, L0!IsActiveCall, L0!ActiveCallStates,
          TypeOK, L0!TypeOK
    <3>6. QED
      BY <2>3, <3>1, <3>2, <3>3, <3>4, <3>5 DEF NextSafeCallOnly
  <2>4. CASE NextFail \/ NextExplicitStutter
    <3>1. UNCHANGED ffi_vars
      BY <2>4, SMT
      DEF NextFail, NextExplicitStutter, RuntimeFail, RemainFailed,
          RemainReleased
    <3>2. QED
      BY <2>0, <3>1, SMT DEF ffi_vars, IsDeliveryCallbackRunning
  <2>5. CASE UNCHANGED vars
    BY <2>0, <2>5, SMT DEF vars, ffi_vars, IsDeliveryCallbackRunning
  <2>6. QED
    BY <2>1, <2>2, <2>3, <2>4, <2>5, NextDecomposition
    DEF NextByFootprint, NextSafe, NextSafeRefining
<1>5. QED BY <1>3, <1>4, <1>45

\* One buffer leaves the lent state: either the host gives it back, which
\* fairness promises, or a send commits it - and both land in returned.
THEOREM TerminalBufferReturns ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ []TypeOK
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ WF_vars(HostReturnsBuffer(cId, b))
           => (IsLentBuffer(cId, b) ~> ~IsLentBuffer(cId, b))
<1>0. /\ [](TypeOK /\ LentCountMatchesBufferStates /\
             IsLentBuffer(cId, b) =>
                 ENABLED <<HostReturnsBuffer(cId, b)>>_vars)
      /\ [](TypeOK /\ <<HostReturnsBuffer(cId, b)>>_vars =>
             (~IsLentBuffer(cId, b))')
      /\ [](TypeOK /\ IsLentBuffer(cId, b) /\ [Next]_vars /\
             ~<<HostReturnsBuffer(cId, b)>>_vars =>
                 (IsLentBuffer(cId, b))' \/ (~IsLentBuffer(cId, b))')
  <2>1. TypeOK /\ LentCountMatchesBufferStates /\ IsLentBuffer(cId, b) =>
            ENABLED <<HostReturnsBuffer(cId, b)>>_vars
    BY ReturnBufferEnabled, Zenon
  <2>2. TypeOK /\ <<HostReturnsBuffer(cId, b)>>_vars =>
            (~IsLentBuffer(cId, b))'
    BY SMT DEF HostReturnsBuffer, IsLentBuffer, TypeOK, L0!TypeOK
  <2>3. TypeOK /\ IsLentBuffer(cId, b) /\ [Next]_vars /\
            ~<<HostReturnsBuffer(cId, b)>>_vars =>
                (IsLentBuffer(cId, b))' \/ (~IsLentBuffer(cId, b))'
    OBVIOUS
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>1. QED BY <1>0, PTL

\* The finite lift, kept free of any behavioural hypothesis: PTL and the
\* set induction only work outside a temporal context.
THEOREM AllBuffersSettle ==
    ASSUME NEW cId \in CallIds
    PROVE  (\A b \in BufferIds : <>[]~IsLentBuffer(cId, b))
               => <>[](\A b \in BufferIds : ~IsLentBuffer(cId, b))
<1>0. USE BufferIdsAreAFiniteNonemptySet DEF BufferIdsAreAFiniteNonemptySet
<1> DEFINE G(b) == ~IsLentBuffer(cId, b)
           K(b) == <>[]G(b)
           I(T) == (\A b \in T : K(b)) =>
                       <>[](\A b \in T : G(b))
<1>1. I({})
  <2>1. \A b \in {} : G(b)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET BufferIds, NEW x \in BufferIds \ T
       PROVE <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
                 <>[](\A b \in T \cup {x} : G(b))
  <2>1. (\A b \in T : G(b)) /\ G(x) =>
            (\A b \in T \cup {x} : G(b))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET BufferIds, IsFiniteSet(T), I(T),
             NEW x \in BufferIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A b \in T \cup {x} : K(b)) =>
            (\A b \in T : K(b)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
            <>[](\A b \in T \cup {x} : G(b))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(BufferIds)
    BY <1>1, <1>2, FS_Induction, IsaM("blast")
<1>4. QED BY <1>3, Zenon DEF I

\* The per-buffer fairness bundle, behind a name.  A quantifier under a box
\* is two unrelated atoms to the temporal backend: it is propositional, so it
\* cannot cross \A to see that [](\A b : P(b)) gives \A b : P(b).  Naming the
\* bundle leaves it one atom, and only the boxing of that atom is temporal.
BufferFairnessFor(cId) ==
    \A b \in BufferIds :
        /\ WF_vars(HostReturnsBuffer(cId, b))
        /\ WF_vars(FreeReturnedBuffer(cId, b))

\* First drain: the buffers.  On a terminal call a buffer that has left the
\* lent state never comes back to it, so each one settles for good, and
\* finitely many of them settle together.
\* The stability fact is a validity, so it is boxed out here.
THEOREM TerminalCallBuffersSettle ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []TypeOK
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ BufferFairnessFor(cId)
           => <>[](\A b \in BufferIds : ~IsLentBuffer(cId, b))
<1>0. ASSUME NEW b \in BufferIds
      PROVE  [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
                 ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))')
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
            ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))'
    BY TerminalCallDebtOnlyFalls, Zenon
  <2>2. QED BY <2>1, PTL
<1>1. ASSUME []TypeOK, []LentCountMatchesBufferStates, [][Next]_vars,
             [](L0!IsTerminalCall(cId)),
             BufferFairnessFor(cId)
                PROVE  <>[](\A b \in BufferIds : ~IsLentBuffer(cId, b))
  <2>1. ASSUME NEW b \in BufferIds
        PROVE  <>[]~IsLentBuffer(cId, b)
    <3>1. WF_vars(HostReturnsBuffer(cId, b))
      BY <1>1, IsaT(600) DEF BufferFairnessFor
    <3>2. IsLentBuffer(cId, b) ~> ~IsLentBuffer(cId, b)
      BY <1>1, <3>1, TerminalBufferReturns, PTL
    <3>3. QED
      BY <1>0, <1>1, <3>2, PTL
  <2>2. QED BY <2>1, AllBuffersSettle
<1>2. QED BY <1>1
\* And the two reclamation guarantees, which are what the removal of
\* ak_call_release from the ABI buys: the runtime does the work, so the
\* model can promise it.
\* Terminal is where a call ends: no step leaves that state.  Needed because
\* every drain below is stated under a standing terminal hypothesis, and the
\* guarantee starts from a single terminal state.
\* Terminal is where a call ends: no step leaves that state.  Stated over the
\* level-0 step, because the level-0 machinery is the only writer of a call
\* state and reading it there keeps the obligation to the five actions that
\* write it rather than to every action of the refinement.
LEMMA TerminalCallStaysTerminal ==
    ASSUME NEW cId \in CallIds, TypeOK, [Next]_vars,
           L0!IsTerminalCall(cId)
    PROVE  (L0!IsTerminalCall(cId))'
<1>0. call_state \in [CallIds -> L0!CallStates]
    BY Zenon DEF TypeOK, L0!TypeOK
<1>1. [L0!Next]_l0_vars
    BY RefinesNext
<1>2. CASE L0!Next
\* Every writer of a call state names the state it leaves, and none of them
\* names terminal.
    BY <1>0, <1>2, SMTT(60)
    DEF L0!Next, L0!CallStart, L0!SendMessage, L0!EndSend,
        L0!NetworkSend, L0!NetworkReceive, L0!ReceiveStatus,
        L0!DeliverInitialMetadata, L0!DeliverMessage, L0!DeliverStatus,
        L0!CallCancel, L0!RuntimeCreate, L0!RuntimeBeginShutdown,
        L0!RuntimeRelease, L0!RuntimeFail, L0!RemainFailed,
        L0!RemainReleased, L0!ChannelCreate, L0!ChannelStartClosing,
        L0!ChannelFinishClosing,
        L0!RuntimeVars, L0!ChannelVars, L0!CallVars, L0!vars,
        L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall,
        L0!ActiveCallStates, L0!CallStates
<1>3. CASE UNCHANGED l0_vars
    BY <1>0, <1>3, SMT
    DEF l0_vars, L0!vars, L0!CallVars, L0!IsTerminalCall
<1>4. QED BY <1>1, <1>2, <1>3

\* The delivery side of a terminal call goes quiet: the callback returns
\* because the host owes that, and no step starts another one because
\* starting one needs an active call.
THEOREM TerminalDeliveryQuiets ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ []TypeOK
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           => <>[]~IsDeliveryCallbackRunning(cId)
<1>1. [](TypeOK /\ IsDeliveryCallbackRunning(cId) =>
              ENABLED <<DeliveryCallbackReturns(cId)>>_vars)
  <2>1. TypeOK /\ IsDeliveryCallbackRunning(cId) =>
            ENABLED <<DeliveryCallbackReturns(cId)>>_vars
    BY DeliveryCallbackReturnsEnabled, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
              (~IsDeliveryCallbackRunning(cId))')
  <2>1. TypeOK /\ <<DeliveryCallbackReturns(cId)>>_vars =>
            (~IsDeliveryCallbackRunning(cId))'
    BY DeliveryReturnClears, Zenon
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
              ~<<DeliveryCallbackReturns(cId)>>_vars =>
                  (IsDeliveryCallbackRunning(cId))')
  <2>1. TypeOK /\ IsDeliveryCallbackRunning(cId) /\ [Next]_vars /\
            ~<<DeliveryCallbackReturns(cId)>>_vars =>
                (IsDeliveryCallbackRunning(cId))'
    BY CallbackFrame, Zenon
  <2>2. QED BY <2>1, PTL
<1>4. [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
              ~IsDeliveryCallbackRunning(cId) =>
                  (~IsDeliveryCallbackRunning(cId))')
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
            ~IsDeliveryCallbackRunning(cId) =>
                (~IsDeliveryCallbackRunning(cId))'
    BY TerminalCallDebtOnlyFalls, Zenon
  <2>2. QED BY <2>1, PTL
<1>5. QED BY <1>1, <1>2, <1>3, <1>4, PTL

\* The payload debt of a terminal call falls by one under the host's own
\* fairness conjunct.  One action moves it, so this is a plain WF1 with no
\* pairing: nothing here waits for a callback to unwind.
THEOREM TerminalPayloadDescent ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           => ((OwedPayloads(cId) = n + 1) ~> (OwedPayloads(cId) = n))
\* A debt of at least one means the last delivered payload is still owed, and
\* that is the index the consume step is enabled at.
<1>10. [](TypeOK /\ OwedPayloads(cId) = n + 1 =>
              ENABLED <<HostConsumesEvent(cId)>>_vars)
  <2>1. TypeOK /\ OwedPayloads(cId) = n + 1 =>
            ENABLED <<HostConsumesEvent(cId)>>_vars
    <3>1. SUFFICES ASSUME TypeOK, OwedPayloads(cId) = n + 1
                   PROVE  ENABLED <<HostConsumesEvent(cId)>>_vars
      OBVIOUS
    <3>2. /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
          /\ payloads_consumed_by_host \in [CallIds -> Nat]
          /\ Len(events_delivered[cId]) \in Nat
      BY <3>1, LenProperties, Zenon DEF TypeOK, L0!TypeOK
    <3>3. Len(events_delivered[cId]) \in PayloadIndices
      BY <3>2 DEF PayloadIndices
    <3>4. HostOwnsPayload(cId, Len(events_delivered[cId]))
      BY <3>1, <3>2, SMT DEF HostOwnsPayload, OwedPayloads
    <3>5. QED
      BY <3>1, <3>3, <3>4, HostConsumesEventEnabled
  <2>2. QED BY <2>1, PTL
<1>20. [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
              OwedPayloads(cId) = n + 1 /\
              <<HostConsumesEvent(cId)>>_vars =>
                  (OwedPayloads(cId) = n)')
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
            OwedPayloads(cId) = n + 1 /\
            <<HostConsumesEvent(cId)>>_vars =>
                (OwedPayloads(cId) = n)'
    <3>1. SUFFICES ASSUME TypeOK, L0!IsTerminalCall(cId), [Next]_vars,
                          OwedPayloads(cId) = n + 1,
                          <<HostConsumesEvent(cId)>>_vars
                   PROVE  (OwedPayloads(cId) = n)'
      OBVIOUS
    <3>2. (events_delivered[cId])' = events_delivered[cId]
      BY <3>1, TerminalCallDebtOnlyFalls
    <3>3. (payloads_consumed_by_host[cId])' =
              payloads_consumed_by_host[cId] + 1
      BY <3>1, ConsumeAdvancesRelease
    <3>4. /\ payloads_consumed_by_host \in [CallIds -> Nat]
          /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
          /\ Len(events_delivered[cId]) \in Nat
      BY <3>1, LenProperties, Zenon DEF TypeOK, L0!TypeOK
    <3>5. QED
      BY <3>1, <3>2, <3>3, <3>4, SMT DEF OwedPayloads
  <2>2. QED BY <2>1, PTL
<1>30. [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
              ~<<HostConsumesEvent(cId)>>_vars /\
              OwedPayloads(cId) = n + 1 =>
                  (OwedPayloads(cId) = n + 1)')
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
            ~<<HostConsumesEvent(cId)>>_vars /\
            OwedPayloads(cId) = n + 1 =>
                (OwedPayloads(cId) = n + 1)'
    <3>1. SUFFICES ASSUME TypeOK, L0!IsTerminalCall(cId), [Next]_vars,
                          ~<<HostConsumesEvent(cId)>>_vars,
                          OwedPayloads(cId) = n + 1
                   PROVE  (OwedPayloads(cId) = n + 1)'
      OBVIOUS
    <3>2. (events_delivered[cId])' = events_delivered[cId]
      BY <3>1, TerminalCallDebtOnlyFalls
    <3>3. (payloads_consumed_by_host[cId])' =
              payloads_consumed_by_host[cId]
      BY <3>1, ReleasesOnlyRiseOnConsume
    <3>4. QED
      BY <3>1, <3>2, <3>3, Zenon DEF OwedPayloads
  <2>2. QED BY <2>1, PTL
<1>1. QED BY <1>10, <1>20, <1>30, PTL

\* The bound splits of the payload ladder, standalone for the same reason as
\* the buffer ones.
LEMMA TerminalPayloadBoundSplit ==
    ASSUME NEW cId \in CallIds, NEW n \in Nat
    PROVE  /\ [](TypeOK /\ OwedPayloads(cId) <= n + 1 =>
                     \/ OwedPayloads(cId) <= n
                     \/ OwedPayloads(cId) = n + 1)
           /\ [](TypeOK /\ OwedPayloads(cId) = n =>
                     OwedPayloads(cId) <= n)
<1>1. TypeOK /\ OwedPayloads(cId) <= n + 1 =>
          \/ OwedPayloads(cId) <= n
          \/ OwedPayloads(cId) = n + 1
    BY SMT DEF OwedPayloads, TypeOK, L0!TypeOK
<1>2. TypeOK /\ OwedPayloads(cId) = n => OwedPayloads(cId) <= n
    BY SMT DEF OwedPayloads, TypeOK, L0!TypeOK
<1>3. QED BY <1>1, <1>2, PTL

\* And the ladder, instantiated at DeliveryCredits + 1: the delivery window
\* bounds the debt exactly as the send window bounds a buffer's.
THEOREM TerminalPayloadsDrain ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv)
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           => <>[]HostOwnsNoPayload(cId)
<1>0. DeliveryCredits + 1 \in Nat
    BY DeliveryCreditsArePositive, SMT DEF DeliveryCreditsArePositive
<1>1. ASSUME [](TypeOK /\ FfiCallInv),
             [][Next]_vars,
             [](L0!IsTerminalCall(cId)),
             WF_vars(HostConsumesEvent(cId))
      PROVE  <>[]HostOwnsNoPayload(cId)
  <2> DEFINE Ind(n) == (OwedPayloads(cId) <= n) ~> HostOwnsNoPayload(cId)
  <2>2. Ind(0)
    <3>1. TypeOK /\ FfiCallInv /\ OwedPayloads(cId) <= 0 =>
              HostOwnsNoPayload(cId)
      BY SMT
      DEF OwedPayloads, HostOwnsNoPayload, FfiCallInv,
          ReleasesNeverExceedDeliveries, TypeOK, L0!TypeOK
    <3>2. [](TypeOK /\ FfiCallInv /\ OwedPayloads(cId) <= 0 =>
                 HostOwnsNoPayload(cId))
      BY <3>1, PTL
    <3>3. QED BY <1>1, <3>2, PTL
  <2>3. ASSUME NEW n \in Nat, Ind(n)
        PROVE Ind(n + 1)
    <3>1. (OwedPayloads(cId) = n + 1) ~> (OwedPayloads(cId) = n)
      BY <1>1, <2>3, TerminalPayloadDescent, PTL
    <3>2. [](TypeOK /\ OwedPayloads(cId) <= n + 1 =>
                 \/ OwedPayloads(cId) <= n
                 \/ OwedPayloads(cId) = n + 1)
      BY <2>3, TerminalPayloadBoundSplit
    <3>3. [](TypeOK /\ OwedPayloads(cId) = n => OwedPayloads(cId) <= n)
      BY <2>3, TerminalPayloadBoundSplit
    <3>4. []TypeOK
      BY <1>1, PTL
    <3>5. QED BY <2>3, <3>1, <3>2, <3>3, <3>4, PTL
  <2> HIDE DEF Ind
  <2>4. \A n \in Nat : Ind(n)
    BY <2>2, <2>3, NatInduction, IsaT(600)
  <2>5. Ind(DeliveryCredits + 1)
    BY <1>0, <2>4
  <2>6. [](TypeOK /\ FfiCallInv => OwedPayloads(cId) <= DeliveryCredits + 1)
    <3>1. TypeOK /\ FfiCallInv => OwedPayloads(cId) <= DeliveryCredits + 1
      BY Zenon
      DEF FfiCallInv, PayloadsOwnedWithinCreditsPlusOne,
          HostOwnsAtMostCreditsPlusOne
    <3>2. QED BY <3>1, PTL
\* Once nothing is owed nothing becomes owed: a terminal call receives no
\* further delivery, and consumption only rises.
  <2>7. [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
               HostOwnsNoPayload(cId) => (HostOwnsNoPayload(cId))')
    <3>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
              HostOwnsNoPayload(cId) => (HostOwnsNoPayload(cId))'
      <4>1. SUFFICES ASSUME TypeOK, L0!IsTerminalCall(cId), [Next]_vars,
                            HostOwnsNoPayload(cId)
                     PROVE  (HostOwnsNoPayload(cId))'
        OBVIOUS
      <4>2. (events_delivered[cId])' = events_delivered[cId]
        BY <4>1, TerminalCallDebtOnlyFalls
      <4>25. /\ payloads_consumed_by_host \in [CallIds -> Nat]
             /\ events_delivered \in [CallIds -> Seq(L0!EventKinds)]
             /\ Len(events_delivered[cId]) \in Nat
        BY <4>1, LenProperties, Zenon DEF TypeOK, L0!TypeOK
\* Nothing owed means nothing to consume, so the counter does not move at
\* all - an inequality would leave the debt free to go negative.
      <4>3. (payloads_consumed_by_host[cId])' =
                payloads_consumed_by_host[cId]
        <5>1. ~<<HostConsumesEvent(cId)>>_vars
          BY <4>1, <4>25, SMT
          DEF HostConsumesEvent, HostOwnsSomePayload, HostOwnsNoPayload,
              OwedPayloads
        <5>2. QED
          BY <4>1, <5>1, ReleasesOnlyRiseOnConsume
      <4>4. QED
        BY <4>1, <4>2, <4>3, <4>25, SMT
        DEF HostOwnsNoPayload, OwedPayloads, TypeOK, L0!TypeOK
    <3>2. QED BY <3>1, PTL
  <2>8. QED BY <1>1, <2>5, <2>6, <2>7, PTL DEF Ind
<1>2. QED BY <1>1, PTL

\* Off the lent state a buffer never becomes returned: both writers of that
\* state - the host giving it back and a send committing it - need it lent.
LEMMA NotReturnedStaysOffLent ==
    ASSUME TypeOK, [Next]_vars, NEW cId \in CallIds, NEW b \in BufferIds,
           ~IsLentBuffer(cId, b), ~IsReturnedBuffer(cId, b)
    PROVE  ~(IsReturnedBuffer(cId, b))'
<1>0. /\ buffer_state \in [CallIds -> [BufferIds -> BufferStates]]
      /\ buffer_send \in [CallIds -> [BufferIds -> Nat]]
      /\ submitted \in [CallIds -> Seq(Messages)]
      /\ write_dones_emitted \in [CallIds -> Nat]
    BY TypeOKSplit, Zenon DEF TypeOK, L0!TypeOK, BufferTypes
<1>1. CASE \E d \in CallIds, e \in BufferIds, msg \in Messages, ch \in Sizes : LendSendBuffer(d, e, msg, ch)
    BY <1>0, <1>1, LendMovesOneEntry, SMT DEF IsReturnedBuffer
<1>2. CASE \E d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
  <2>1. PICK d \in CallIds, e \in BufferIds : HostReturnsBuffer(d, e)
    BY <1>2
  <2>2. ~(cId = d /\ b = e)
    BY <2>1, Zenon DEF HostReturnsBuffer
  <2>3. QED
    BY <1>0, <2>1, <2>2, ReturnMovesOneEntry, SMT DEF IsReturnedBuffer
<1>3. CASE \E d \in CallIds, e \in BufferIds : FreeReturnedBuffer(d, e)
    BY <1>0, <1>3, FreeMovesOneEntry, SMT DEF IsReturnedBuffer
<1>4. CASE \E d \in CallIds, m \in Messages, e \in BufferIds :
               SendMessage(d, m, e)
  <2>1. PICK d \in CallIds, m \in Messages, e \in BufferIds :
            SendMessage(d, m, e)
    BY <1>4
  <2>2. ~(cId = d /\ b = e)
    BY <2>1, Zenon DEF SendMessage
  <2>3. QED
    BY <1>0, <2>1, <2>2, SendMovesOneEntry, SMT DEF IsReturnedBuffer
<1>5. CASE UNCHANGED buffer_state
    BY <1>5, Zenon DEF IsReturnedBuffer
<1>6. QED
    BY <1>1, <1>2, <1>3, <1>4, <1>5,
       OnlyBufferStepsWriteBufferStates, Zenon

\* The lift twin of AllBuffersSettle, for the state the release guard reads.
THEOREM AllBuffersUnreturned ==
    ASSUME NEW cId \in CallIds
    PROVE  (\A b \in BufferIds : <>[]~IsReturnedBuffer(cId, b))
               => <>[](\A b \in BufferIds : ~IsReturnedBuffer(cId, b))
<1>0. USE BufferIdsAreAFiniteNonemptySet DEF BufferIdsAreAFiniteNonemptySet
<1> DEFINE G(b) == ~IsReturnedBuffer(cId, b)
           K(b) == <>[]G(b)
           I(T) == (\A b \in T : K(b)) => <>[](\A b \in T : G(b))
<1>1. I({})
  <2>1. \A b \in {} : G(b)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET BufferIds, NEW x \in BufferIds \ T
       PROVE <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
                 <>[](\A b \in T \cup {x} : G(b))
  <2>1. (\A b \in T : G(b)) /\ G(x) => (\A b \in T \cup {x} : G(b))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET BufferIds, IsFiniteSet(T), I(T),
             NEW x \in BufferIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A b \in T \cup {x} : K(b)) => (\A b \in T : K(b)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
            <>[](\A b \in T \cup {x} : G(b))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(BufferIds)
    BY <1>1, <1>2, FS_Induction, IsaM("blast")
<1>4. QED BY <1>3, Zenon DEF I

\* The second rung of the buffer chain on its own, so the reclamation proof
\* can cite it: a buffer given back is released, the acquittal first and the
\* release after.
THEOREM ReturnedBufferFreedFor ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ [][Next]_vars
           /\ WF_vars(FreeReturnedBuffer(cId, b))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => (IsReturnedBuffer(cId, b) ~> IsFreedBuffer(cId, b))
<1>1. [](TypeOK /\ IsReturnedBuffer(cId, b) /\
              CarriesNoUnacquittedSend(cId, b) =>
                  ENABLED <<FreeReturnedBuffer(cId, b)>>_vars)
  <2>1. TypeOK /\ IsReturnedBuffer(cId, b) /\
            CarriesNoUnacquittedSend(cId, b) =>
                ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
    BY FreeBufferEnabled, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
              (IsFreedBuffer(cId, b))')
  <2>1. TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
            (IsFreedBuffer(cId, b))'
    BY SMT DEF FreeReturnedBuffer, IsFreedBuffer, TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ IsReturnedBuffer(cId, b) /\
              CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
              ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                  (IsReturnedBuffer(cId, b) /\
                       CarriesNoUnacquittedSend(cId, b))')
  <2>1. TypeOK /\ IsReturnedBuffer(cId, b) /\
            CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
            ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                (IsReturnedBuffer(cId, b) /\
                     CarriesNoUnacquittedSend(cId, b))'
    <3>1. SUFFICES ASSUME TypeOK, IsReturnedBuffer(cId, b),
                          CarriesNoUnacquittedSend(cId, b), [Next]_vars,
                          ~<<FreeReturnedBuffer(cId, b)>>_vars
                   PROVE  (IsReturnedBuffer(cId, b) /\
                               CarriesNoUnacquittedSend(cId, b))'
      OBVIOUS
    <3>2. ~IsLentBuffer(cId, b)
      BY <3>1, SMT DEF IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
    <3>3. (IsReturnedBuffer(cId, b))'
      BY <3>1, BufferStateFrame, Zenon
    <3>4. QED
      BY <3>1, <3>2, <3>3, AcquittalPersistsOffLent
  <2>2. QED BY <2>1, PTL
<1>4. [](TypeOK /\ [Next]_vars /\
              (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b)) =>
                  (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b))')
  <2>1. TypeOK /\ [Next]_vars /\
            (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b)) =>
                (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b))'
    BY ReturnedBufferStaysOrIsFreed, FreedStaysFreed, Zenon
  <2>2. QED BY <2>1, PTL
<1>5. [](TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b)))
  <2>1. TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b))
    BY SMT DEF IsFreedBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, PTL
<1>6. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             [][Next]_vars,
             WF_vars(FreeReturnedBuffer(cId, b)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  IsReturnedBuffer(cId, b) ~> IsFreedBuffer(cId, b)
  <2>1. IsReturnedBuffer(cId, b) ~>
            (CarriesNoUnacquittedSend(cId, b) \/ IsFreedBuffer(cId, b))
    BY <1>6, ReturnedBufferAcquits, PTL
  <2>2. (IsReturnedBuffer(cId, b) /\ CarriesNoUnacquittedSend(cId, b))
            ~> IsFreedBuffer(cId, b)
    BY <1>1, <1>2, <1>3, <1>6, PTL
  <2>3. QED BY <1>4, <1>5, <1>6, <2>1, <2>2, PTL
<1>7. QED BY <1>6, PTL

\* So on a terminal call no buffer stays given back: it is lent at most until
\* the host hands it over, and given back at most until the runtime releases
\* it.  This is the last conjunct of the release guard.
THEOREM TerminalBufferUnreturned ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ WF_vars(HostReturnsBuffer(cId, b))
           /\ WF_vars(FreeReturnedBuffer(cId, b))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           => <>[]~IsReturnedBuffer(cId, b)
<1>1. [](TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
              ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))')
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ [Next]_vars /\
            ~IsLentBuffer(cId, b) => (~IsLentBuffer(cId, b))'
    BY TerminalCallDebtOnlyFalls, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](TypeOK /\ [Next]_vars /\ ~IsLentBuffer(cId, b) /\
              ~IsReturnedBuffer(cId, b) => ~(IsReturnedBuffer(cId, b))')
  <2>1. TypeOK /\ [Next]_vars /\ ~IsLentBuffer(cId, b) /\
            ~IsReturnedBuffer(cId, b) => ~(IsReturnedBuffer(cId, b))'
    BY NotReturnedStaysOffLent, Zenon
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ [Next]_vars /\ IsFreedBuffer(cId, b) =>
              (IsFreedBuffer(cId, b))')
  <2>1. TypeOK /\ [Next]_vars /\ IsFreedBuffer(cId, b) =>
            (IsFreedBuffer(cId, b))'
    BY FreedStaysFreed, Zenon
  <2>2. QED BY <2>1, PTL
<1>4. [](TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b)))
  <2>1. TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b))
    BY SMT DEF IsFreedBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, PTL
<1>5. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             [](L0!IsTerminalCall(cId)),
             WF_vars(HostReturnsBuffer(cId, b)),
             WF_vars(FreeReturnedBuffer(cId, b)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId))
      PROVE  <>[]~IsReturnedBuffer(cId, b)
  <2>0. []TypeOK
    BY <1>5, PTL
  <2>1. IsLentBuffer(cId, b) ~> ~IsLentBuffer(cId, b)
    BY <1>5, <2>0, TerminalBufferReturns, PTL
  <2>2. <>[]~IsLentBuffer(cId, b)
    BY <1>1, <1>5, <2>0, <2>1, PTL
  <2>3. IsReturnedBuffer(cId, b) ~> IsFreedBuffer(cId, b)
    BY <1>5, ReturnedBufferFreedFor, PTL
  <2>4. QED
    BY <1>2, <1>3, <1>4, <1>5, <2>0, <2>2, <2>3, PTL
<1>6. QED BY <1>5, PTL

\* The counter side of the buffer states: nothing lent means nothing held.
LEMMA NoLentMeansNoneHeld ==
    ASSUME TypeOK, LentCountMatchesBufferStates, NEW cId \in CallIds,
           \A b \in BufferIds : ~IsLentBuffer(cId, b)
    PROVE  HostHoldsNoBuffer(cId)
<1>1. {b \in BufferIds : IsLentBuffer(cId, b)} = {}
    BY Zenon
<1>2. Cardinality({b \in BufferIds : IsLentBuffer(cId, b)}) = 0
    BY <1>1, FS_EmptySet, Zenon
<1>3. QED
    BY <1>2, Zenon DEF LentCountMatchesBufferStates, HostHoldsNoBuffer

\* Reclamation, under a standing terminal hypothesis.  The four conjuncts of
\* the release guard that a terminal call has to work off each settle for
\* good, so from some point the step is enabled and stays enabled; weak
\* fairness then fires it.  A theorem rather than a step, because the
\* consumer has to necessitate it.
THEOREM TerminalCallReclaims ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ [](L0!IsTerminalCall(cId))
           /\ WF_vars(ReleaseCallHandle(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ BufferFairnessFor(cId)
           => <>(IsHandleReleased(cId) \/ IsRuntimeOfCallDestroyed(cId))
<1>1. [](TypeOK /\ L0!IsTerminalCall(cId) /\ ~IsHandleReleased(cId) /\
              ~IsRuntimeOfCallDestroyed(cId) /\ HostOwnsNoPayload(cId) /\
              HostHoldsNoBuffer(cId) /\ ~IsDeliveryCallbackRunning(cId) /\
              (\A b \in BufferIds : ~IsReturnedBuffer(cId, b)) =>
                  ENABLED <<ReleaseCallHandle(cId)>>_vars)
  <2>1. TypeOK /\ L0!IsTerminalCall(cId) /\ ~IsHandleReleased(cId) /\
            ~IsRuntimeOfCallDestroyed(cId) /\ HostOwnsNoPayload(cId) /\
            HostHoldsNoBuffer(cId) /\ ~IsDeliveryCallbackRunning(cId) /\
            (\A b \in BufferIds : ~IsReturnedBuffer(cId, b)) =>
                ENABLED <<ReleaseCallHandle(cId)>>_vars
    BY ReleaseEnabled, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](TypeOK /\ <<ReleaseCallHandle(cId)>>_vars =>
              (IsHandleReleased(cId))')
  <2>1. TypeOK /\ <<ReleaseCallHandle(cId)>>_vars =>
            (IsHandleReleased(cId))'
    BY SMT DEF ReleaseCallHandle, IsHandleReleased, TypeOK, L0!TypeOK,
        vars, l0_vars, L0!vars, ffi_vars
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ LentCountMatchesBufferStates /\
              (\A b \in BufferIds : ~IsLentBuffer(cId, b)) =>
                  HostHoldsNoBuffer(cId))
  <2>1. TypeOK /\ LentCountMatchesBufferStates /\
            (\A b \in BufferIds : ~IsLentBuffer(cId, b)) =>
                HostHoldsNoBuffer(cId)
    BY NoLentMeansNoneHeld, Zenon
  <2>2. QED BY <2>1, PTL
<1>4. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             [](L0!IsTerminalCall(cId)),
             WF_vars(ReleaseCallHandle(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(HostConsumesEvent(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             BufferFairnessFor(cId)
      PROVE  <>(IsHandleReleased(cId) \/ IsRuntimeOfCallDestroyed(cId))
  <2>0. /\ []TypeOK
        /\ [](TypeOK /\ FfiCallInv)
    BY <1>4, PTL
  <2>1. <>[]HostOwnsNoPayload(cId)
    BY <1>4, <2>0, TerminalPayloadsDrain, PTL
  <2>2. <>[]~IsDeliveryCallbackRunning(cId)
    BY <1>4, <2>0, TerminalDeliveryQuiets, PTL
  <2>3. <>[](\A b \in BufferIds : ~IsLentBuffer(cId, b))
    BY <1>4, <2>0, TerminalCallBuffersSettle, PTL
  <2>4. <>[]HostHoldsNoBuffer(cId)
    BY <1>3, <1>4, <2>0, <2>3, PTL
  <2>5. \A b \in BufferIds : <>[]~IsReturnedBuffer(cId, b)
    <3>1. ASSUME NEW b \in BufferIds
          PROVE  <>[]~IsReturnedBuffer(cId, b)
      <4>1. /\ WF_vars(HostReturnsBuffer(cId, b))
            /\ WF_vars(FreeReturnedBuffer(cId, b))
        BY <1>4, IsaT(600) DEF BufferFairnessFor
      <4>2. QED
        BY <1>4, <4>1, TerminalBufferUnreturned, PTL
    <3>2. QED BY <3>1
  <2>6. <>[](\A b \in BufferIds : ~IsReturnedBuffer(cId, b))
    BY <2>5, AllBuffersUnreturned
  <2>7. QED
    BY <1>1, <1>2, <1>4, <2>0, <2>1, <2>2, <2>4, <2>6, PTL
<1>5. QED BY <1>4, PTL

\* Quantified weak fairness is invariant, per buffer family: the same
\* three-move as the per-call ones, over the buffer identity space.
THEOREM BoxedBufferFairness ==
    ASSUME NEW cId \in CallIds
    PROVE  BufferFairnessFor(cId) <=> []BufferFairnessFor(cId)
<1>1. []BufferFairnessFor(cId)
          <=> \A b \in BufferIds :
                  [](/\ WF_vars(HostReturnsBuffer(cId, b))
                     /\ WF_vars(FreeReturnedBuffer(cId, b)))
    BY IsaT(600) DEF BufferFairnessFor
<1>2. ASSUME NEW b \in BufferIds
      PROVE [](/\ WF_vars(HostReturnsBuffer(cId, b))
               /\ WF_vars(FreeReturnedBuffer(cId, b)))
            <=> /\ WF_vars(HostReturnsBuffer(cId, b))
                /\ WF_vars(FreeReturnedBuffer(cId, b))
    BY PTL
<1>3. QED BY <1>1, <1>2, IsaT(600) DEF BufferFairnessFor

\* The same result as a leads-to.  The standing terminal hypothesis becomes a
\* single terminal state here, where the absorbing fact can still be boxed:
\* one level up, under Spec, necessitation is no longer available.
THEOREM TerminalCallReclaimsFrom ==
    ASSUME NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ WF_vars(ReleaseCallHandle(cId))
           /\ WF_vars(DeliveryCallbackReturns(cId))
           /\ WF_vars(HostConsumesEvent(cId))
           /\ WF_vars(EmitWriteDone(cId))
           /\ WF_vars(WriteDoneReturns(cId))
           /\ BufferFairnessFor(cId)
           => []([](L0!IsTerminalCall(cId)) =>
                     <>(IsHandleReleased(cId) \/
                            IsRuntimeOfCallDestroyed(cId)))
<1>2. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             WF_vars(ReleaseCallHandle(cId)),
             WF_vars(DeliveryCallbackReturns(cId)),
             WF_vars(HostConsumesEvent(cId)),
             WF_vars(EmitWriteDone(cId)),
             WF_vars(WriteDoneReturns(cId)),
             BufferFairnessFor(cId)
      PROVE  []([](L0!IsTerminalCall(cId)) =>
                    <>(IsHandleReleased(cId) \/
                           IsRuntimeOfCallDestroyed(cId)))
\* Every hypothesis in the form the temporal backend can read at any suffix,
\* then the theorem itself: tlapm necessitates a cited theorem, and only a
\* cited theorem - routing the same fact through a proof step leaves it
\* marked non-[] and unusable under the goal's box.
  <2>16. /\ [][](TypeOK /\ FfiCallInv /\ BufferStateInv)
         /\ [][]LentCountMatchesBufferStates
         /\ [][][Next]_vars
    BY <1>2, PTL
  <2>17. /\ []WF_vars(ReleaseCallHandle(cId))
         /\ []WF_vars(DeliveryCallbackReturns(cId))
         /\ []WF_vars(HostConsumesEvent(cId))
         /\ []WF_vars(EmitWriteDone(cId))
         /\ []WF_vars(WriteDoneReturns(cId))
    BY <1>2, PTL
  <2>18. []BufferFairnessFor(cId)
    BY <1>2, BoxedBufferFairness, PTL
  <2>2. QED
    BY <2>16, <2>17, <2>18, TerminalCallReclaims, PTL
<1>3. QED BY <1>2, PTL

THEOREM CallEventuallyReclaimedHolds == Spec => CallEventuallyReclaimed
<1>1. ASSUME Spec, NEW cId \in CallIds
      PROVE  L0!IsTerminalCall(cId) ~>
                 (IsHandleReleased(cId) \/ IsRuntimeOfCallDestroyed(cId))
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
        /\ []LentCountMatchesBufferStates
    BY <2>1, IndInvBufferParts, PTL
  <2>3. /\ WF_vars(ReleaseCallHandle(cId))
        /\ WF_vars(DeliveryCallbackReturns(cId))
        /\ WF_vars(HostConsumesEvent(cId))
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
        /\ BufferFairnessFor(cId)
    BY <2>0, FairnessAtCall, FairnessEverywhere, IsaT(600) DEF BufferFairnessFor
\* Terminal being absorbing turns the standing hypothesis into a single
\* terminal state, and the boxed form above carries it to every suffix.
  <2>4. [](TypeOK /\ [Next]_vars /\ L0!IsTerminalCall(cId) =>
               (L0!IsTerminalCall(cId))')
    BY TerminalCallStaysTerminal, PTL
  <2>45. []([](L0!IsTerminalCall(cId)) =>
                <>(IsHandleReleased(cId) \/ IsRuntimeOfCallDestroyed(cId)))
    BY <2>0, <2>2, <2>3, TerminalCallReclaimsFrom, PTL
  <2>5. QED
    BY <2>0, <2>2, <2>4, <2>45, PTL
<1>2. QED BY <1>1, Zenon DEF CallEventuallyReclaimed

\* The per-call half of the unload condition, behind a name, so the temporal
\* backend never has to look inside it.
CallQuietFor(rtId, cId) ==
    call_channel[cId] \in L0!ChannelsOf(rtId) =>
        /\ HostOwnsNoPayload(cId)
        /\ HostHoldsNoBuffer(cId)


\* The boxed form, so a consumer under a temporal hypothesis can use it: a
\* parameterised lemma is not lifted into a box at the point of use, and the
\* lifting has to happen here, where no temporal hypothesis is in scope.
LEMMA ReleasedRuntimeStaysReleasedBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](TypeOK /\ [Next]_vars /\ IsReleasedRuntime(rtId) =>
                  (IsReleasedRuntime(rtId))')
<1>1. TypeOK /\ [Next]_vars /\ IsReleasedRuntime(rtId) =>
          (IsReleasedRuntime(rtId))'
    BY ReleasedRuntimeStaysReleased
<1>2. QED BY <1>1, PTL

\* A call on a released runtime's channel is not active: the channel is
\* closed, and an active call sits on an open or closing one.
LEMMA ReleasedRuntimeCallNotActive ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds,
           TypeOK, L0!StrongInv, IsReleasedRuntime(rtId),
           call_channel[cId] \in L0!ChannelsOf(rtId)
    PROVE  ~L0!IsActiveCall(cId)
<1>1. SUFFICES ASSUME L0!IsActiveCall(cId)
               PROVE  FALSE
    OBVIOUS
\* The invariants quantify over the used sets, so membership comes first.
<1>15. cId \in L0!UsedCalls
    BY <1>1, SMT
    DEF L0!UsedCalls, L0!IsUnusedCall, L0!IsActiveCall,
        L0!ActiveCallStates, TypeOK, L0!TypeOK, L0!CallStates
<1>2. call_channel[cId] \in L0!ActiveChannels
    BY <1>1, <1>15, Zenon
    DEF L0!StrongInv, L0!StructuralInv, L0!CallLifecycleInv
<1>25. call_channel[cId] \in L0!UsedChannels
    BY <1>2, SMT
    DEF L0!ActiveChannels, L0!ActiveChannelStates, L0!UsedChannels,
        TypeOK, L0!TypeOK, L0!ChannelStates
<1>3. channel_runtime[call_channel[cId]] = rtId
    BY Zenon DEF L0!ChannelsOf
<1>4. channel_state[call_channel[cId]] \in L0!ActiveChannelStates
    BY <1>2, Zenon DEF L0!ActiveChannels
<1>5. QED
    BY <1>25, <1>3, <1>4, SMT
    DEF L0!StrongInv, L0!StructuralInv, L0!ChannelLifecycleInv,
        L0!ActiveChannelStates, IsReleasedRuntime, TypeOK, L0!TypeOK

\* An unused call owes nothing, which is the other half of the case split.
LEMMA UnusedCallIsQuiet ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds,
           FfiCallInv, L0!IsUnusedCall(cId)
    PROVE  CallQuietFor(rtId, cId)
<1>1. QED
    BY Zenon
    DEF CallQuietFor, FfiCallInv, UnusedCallsAreFfiClean

\* One call of a released runtime settles.  The case split is on whether the
\* call is ever terminal: if it is, the terminal drains do the work; if it is
\* not, it stays unused, and an unused call owes nothing to begin with.  No
\* argument about the call joining or leaving the runtime is needed, because
\* the predicate is an implication and is vacuous off the runtime.
THEOREM ReleasedCallQuiets ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []L0!StrongInv
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ [](IsReleasedRuntime(rtId))
           /\ []WF_vars(HostConsumesEvent(cId))
           /\ []BufferFairnessFor(cId)
           => <>[]CallQuietFor(rtId, cId)
<1>1. [](TypeOK /\ L0!StrongInv /\ IsReleasedRuntime(rtId) /\
              call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  ~L0!IsActiveCall(cId))
  <2>1. TypeOK /\ L0!StrongInv /\ IsReleasedRuntime(rtId) /\
            call_channel[cId] \in L0!ChannelsOf(rtId) =>
                ~L0!IsActiveCall(cId)
    BY ReleasedRuntimeCallNotActive, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](FfiCallInv /\ L0!IsUnusedCall(cId) => CallQuietFor(rtId, cId))
  <2>1. FfiCallInv /\ L0!IsUnusedCall(cId) => CallQuietFor(rtId, cId)
    BY UnusedCallIsQuiet, Zenon
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ ~L0!IsActiveCall(cId) =>
              L0!IsUnusedCall(cId) \/ L0!IsTerminalCall(cId))
  <2>1. TypeOK /\ ~L0!IsActiveCall(cId) =>
            L0!IsUnusedCall(cId) \/ L0!IsTerminalCall(cId)
    BY SMT
    DEF L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall,
        L0!ActiveCallStates, L0!CallStates, TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, PTL
<1>4. [](TypeOK /\ LentCountMatchesBufferStates /\
              (\A b \in BufferIds : ~IsLentBuffer(cId, b)) =>
                  HostHoldsNoBuffer(cId))
  <2>1. TypeOK /\ LentCountMatchesBufferStates /\
            (\A b \in BufferIds : ~IsLentBuffer(cId, b)) =>
                HostHoldsNoBuffer(cId)
    BY NoLentMeansNoneHeld, Zenon
  <2>2. QED BY <2>1, PTL
\* Named, the predicate is an atom, so the two ways it holds have to be
\* handed over explicitly: off the runtime it is vacuous.
<1>45. [](~(call_channel[cId] \in L0!ChannelsOf(rtId)) =>
               CallQuietFor(rtId, cId))
  <2>1. ~(call_channel[cId] \in L0!ChannelsOf(rtId)) =>
            CallQuietFor(rtId, cId)
    BY Zenon DEF CallQuietFor
  <2>2. QED BY <2>1, PTL
<1>5. [](HostOwnsNoPayload(cId) /\ HostHoldsNoBuffer(cId) =>
              CallQuietFor(rtId, cId))
  <2>1. HostOwnsNoPayload(cId) /\ HostHoldsNoBuffer(cId) =>
            CallQuietFor(rtId, cId)
    BY Zenon DEF CallQuietFor
  <2>2. QED BY <2>1, PTL
<1>6. [](TypeOK /\ [Next]_vars /\ L0!IsTerminalCall(cId) =>
              (L0!IsTerminalCall(cId))')
    BY TerminalCallStaysTerminal, PTL
<1>7. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []L0!StrongInv,
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             [](IsReleasedRuntime(rtId)),
             []WF_vars(HostConsumesEvent(cId)),
             []BufferFairnessFor(cId)
      PROVE  <>[]CallQuietFor(rtId, cId)
  <2>0. /\ []TypeOK
        /\ [](TypeOK /\ FfiCallInv)
    BY <1>7, PTL
\* Never terminal means always unused, and an unused call is already quiet.
  <2>1. CASE []~L0!IsTerminalCall(cId)
    <3>1. []CallQuietFor(rtId, cId)
      BY <1>1, <1>2, <1>3, <1>45, <1>7, <2>0, <2>1, PTL
    <3>2. QED BY <3>1, PTL
\* Terminal once means terminal for good, and then the two drains apply.
  <2>2. CASE <>L0!IsTerminalCall(cId)
    <3>1. <>[]L0!IsTerminalCall(cId)
      BY <1>6, <1>7, <2>0, <2>2, PTL
    <3>2. <>[]HostOwnsNoPayload(cId)
      BY <1>7, <2>0, <3>1, TerminalPayloadsDrain, PTL
    <3>3. <>[](\A b \in BufferIds : ~IsLentBuffer(cId, b))
      BY <1>7, <2>0, <3>1, TerminalCallBuffersSettle, PTL
    <3>4. <>[]HostHoldsNoBuffer(cId)
      BY <1>4, <1>7, <2>0, <3>3, PTL
    <3>5. QED BY <1>5, <3>2, <3>4, PTL
  <2>3. QED BY <2>1, <2>2, PTL
<1>8. QED BY <1>7

\* The two host obligations a call has to drain, bundled per call and then
\* over the calls, for the same reason as the buffer bundle.
CallDrainFairness ==
    \A cId \in CallIds :
        /\ WF_vars(HostConsumesEvent(cId))
        /\ BufferFairnessFor(cId)

LEMMA BoxedCallDrainFairness ==
    CallDrainFairness <=> []CallDrainFairness
<1>1. []CallDrainFairness
          <=> \A cId \in CallIds :
                  [](/\ WF_vars(HostConsumesEvent(cId))
                     /\ BufferFairnessFor(cId))
    BY IsaT(600) DEF CallDrainFairness
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](/\ WF_vars(HostConsumesEvent(cId))
               /\ BufferFairnessFor(cId))
            <=> /\ WF_vars(HostConsumesEvent(cId))
                /\ BufferFairnessFor(cId)
  <2>1. BufferFairnessFor(cId) <=> []BufferFairnessFor(cId)
    BY BoxedBufferFairness
  <2>2. QED BY <2>1, PTL
<1>3. QED BY <1>1, <1>2, IsaT(600) DEF CallDrainFairness

\* Reclaimable is exactly the conjunction of the per-call predicates.
LEMMA ReclaimableIsAllQuiet ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  NoHostDebt(rtId) <=>
               (\A cId \in CallIds : CallQuietFor(rtId, cId))
<1>1. QED
    BY Zenon DEF NoHostDebt, CallQuietFor

\* The finite lift over the calls, free of behavioural hypotheses.
THEOREM AllCallsQuietFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  (\A cId \in CallIds : <>[]CallQuietFor(rtId, cId))
               => <>[](\A cId \in CallIds : CallQuietFor(rtId, cId))
<1>0. USE FiniteCallIds DEF FiniteCallIds
<1> DEFINE G(c) == CallQuietFor(rtId, c)
           K(c) == <>[]G(c)
           I(T) == (\A cId \in T : K(cId)) => <>[](\A cId \in T : G(cId))
<1>1. I({})
  <2>1. \A cId \in {} : G(cId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET CallIds, NEW x \in CallIds \ T
       PROVE <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
                 <>[](\A cId \in T \cup {x} : G(cId))
  <2>1. (\A cId \in T : G(cId)) /\ G(x) =>
            (\A cId \in T \cup {x} : G(cId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET CallIds, IsFiniteSet(T), I(T),
             NEW x \in CallIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A cId \in T \cup {x} : K(cId)) => (\A cId \in T : K(cId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
            <>[](\A cId \in T \cup {x} : G(cId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(CallIds)
    BY <1>1, <1>2, FS_Induction, IsaMT("blast", 600)
<1>4. QED BY <1>3, Zenon DEF I

\* A released runtime becomes reclaimable, under a standing released and
\* unfailed hypothesis.  The boxed conclusion is what the leads-to needs:
\* tlapm necessitates a cited theorem, never a proof step.
THEOREM ReleasedRuntimeReclaims ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ []IndInv
           /\ []CallDrainFairness
           => []( ([](IsReleasedRuntime(rtId)) /\ [](L0!NotFailed))
                      => <>[]NoHostDebt(rtId) )
<1>1. [](NoHostDebt(rtId) <=>
              (\A cId \in CallIds : CallQuietFor(rtId, cId)))
  <2>1. NoHostDebt(rtId) <=>
            (\A cId \in CallIds : CallQuietFor(rtId, cId))
    BY ReclaimableIsAllQuiet, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](IndInv /\ L0!NotFailed => L0!StrongInv)
  <2>1. IndInv /\ L0!NotFailed => L0!StrongInv
    BY UmbrellaAt, Zenon DEF StrongInv
  <2>2. QED BY <2>1, PTL
<1>3. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             []IndInv,
             []CallDrainFairness,
             [](IsReleasedRuntime(rtId)),
             [](L0!NotFailed)
      PROVE  <>[]NoHostDebt(rtId)
  <2>0. []L0!StrongInv
    BY <1>2, <1>3, PTL
  <2>1. CallDrainFairness
    BY <1>3, PTL
  <2>2. ASSUME NEW cId \in CallIds
        PROVE  <>[]CallQuietFor(rtId, cId)
    <3>1. /\ WF_vars(HostConsumesEvent(cId))
          /\ BufferFairnessFor(cId)
      BY <2>1, IsaT(600) DEF CallDrainFairness
    <3>2. []WF_vars(HostConsumesEvent(cId))
      BY <3>1, PTL
    <3>3. []BufferFairnessFor(cId)
      BY <3>1, BoxedBufferFairness, PTL
    <3>4. QED
      BY <1>3, <2>0, <3>2, <3>3, ReleasedCallQuiets
  <2>3. <>[](\A cId \in CallIds : CallQuietFor(rtId, cId))
    BY <2>2, AllCallsQuietFor
  <2>4. QED BY <1>1, <1>3, <2>3, PTL
<1>4. QED
    BY <1>3, BoxedCallDrainFairness, PTL

\* The other runtime-level ledger, per call: what the runtime has given back
\* to itself but not yet released.  Same shape as CallQuietFor - an
\* implication, vacuous off the runtime - for the same reason: the finite lift
\* over the calls has to see one predicate, not a membership and a body.
CallBytesFreeFor(rtId, cId) ==
    call_channel[cId] \in L0!ChannelsOf(rtId) =>
        \A b \in BufferIds : ~IsReturnedBuffer(cId, b)

\* An unused call has given nothing back: its buffers have never left "none".
LEMMA UnusedCallHasNoReturnedBytes ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds,
           BufferStateInv, L0!IsUnusedCall(cId)
    PROVE  CallBytesFreeFor(rtId, cId)
<1>1. QED
    BY Zenon
    DEF CallBytesFreeFor, BufferStateInv, UnusedCallsHaveFreshBuffers,
        IsFreshBuffer, IsReturnedBuffer

\* One call of a released runtime gives its bytes back, by the same case split
\* as ReleasedCallQuiets: terminal once means terminal for good and the buffer
\* drain applies, never terminal means always unused and nothing was ever lent.
THEOREM ReleasedCallBytesFree ==
    ASSUME NEW rtId \in RuntimeIds, NEW cId \in CallIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []L0!StrongInv
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ [](IsReleasedRuntime(rtId))
           /\ []BufferFairnessFor(cId)
           /\ []WF_vars(EmitWriteDone(cId))
           /\ []WF_vars(WriteDoneReturns(cId))
           => <>[]CallBytesFreeFor(rtId, cId)
<1>1. [](TypeOK /\ L0!StrongInv /\ IsReleasedRuntime(rtId) /\
              call_channel[cId] \in L0!ChannelsOf(rtId) =>
                  ~L0!IsActiveCall(cId))
  <2>1. TypeOK /\ L0!StrongInv /\ IsReleasedRuntime(rtId) /\
            call_channel[cId] \in L0!ChannelsOf(rtId) =>
                ~L0!IsActiveCall(cId)
    BY ReleasedRuntimeCallNotActive, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](BufferStateInv /\ L0!IsUnusedCall(cId) =>
              CallBytesFreeFor(rtId, cId))
  <2>1. BufferStateInv /\ L0!IsUnusedCall(cId) =>
            CallBytesFreeFor(rtId, cId)
    BY UnusedCallHasNoReturnedBytes, Zenon
  <2>2. QED BY <2>1, PTL
<1>3. [](TypeOK /\ ~L0!IsActiveCall(cId) =>
              L0!IsUnusedCall(cId) \/ L0!IsTerminalCall(cId))
  <2>1. TypeOK /\ ~L0!IsActiveCall(cId) =>
            L0!IsUnusedCall(cId) \/ L0!IsTerminalCall(cId)
    BY SMT
    DEF L0!IsActiveCall, L0!IsUnusedCall, L0!IsTerminalCall,
        L0!ActiveCallStates, L0!CallStates, TypeOK, L0!TypeOK
  <2>2. QED BY <2>1, PTL
\* Named, the predicate is an atom, so the two ways it holds are handed over
\* explicitly: off the runtime it is vacuous.
<1>45. [](~(call_channel[cId] \in L0!ChannelsOf(rtId)) =>
               CallBytesFreeFor(rtId, cId))
  <2>1. ~(call_channel[cId] \in L0!ChannelsOf(rtId)) =>
            CallBytesFreeFor(rtId, cId)
    BY Zenon DEF CallBytesFreeFor
  <2>2. QED BY <2>1, PTL
<1>5. []((\A b \in BufferIds : ~IsReturnedBuffer(cId, b)) =>
              CallBytesFreeFor(rtId, cId))
  <2>1. (\A b \in BufferIds : ~IsReturnedBuffer(cId, b)) =>
            CallBytesFreeFor(rtId, cId)
    BY Zenon DEF CallBytesFreeFor
  <2>2. QED BY <2>1, PTL
<1>6. [](TypeOK /\ [Next]_vars /\ L0!IsTerminalCall(cId) =>
              (L0!IsTerminalCall(cId))')
    BY TerminalCallStaysTerminal, PTL
<1>7. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []L0!StrongInv,
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             [](IsReleasedRuntime(rtId)),
             []BufferFairnessFor(cId),
             []WF_vars(EmitWriteDone(cId)),
             []WF_vars(WriteDoneReturns(cId))
      PROVE  <>[]CallBytesFreeFor(rtId, cId)
  <2>0. /\ []TypeOK
        /\ []BufferStateInv
    BY <1>7, PTL
  <2>1. CASE []~L0!IsTerminalCall(cId)
    <3>1. []CallBytesFreeFor(rtId, cId)
      BY <1>1, <1>2, <1>3, <1>45, <1>7, <2>0, <2>1, PTL
    <3>2. QED BY <3>1, PTL
  <2>2. CASE <>L0!IsTerminalCall(cId)
    <3>1. <>[]L0!IsTerminalCall(cId)
      BY <1>6, <1>7, <2>0, <2>2, PTL
    <3>2. ASSUME NEW b \in BufferIds
          PROVE  <>[]~IsReturnedBuffer(cId, b)
\* Boxed, because the theorem below is applied at the point the standing
\* terminal hypothesis starts holding, not here.  Each extraction is proved
\* without the temporal context so PTL may necessitate it.
      <4>1. [](/\ WF_vars(HostReturnsBuffer(cId, b))
               /\ WF_vars(FreeReturnedBuffer(cId, b)))
        <5>1. BufferFairnessFor(cId) =>
                  /\ WF_vars(HostReturnsBuffer(cId, b))
                  /\ WF_vars(FreeReturnedBuffer(cId, b))
          BY IsaT(600), PTL DEF BufferFairnessFor
        <5>2. QED BY <1>7, <5>1, PTL
\* The drain, instantiated on this buffer as a step of its own.  The temporal
\* backend does not instantiate a cited theorem, and it is the step below that
\* has to read the drain at the point the call turns terminal.
      <4>2. /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
            /\ []LentCountMatchesBufferStates
            /\ [][Next]_vars
            /\ [](L0!IsTerminalCall(cId))
            /\ WF_vars(HostReturnsBuffer(cId, b))
            /\ WF_vars(FreeReturnedBuffer(cId, b))
            /\ WF_vars(EmitWriteDone(cId))
            /\ WF_vars(WriteDoneReturns(cId))
            => <>[]~IsReturnedBuffer(cId, b)
        BY TerminalBufferUnreturned, IsaT(600), PTL
      <4>3. QED
        BY <1>7, <3>1, <4>1, <4>2, PTL
    <3>3. <>[](\A b \in BufferIds : ~IsReturnedBuffer(cId, b))
      BY <3>2, AllBuffersUnreturned
    <3>4. QED BY <1>5, <3>3, PTL
  <2>3. QED BY <2>1, <2>2, PTL
<1>8. QED BY <1>7

\* The finite lift over the calls, free of behavioural hypotheses.  Same
\* induction as AllCallsQuietFor, on the other predicate.
THEOREM AllCallsBytesFreeFor ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  (\A cId \in CallIds : <>[]CallBytesFreeFor(rtId, cId))
               => <>[](\A cId \in CallIds : CallBytesFreeFor(rtId, cId))
<1>0. USE FiniteCallIds DEF FiniteCallIds
<1> DEFINE G(c) == CallBytesFreeFor(rtId, c)
           K(c) == <>[]G(c)
           I(T) == (\A cId \in T : K(cId)) => <>[](\A cId \in T : G(cId))
<1>1. I({})
  <2>1. \A cId \in {} : G(cId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET CallIds, NEW x \in CallIds \ T
       PROVE <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
                 <>[](\A cId \in T \cup {x} : G(cId))
  <2>1. (\A cId \in T : G(cId)) /\ G(x) =>
            (\A cId \in T \cup {x} : G(cId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET CallIds, IsFiniteSet(T), I(T),
             NEW x \in CallIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A cId \in T \cup {x} : K(cId)) => (\A cId \in T : K(cId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
            <>[](\A cId \in T \cup {x} : G(cId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(CallIds)
    BY <1>1, <1>2, FS_Induction, IsaMT("blast", 600)
<1>4. QED BY <1>3, Zenon DEF I

\* And the runtime-level predicate is exactly that conjunction.
LEMMA BytesFreeIsAllCallsFree ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  RuntimeHoldsNoReturnedBytes(rtId) <=>
               (\A cId \in CallIds : CallBytesFreeFor(rtId, cId))
<1>1. QED
    BY Zenon DEF RuntimeHoldsNoReturnedBytes, CallBytesFreeFor

\* The host obligations a call has to drain before the second event may go
\* out.  CallDrainFairness is the payload and buffer half; the two write-done
\* conjuncts are what carries a committed buffer from "returned" to "freed",
\* and the release signal waits on that too.
CallSettleFairness ==
    \A cId \in CallIds :
        /\ WF_vars(HostConsumesEvent(cId))
        /\ BufferFairnessFor(cId)
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))

LEMMA SettleFairnessIncludesDrain ==
    CallSettleFairness => CallDrainFairness
<1>1. QED
    BY IsaT(600), PTL DEF CallSettleFairness, CallDrainFairness

LEMMA BoxedCallSettleFairness ==
    CallSettleFairness <=> []CallSettleFairness
<1>1. []CallSettleFairness
          <=> \A cId \in CallIds :
                  [](/\ WF_vars(HostConsumesEvent(cId))
                     /\ BufferFairnessFor(cId)
                     /\ WF_vars(EmitWriteDone(cId))
                     /\ WF_vars(WriteDoneReturns(cId)))
    BY IsaT(600) DEF CallSettleFairness
<1>2. ASSUME NEW cId \in CallIds
      PROVE [](/\ WF_vars(HostConsumesEvent(cId))
               /\ BufferFairnessFor(cId)
               /\ WF_vars(EmitWriteDone(cId))
               /\ WF_vars(WriteDoneReturns(cId)))
            <=> /\ WF_vars(HostConsumesEvent(cId))
                /\ BufferFairnessFor(cId)
                /\ WF_vars(EmitWriteDone(cId))
                /\ WF_vars(WriteDoneReturns(cId))
  <2>1. BufferFairnessFor(cId) <=> []BufferFairnessFor(cId)
    BY BoxedBufferFairness
  <2>2. QED BY <2>1, PTL
<1>3. QED BY <1>1, <1>2, IsaT(600) DEF CallSettleFairness

\* Both ledgers empty, stably.  This is the state the second event is allowed
\* to go out from, and the reason it is stated as one theorem is that the two
\* halves have to hold in the same suffix, not one after the other.
THEOREM ReleasedRuntimeSettles ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ []IndInv
           /\ []CallSettleFairness
           => []( ([](IsReleasedRuntime(rtId)) /\ [](L0!NotFailed))
                      => <>[](NoHostDebt(rtId) /\
                                  RuntimeHoldsNoReturnedBytes(rtId)) )
<1>1. [](RuntimeHoldsNoReturnedBytes(rtId) <=>
              (\A cId \in CallIds : CallBytesFreeFor(rtId, cId)))
  <2>1. RuntimeHoldsNoReturnedBytes(rtId) <=>
            (\A cId \in CallIds : CallBytesFreeFor(rtId, cId))
    BY BytesFreeIsAllCallsFree, Zenon
  <2>2. QED BY <2>1, PTL
<1>2. [](IndInv /\ L0!NotFailed => L0!StrongInv)
  <2>1. IndInv /\ L0!NotFailed => L0!StrongInv
    BY UmbrellaAt, Zenon DEF StrongInv
  <2>2. QED BY <2>1, PTL
<1>3. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             []IndInv,
             []CallSettleFairness,
             [](IsReleasedRuntime(rtId)),
             [](L0!NotFailed)
      PROVE  <>[](NoHostDebt(rtId) /\
                      RuntimeHoldsNoReturnedBytes(rtId))
  <2>0. []L0!StrongInv
    BY <1>2, <1>3, PTL
  <2>01. []CallDrainFairness
    BY <1>3, SettleFairnessIncludesDrain, PTL
  <2>1. <>[]NoHostDebt(rtId)
    BY <1>3, <2>01, ReleasedRuntimeReclaims, PTL
  <2>2. CallSettleFairness
    BY <1>3, PTL
  <2>3. ASSUME NEW cId \in CallIds
        PROVE  <>[]CallBytesFreeFor(rtId, cId)
    <3>1. /\ BufferFairnessFor(cId)
          /\ WF_vars(EmitWriteDone(cId))
          /\ WF_vars(WriteDoneReturns(cId))
      BY <2>2, IsaT(600) DEF CallSettleFairness
    <3>2. []BufferFairnessFor(cId)
      BY <3>1, BoxedBufferFairness, PTL
    <3>3. /\ []WF_vars(EmitWriteDone(cId))
          /\ []WF_vars(WriteDoneReturns(cId))
      BY <3>1, PTL
    <3>4. QED
      BY <1>3, <2>0, <3>2, <3>3, ReleasedCallBytesFree
  <2>4. <>[](\A cId \in CallIds : CallBytesFreeFor(rtId, cId))
    BY <2>3, AllCallsBytesFreeFor
  <2>5. QED BY <1>1, <2>1, <2>4, PTL
<1>4. QED
    BY <1>3, BoxedCallSettleFairness, PTL

(***************************************************************************)
(* THE RELEASE SIGNAL, LIVE                                                *)
(* Quiescence is the level-1 status the host waits for before it unloads.   *)
(* Four facts on top of an empty ledger: the runtime is released, neither   *)
(* callback is on the stack, and the second event has gone out if the first *)
(* one said it was owed.                                                   *)
(***************************************************************************)



\* And once it is out, nothing puts that callback back on the stack: the only
\* step that would is guarded on the flag it just raised.
LEMMA ReleaseRunningOffForGood ==
    ASSUME NEW rtId \in RuntimeIds, TypeOK, [Next]_vars,
           IsResourcesReleasedEmitted(rtId),
           ~IsResourcesReleasedCallbackRunning(rtId)
    PROVE  (~IsResourcesReleasedCallbackRunning(rtId))'
<1>1. CASE \E rt \in RuntimeIds : EmitResourcesReleased(rt)
    BY <1>1, SMT DEF EmitResourcesReleased, TypeOK, L0!TypeOK
<1>2. CASE \E rt \in RuntimeIds : ResourcesReleasedCallbackReturns(rt)
    BY <1>2, SMT DEF ResourcesReleasedCallbackReturns, TypeOK, L0!TypeOK
<1>3. CASE UNCHANGED <<resources_released_emitted,
                       resources_released_callback_running>>
    BY <1>3, SMT
<1>4. QED
    BY <1>1, <1>2, <1>3, OnlyReleaseStepsWriteReleaseFlags, Zenon

\* The second event's own enabling condition, which is its guard: the first
\* event out and its tag set, this one not yet out, no callback on the stack,
\* and both ledgers empty.
LEMMA EmitResourcesReleasedEnabled ==
    ASSUME NEW rtId \in RuntimeIds, TypeOK,
           IsShutdownEventEmitted(rtId), SecondEventOwed(rtId),
           ~IsResourcesReleasedEmitted(rtId),
           ~IsShutdownCallbackRunning(rtId),
           NoHostDebt(rtId), RuntimeHoldsNoReturnedBytes(rtId)
    PROVE  ENABLED <<EmitResourcesReleased(rtId)>>_vars
<1>1. QED
    BY ExpandENABLED, SMT
    DEF EmitResourcesReleased, TypeOK, L0!TypeOK,
        l0_vars, L0!vars, vars, ffi_vars

LEMMA EmitResourcesReleasedSets ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  TypeOK /\ <<EmitResourcesReleased(rtId)>>_vars =>
               (IsResourcesReleasedEmitted(rtId))'
<1>1. QED
    BY SMT DEF EmitResourcesReleased, vars, l0_vars, L0!vars, ffi_vars,
        TypeOK, L0!TypeOK

\* Quiescence out of its six parts.  Named, the status is an atom, so the
\* temporal step cannot see the conjunction it is made of.
LEMMA QuiescentFromParts ==
    ASSUME NEW rtId \in RuntimeIds,
           IsReleasedRuntime(rtId),
           ~IsShutdownCallbackRunning(rtId),
           ~IsResourcesReleasedCallbackRunning(rtId),
           SecondEventOwed(rtId) =>
               IsResourcesReleasedEmitted(rtId),
           NoHostDebt(rtId),
           RuntimeHoldsNoReturnedBytes(rtId)
    PROVE  IsRuntimeQuiescent(rtId)
<1>1. QED
    BY Zenon DEF IsRuntimeQuiescent

\* A released runtime becomes quiescent.  The tag splits the argument: it is
\* frozen once the first event is out, so it is either set for the whole
\* suffix or clear for the whole suffix.  Clear means nothing was owed and
\* the invariant already forbids the callback; set means the second event's
\* step is enabled as soon as both ledgers are empty, and its callback then
\* returns and cannot be re-entered.
THEOREM ReleasedRuntimeQuiesces ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
           /\ []LentCountMatchesBufferStates
           /\ [][Next]_vars
           /\ []IndInv
           /\ []CallSettleFairness
           /\ []WF_vars(EmitResourcesReleased(rtId))
           /\ []WF_vars(ResourcesReleasedCallbackReturns(rtId))
           => []( ([](IsReleasedRuntime(rtId)) /\ [](L0!NotFailed))
                      => <>IsRuntimeQuiescent(rtId) )
<1>1. [](IndInv /\ L0!NotFailed => StrongInv)
  <2>1. IndInv /\ L0!NotFailed => StrongInv
    BY UmbrellaAt, Zenon
  <2>2. QED BY <2>1, PTL
\* What a released runtime already satisfies: the first event is out and its
\* callback has returned - that is what RELEASED means at level 1.
<1>2. [](StrongInv /\ IsReleasedRuntime(rtId) =>
              IsShutdownEventEmitted(rtId) /\
                  ~IsShutdownCallbackRunning(rtId))
  <2>1. StrongInv /\ IsReleasedRuntime(rtId) =>
            IsShutdownEventEmitted(rtId) /\
                ~IsShutdownCallbackRunning(rtId)
    BY Zenon DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
  <2>2. QED BY <2>1, PTL
\* And what the release invariant says about the second one.
<1>3. [](StrongInv /\ IsResourcesReleasedCallbackRunning(rtId) =>
              SecondEventOwed(rtId))
  <2>1. StrongInv /\ IsResourcesReleasedCallbackRunning(rtId) =>
            SecondEventOwed(rtId)
    BY Zenon DEF StrongInv, ShutdownSignalInv, ReleaseSignalInv
  <2>2. QED BY <2>1, PTL
\* Two implications rather than the equality they add up to: an equality
\* between two booleans is one opaque atom to the temporal backend.
<1>4. /\ [](TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
                SecondEventOwed(rtId) =>
                    (SecondEventOwed(rtId))')
      /\ [](TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
                ~SecondEventOwed(rtId) =>
                    (~SecondEventOwed(rtId))')
  <2>1. TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
            SecondEventOwed(rtId) =>
                (SecondEventOwed(rtId))'
    BY EmittedRuntimeTagFrozen, Zenon
  <2>2. TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
            ~SecondEventOwed(rtId) =>
                (~SecondEventOwed(rtId))'
    BY EmittedRuntimeTagFrozen, Zenon
  <2>3. QED BY <2>1, <2>2, PTL
<1>5. /\ [](TypeOK /\ IsShutdownEventEmitted(rtId) /\
                SecondEventOwed(rtId) /\
                ~IsResourcesReleasedEmitted(rtId) /\
                ~IsShutdownCallbackRunning(rtId) /\
                NoHostDebt(rtId) /\
                RuntimeHoldsNoReturnedBytes(rtId) =>
                    ENABLED <<EmitResourcesReleased(rtId)>>_vars)
      /\ [](TypeOK /\ <<EmitResourcesReleased(rtId)>>_vars =>
                (IsResourcesReleasedEmitted(rtId))')
      /\ [](TypeOK /\ [Next]_vars /\ IsResourcesReleasedEmitted(rtId) =>
                (IsResourcesReleasedEmitted(rtId))')
  <2>1. TypeOK /\ IsShutdownEventEmitted(rtId) /\
            SecondEventOwed(rtId) /\
            ~IsResourcesReleasedEmitted(rtId) /\
            ~IsShutdownCallbackRunning(rtId) /\
            NoHostDebt(rtId) /\
            RuntimeHoldsNoReturnedBytes(rtId) =>
                ENABLED <<EmitResourcesReleased(rtId)>>_vars
    BY EmitResourcesReleasedEnabled, Zenon
  <2>2. TypeOK /\ <<EmitResourcesReleased(rtId)>>_vars =>
            (IsResourcesReleasedEmitted(rtId))'
    BY EmitResourcesReleasedSets, Zenon
  <2>3. TypeOK /\ [Next]_vars /\ IsResourcesReleasedEmitted(rtId) =>
            (IsResourcesReleasedEmitted(rtId))'
    BY ResourcesEmittedStable, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>6. /\ [](TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) =>
                ENABLED <<ResourcesReleasedCallbackReturns(rtId)>>_vars)
      /\ [](TypeOK /\
                <<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
                    (~IsResourcesReleasedCallbackRunning(rtId))')
      /\ [](TypeOK /\ [Next]_vars /\ IsResourcesReleasedEmitted(rtId) /\
                ~IsResourcesReleasedCallbackRunning(rtId) =>
                    (~IsResourcesReleasedCallbackRunning(rtId))')
  <2>1. TypeOK /\ IsResourcesReleasedCallbackRunning(rtId) =>
            ENABLED <<ResourcesReleasedCallbackReturns(rtId)>>_vars
    BY RRCREnabled, Zenon
  <2>2. TypeOK /\ <<ResourcesReleasedCallbackReturns(rtId)>>_vars =>
            (~IsResourcesReleasedCallbackRunning(rtId))'
    BY RRCRClears, Zenon
  <2>3. TypeOK /\ [Next]_vars /\ IsResourcesReleasedEmitted(rtId) /\
            ~IsResourcesReleasedCallbackRunning(rtId) =>
                (~IsResourcesReleasedCallbackRunning(rtId))'
    BY ReleaseRunningOffForGood, Zenon
  <2>4. QED BY <2>1, <2>2, <2>3, PTL
<1>7. [](IsReleasedRuntime(rtId) /\ ~IsShutdownCallbackRunning(rtId) /\
              ~IsResourcesReleasedCallbackRunning(rtId) /\
              (SecondEventOwed(rtId) =>
                   IsResourcesReleasedEmitted(rtId)) /\
              NoHostDebt(rtId) /\
              RuntimeHoldsNoReturnedBytes(rtId) =>
                  IsRuntimeQuiescent(rtId))
  <2>1. IsReleasedRuntime(rtId) /\ ~IsShutdownCallbackRunning(rtId) /\
            ~IsResourcesReleasedCallbackRunning(rtId) /\
            (SecondEventOwed(rtId) =>
                 IsResourcesReleasedEmitted(rtId)) /\
            NoHostDebt(rtId) /\
            RuntimeHoldsNoReturnedBytes(rtId) =>
                IsRuntimeQuiescent(rtId)
    BY QuiescentFromParts, Zenon
  <2>2. QED BY <2>1, PTL
<1>8. ASSUME [](TypeOK /\ FfiCallInv /\ BufferStateInv),
             []LentCountMatchesBufferStates,
             [][Next]_vars,
             []IndInv,
             []CallSettleFairness,
             []WF_vars(EmitResourcesReleased(rtId)),
             []WF_vars(ResourcesReleasedCallbackReturns(rtId)),
             [](IsReleasedRuntime(rtId)),
             [](L0!NotFailed)
      PROVE  <>IsRuntimeQuiescent(rtId)
  <2>0. /\ []TypeOK
        /\ []StrongInv
    BY <1>1, <1>8, PTL
  <2>1. /\ []IsShutdownEventEmitted(rtId)
        /\ []~IsShutdownCallbackRunning(rtId)
    BY <1>2, <1>8, <2>0, PTL
  <2>2. <>[](NoHostDebt(rtId) /\
                 RuntimeHoldsNoReturnedBytes(rtId))
    BY <1>8, ReleasedRuntimeSettles, PTL
\* The tag never moves again, so it is one way for the whole suffix.
  <2>3. []SecondEventOwed(rtId) \/
            []~SecondEventOwed(rtId)
    BY <1>4, <1>8, <2>0, <2>1, PTL
\* Nothing owed: the invariant forbids the callback outright, and the
\* implication is vacuous.
  <2>4. CASE []~SecondEventOwed(rtId)
    <3>1. []~IsResourcesReleasedCallbackRunning(rtId)
      BY <1>3, <1>8, <2>0, <2>4, PTL
    <3>2. QED
      BY <1>7, <1>8, <2>1, <2>2, <2>4, <3>1, PTL
\* Something was owed: the step is enabled once the ledgers are empty, so it
\* fires, and then its callback returns for good.
  <2>5. CASE []SecondEventOwed(rtId)
    <3>1. <>[]IsResourcesReleasedEmitted(rtId)
      BY <1>5, <1>8, <2>0, <2>1, <2>2, <2>5, PTL
    <3>2. <>[]~IsResourcesReleasedCallbackRunning(rtId)
      BY <1>6, <1>8, <2>0, <3>1, PTL
    <3>3. QED
      BY <1>7, <1>8, <2>1, <2>2, <3>1, <3>2, PTL
  <2>6. QED BY <2>3, <2>4, <2>5
<1>9. QED
    BY <1>8, BoxedCallSettleFairness, PTL

THEOREM RuntimeEventuallyQuiescentHolds ==
    Spec => RuntimeEventuallyQuiescent
<1>1. ASSUME Spec, NEW rtId \in RuntimeIds
      PROVE  IsReleasedRuntime(rtId) ~>
                 (IsRuntimeQuiescent(rtId) \/ ~L0!NotFailed)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
        /\ []LentCountMatchesBufferStates
    BY <2>1, IndInvBufferParts, PTL
  <2>3. CallSettleFairness
    BY <2>0, FairnessEverywhere, IsaT(600) DEF CallSettleFairness, BufferFairnessFor
  <2>35. []CallSettleFairness
    BY <2>3, BoxedCallSettleFairness
  <2>36. /\ []WF_vars(EmitResourcesReleased(rtId))
         /\ []WF_vars(ResourcesReleasedCallbackReturns(rtId))
    <3>1. /\ WF_vars(EmitResourcesReleased(rtId))
          /\ WF_vars(ResourcesReleasedCallbackReturns(rtId))
      BY <2>0, FairnessAtRuntime, IsaT(600)
    <3>2. QED BY <3>1, PTL
\* Released is absorbing and failure is sticky, so a single released state
\* gives the standing hypothesis, and a failure that ever happens settles
\* the right disjunct for good.
  <2>4. [](TypeOK /\ [Next]_vars /\ IsReleasedRuntime(rtId) =>
               (IsReleasedRuntime(rtId))')
    BY ReleasedRuntimeStaysReleasedBoxed
  <2>5. [](TypeOK /\ ~L0!NotFailed /\ [Next]_vars => (~L0!NotFailed)')
    BY UnfailedSticky, PTL
  <2>6. QED
    BY <2>0, <2>1, <2>2, <2>35, <2>36, <2>4, <2>5,
       ReleasedRuntimeQuiesces, PTL
<1>2. QED BY <1>1, Zenon DEF RuntimeEventuallyQuiescent

\* The four boxed facts the promise below needs.  Each is hoisted to a lemma of
\* its own because a step proved inside a context that holds Spec cannot be
\* necessitated: the temporal backend reads a boxed goal only from a boxed
\* statement, and a top-level lemma is where a state fact can still become one.
LEMMA EmittedTagFrozenBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
                  SecondEventOwed(rtId) =>
                      (IsShutdownEventEmitted(rtId))' /\
                          (SecondEventOwed(rtId))')
<1>1. TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
          SecondEventOwed(rtId) =>
              (IsShutdownEventEmitted(rtId))' /\ (SecondEventOwed(rtId))'
    BY EmittedRuntimeTagFrozen
<1>2. QED BY <1>1, PTL

LEMMA EmittedRuntimeIsStoppingOrReleasedBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](IndInv /\ L0!NotFailed /\ IsShutdownEventEmitted(rtId) =>
                  IsStoppingRuntime(rtId) \/ IsReleasedRuntime(rtId))
<1>1. IndInv /\ L0!NotFailed /\ IsShutdownEventEmitted(rtId) =>
          IsStoppingRuntime(rtId) \/ IsReleasedRuntime(rtId)
    BY UmbrellaAt, Zenon
    DEF StrongInv, ShutdownSignalInv, ShutdownSignalCore
<1>2. QED BY <1>1, PTL

\* The level-0 shutdown vocabulary against the level-1 one, so the temporal
\* step sees a single pair of names.
LEMMA ShutdownVocabularyBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  /\ [](L0!ShutdownWaiting(rtId) <=> IsStoppingRuntime(rtId))
           /\ [](L0!ShutdownSettled(rtId) =>
                     IsReleasedRuntime(rtId) \/ ~L0!NotFailed)
<1>1. L0!ShutdownWaiting(rtId) <=> IsStoppingRuntime(rtId)
    BY Zenon DEF L0!ShutdownWaiting
<1>2. L0!ShutdownSettled(rtId) => IsReleasedRuntime(rtId) \/ ~L0!NotFailed
    BY SMT DEF L0!ShutdownSettled, L0!NotFailed
<1>3. QED BY <1>1, <1>2, PTL

LEMMA QuiescentWithOwedIsEmittedBoxed ==
    ASSUME NEW rtId \in RuntimeIds
    PROVE  [](IsRuntimeQuiescent(rtId) /\ SecondEventOwed(rtId) =>
                  IsResourcesReleasedEmitted(rtId))
<1>1. IsRuntimeQuiescent(rtId) /\ SecondEventOwed(rtId) =>
          IsResourcesReleasedEmitted(rtId)
    BY Zenon DEF IsRuntimeQuiescent
<1>2. QED BY <1>1, PTL

\* The release tag's own promise, named so a level-2 binding can refine it
\* rather than re-derive it: a host that was told to give memory back is told
\* when the runtime is done with what came back.
THEOREM ResourcesReleasedEventuallyHolds ==
    Spec => ResourcesReleasedEventually
<1>1. SUFFICES ASSUME Spec, NEW rtId \in RuntimeIds
      PROVE  (IsShutdownEventEmitted(rtId) /\ SecondEventOwed(rtId)) ~>
                 (IsResourcesReleasedEmitted(rtId) \/ ~L0!NotFailed)
    BY Zenon DEF ResourcesReleasedEventually
<1>2. /\ Init
      /\ [][Next]_vars
      /\ Fairness
    BY <1>1 DEF Spec
<1>3. []IndInv
    BY <1>2, BehaviorEstablishesIndInv, PTL
<1>4. []TypeOK
    BY <1>3, IndInvParts, PTL
<1>45. [][Next]_vars
    BY <1>2, PTL
\* The antecedent is stable: the event is a latch and the tag it carried never
\* moves again, so what the promise is owed to does not expire under it.
<1>5. [](TypeOK /\ [Next]_vars /\ IsShutdownEventEmitted(rtId) /\
              SecondEventOwed(rtId) =>
                  (IsShutdownEventEmitted(rtId))' /\ (SecondEventOwed(rtId))')
    BY EmittedTagFrozenBoxed
\* An emitted runtime is stopping or released; there is nowhere else to be.
<1>6. [](IndInv /\ L0!NotFailed /\ IsShutdownEventEmitted(rtId) =>
              IsStoppingRuntime(rtId) \/ IsReleasedRuntime(rtId))
    BY EmittedRuntimeIsStoppingOrReleasedBoxed
\* The level-0 shutdown settles, inherited through the refinement.
<1>7. IsStoppingRuntime(rtId) ~> (IsReleasedRuntime(rtId) \/ ~L0!NotFailed)
  <2>1. L0!LivenessProperties
    BY <1>1, InheritedLivenessTheorem
  <2>2. L0!ShutdownWaiting(rtId) ~> L0!ShutdownSettled(rtId)
    BY <2>1, IsaT(600) DEF L0!LivenessProperties, L0!EventualShutdown
  <2>3. QED BY <2>2, ShutdownVocabularyBoxed, PTL
\* And from released, the runtime quiesces - which with the tag set is exactly
\* the second event having gone out.
<1>8. IsReleasedRuntime(rtId) ~> (IsRuntimeQuiescent(rtId) \/ ~L0!NotFailed)
  <2>1. RuntimeEventuallyQuiescent
    BY <1>1, RuntimeEventuallyQuiescentHolds
  <2>2. QED BY <2>1, IsaT(600) DEF RuntimeEventuallyQuiescent
<1>9. [](IsRuntimeQuiescent(rtId) /\ SecondEventOwed(rtId) =>
              IsResourcesReleasedEmitted(rtId))
    BY QuiescentWithOwedIsEmittedBoxed
\* Failure is sticky, so the escape disjunct settles for good once taken.
<1>10. [](TypeOK /\ ~L0!NotFailed /\ [Next]_vars => (~L0!NotFailed)')
    BY UnfailedSticky, PTL
<1>11. QED
    BY <1>3, <1>4, <1>45, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, PTL

(***************************************************************************)
(* THE EMISSION BUDGET - exhausting it is temporary                        *)
(***************************************************************************)

\* A buffer that is out reaches freed.  The lent half is BufferEventuallyFreed
\* itself.  The returned half is that theorem's second rung, rebuilt here from
\* the same top-level lemmas rather than reached into: a step of a theorem is
\* not citable, and rebuilding four rungs costs less than hoisting them out of
\* a proof that works.
\* A buffer that is out reaches freed.  The lent half is BufferEventuallyFreed
\* itself.  The returned half is that theorem's second rung, rebuilt here from
\* the same top-level lemmas rather than reached into: a step of a theorem is
\* not citable, and rebuilding four rungs costs less than hoisting them out of
\* a proof that works.
LEMMA OutstandingBufferEventuallyFreed ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  Spec => (BufferOutstanding(cId, b) ~> IsFreedBuffer(cId, b))
\* The rungs are boxed here, where Spec is not in scope.  A fact proved under
\* Spec cannot be necessitated, so the boxing has to happen outside the step
\* that assumes it.
<1>0. /\ [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                 CarriesNoUnacquittedSend(cId, b) =>
                     ENABLED <<FreeReturnedBuffer(cId, b)>>_vars)
      /\ [](TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
                 (IsFreedBuffer(cId, b))')
      /\ [](TypeOK /\ IsReturnedBuffer(cId, b) /\
                 CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
                 ~<<FreeReturnedBuffer(cId, b)>>_vars =>
                     (IsReturnedBuffer(cId, b) /\
                          CarriesNoUnacquittedSend(cId, b))')
      /\ [](TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b)))
      /\ [](TypeOK /\ [Next]_vars /\
                 (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b)) =>
                     (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b))')
  <2>1. TypeOK /\ IsReturnedBuffer(cId, b) /\
          CarriesNoUnacquittedSend(cId, b) =>
              ENABLED <<FreeReturnedBuffer(cId, b)>>_vars
    BY FreeBufferEnabled, Zenon
  <2>2. TypeOK /\ <<FreeReturnedBuffer(cId, b)>>_vars =>
          (IsFreedBuffer(cId, b))'
    BY SMTT(120) DEF FreeReturnedBuffer, IsFreedBuffer, TypeOK, L0!TypeOK
  <2>3. TypeOK /\ IsReturnedBuffer(cId, b) /\
          CarriesNoUnacquittedSend(cId, b) /\ [Next]_vars /\
          ~<<FreeReturnedBuffer(cId, b)>>_vars =>
              (IsReturnedBuffer(cId, b) /\
                   CarriesNoUnacquittedSend(cId, b))'
    <3>1. SUFFICES ASSUME TypeOK, IsReturnedBuffer(cId, b),
                          CarriesNoUnacquittedSend(cId, b), [Next]_vars,
                          ~<<FreeReturnedBuffer(cId, b)>>_vars
                   PROVE  (IsReturnedBuffer(cId, b) /\
                               CarriesNoUnacquittedSend(cId, b))'
      OBVIOUS
    <3>2. ~IsLentBuffer(cId, b)
      BY <3>1, SMT DEF IsReturnedBuffer, IsLentBuffer, TypeOK, L0!TypeOK
    <3>3. (IsReturnedBuffer(cId, b))'
      BY <3>1, BufferStateFrame, Zenon
    <3>4. QED
      BY <3>1, <3>2, <3>3, AcquittalPersistsOffLent
  <2>4. TypeOK => (IsFreedBuffer(cId, b) => ~IsReturnedBuffer(cId, b))
    BY SMT DEF IsFreedBuffer, IsReturnedBuffer, TypeOK, L0!TypeOK
  <2>5. TypeOK /\ [Next]_vars /\
          (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b)) =>
              (IsReturnedBuffer(cId, b) \/ IsFreedBuffer(cId, b))'
    BY ReturnedBufferStaysOrIsFreed, FreedStaysFreed, Zenon
  <2>6. QED BY <2>1, <2>2, <2>3, <2>4, <2>5, PTL
<1>1. ASSUME Spec
      PROVE  BufferOutstanding(cId, b) ~> IsFreedBuffer(cId, b)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []IndInv
    BY <2>0, BehaviorEstablishesIndInv, PTL
  <2>2. []TypeOK
    BY <2>1, IndInvParts, PTL
  <2>3. /\ WF_vars(HostReturnsBuffer(cId, b))
        /\ WF_vars(FreeReturnedBuffer(cId, b))
    BY <2>0, FairnessAtBuffer, IsaT(600)
  <2>4. /\ [](TypeOK /\ FfiCallInv /\ BufferStateInv)
        /\ WF_vars(EmitWriteDone(cId))
        /\ WF_vars(WriteDoneReturns(cId))
    <3>1. [](TypeOK /\ FfiCallInv /\ BufferStateInv)
      BY <2>1, IndInvBufferParts, PTL
    <3>2. QED BY <2>0, <3>1, FairnessAtCall, IsaT(600)
  <2>5. IsReturnedBuffer(cId, b) ~>
            (CarriesNoUnacquittedSend(cId, b) \/ IsFreedBuffer(cId, b))
    BY <2>0, <2>4, ReturnedBufferAcquits, PTL
  <2>6. (IsReturnedBuffer(cId, b) /\ CarriesNoUnacquittedSend(cId, b))
            ~> IsFreedBuffer(cId, b)
    BY <1>0, <2>0, <2>2, <2>3, PTL
  <2>7. IsReturnedBuffer(cId, b) ~> IsFreedBuffer(cId, b)
    BY <1>0, <2>0, <2>2, <2>5, <2>6, PTL
  <2>8. IsLentBuffer(cId, b) ~> IsFreedBuffer(cId, b)
    BY <1>1, BufferEventuallyFreedHolds, ZenonT(120)
       DEF BufferEventuallyFreed
  <2>9. QED
    BY <2>7, <2>8, PTL DEF BufferOutstanding
<1>2. QED BY <1>1

\* Each buffer leaves the outstanding pair for good.  Either it is never in it,
\* or it reaches freed and stays there: freed is absorbing, and freed is neither
\* lent nor returned.  The argument rests on a buffer identity being used once,
\* which is what makes this a settling rather than an induction on a measure.
\* Under buffer reuse it would be the lend guard that carried it - no lend can
\* fire while the budget is exhausted - and that is the mechanism the ABI
\* actually relies on.
LEMMA BufferSettlesOutstanding ==
    ASSUME NEW cId \in CallIds, NEW b \in BufferIds
    PROVE  Spec => <>[]~BufferOutstanding(cId, b)
<1>0. /\ [](TypeOK /\ [Next]_vars /\ IsFreedBuffer(cId, b) =>
                 (IsFreedBuffer(cId, b))')
      /\ [](TypeOK /\ IsFreedBuffer(cId, b) =>
                 ~BufferOutstanding(cId, b))
  <2>1. TypeOK /\ [Next]_vars /\ IsFreedBuffer(cId, b) =>
            (IsFreedBuffer(cId, b))'
    BY FreedStaysFreed, Zenon
  <2>2. TypeOK /\ IsFreedBuffer(cId, b) => ~BufferOutstanding(cId, b)
    BY SMT DEF IsFreedBuffer, IsLentBuffer, IsReturnedBuffer,
        BufferOutstanding, TypeOK, L0!TypeOK
  <2>3. QED BY <2>1, <2>2, PTL
<1>1. ASSUME Spec PROVE <>[]~BufferOutstanding(cId, b)
  <2>0. /\ Init
        /\ [][Next]_vars
        /\ Fairness
    BY <1>1 DEF Spec
  <2>1. []TypeOK
    <3>1. []IndInv
      BY <2>0, BehaviorEstablishesIndInv, PTL
    <3>2. QED BY <3>1, IndInvParts, PTL
  <2>2. BufferOutstanding(cId, b) ~> IsFreedBuffer(cId, b)
    BY <1>1, OutstandingBufferEventuallyFreed
  <2>3. CASE []~BufferOutstanding(cId, b)
    BY <2>3, PTL
  <2>4. CASE <>BufferOutstanding(cId, b)
    <3>1. <>IsFreedBuffer(cId, b)
      BY <2>2, <2>4, PTL
    <3>2. <>[]IsFreedBuffer(cId, b)
      BY <1>0, <2>0, <2>1, <3>1, PTL
    <3>3. QED BY <1>0, <2>1, <3>2, PTL
  <2>5. QED BY <2>3, <2>4, PTL
<1>2. QED BY <1>1

\* The finite lift over the buffers of one call, and then over the calls: the
\* same induction as AllBuffersUnreturned and AllCallsBytesFreeFor, on the
\* outstanding pair instead of the returned state.
THEOREM AllBuffersSettleFor ==
    ASSUME NEW cId \in CallIds
    PROVE  (\A b \in BufferIds : <>[]~BufferOutstanding(cId, b))
               => <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b))
<1>0. USE BufferIdsAreAFiniteNonemptySet DEF BufferIdsAreAFiniteNonemptySet
<1> DEFINE G(b) == ~BufferOutstanding(cId, b)
           K(b) == <>[]G(b)
           I(T) == (\A b \in T : K(b)) => <>[](\A b \in T : G(b))
<1>1. I({})
  <2>1. \A b \in {} : G(b)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET BufferIds, NEW x \in BufferIds \ T
       PROVE <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
                 <>[](\A b \in T \cup {x} : G(b))
  <2>1. (\A b \in T : G(b)) /\ G(x) => (\A b \in T \cup {x} : G(b))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET BufferIds, IsFiniteSet(T), I(T),
             NEW x \in BufferIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A b \in T \cup {x} : K(b)) => (\A b \in T : K(b)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A b \in T : G(b)) /\ <>[]G(x) =>
            <>[](\A b \in T \cup {x} : G(b))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(BufferIds)
    BY <1>1, <1>2, FS_Induction, IsaM("blast")
<1>4. QED BY <1>3 DEF I

THEOREM AllCallsBuffersSettle ==
    (\A cId \in CallIds :
         <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b)))
        => <>[](\A cId \in CallIds :
                    \A b \in BufferIds : ~BufferOutstanding(cId, b))
<1>0. USE FiniteCallIds DEF FiniteCallIds
<1> DEFINE G(c) == \A b \in BufferIds : ~BufferOutstanding(c, b)
           K(c) == <>[]G(c)
           I(T) == (\A cId \in T : K(cId)) => <>[](\A cId \in T : G(cId))
<1>1. I({})
  <2>1. \A cId \in {} : G(cId)
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>1a. ASSUME NEW T \in SUBSET CallIds, NEW x \in CallIds \ T
       PROVE <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
                 <>[](\A cId \in T \cup {x} : G(cId))
  <2>1. (\A cId \in T : G(cId)) /\ G(x) => (\A cId \in T \cup {x} : G(cId))
    <3> HIDE DEF G
    <3>1. QED OBVIOUS
  <2>2. QED BY <2>1, PTL
<1>2. ASSUME NEW T \in SUBSET CallIds, IsFiniteSet(T), I(T),
             NEW x \in CallIds \ T
      PROVE I(T \cup {x})
  <2>1. (\A cId \in T \cup {x} : K(cId)) => (\A cId \in T : K(cId)) /\ K(x)
    <3> HIDE DEF K
    <3>1. QED OBVIOUS
  <2>2. <>[](\A cId \in T : G(cId)) /\ <>[]G(x) =>
            <>[](\A cId \in T \cup {x} : G(cId))
    BY <1>1a
  <2>3. QED BY <1>2, <2>1, <2>2, PTL
<1> HIDE DEF I
<1>3. I(CallIds)
    BY <1>1, <1>2, FS_Induction, IsaM("blast")
<1>4. QED BY <1>3 DEF I

\* A request the ABI would consider, refused for want of room, eventually has
\* room.  Every buffer out is eventually freed - the chain above - so the
\* outstanding set empties, the accounting makes the counter zero, and at zero
\* any lendable size fits.
THEOREM BudgetEventuallyAdmitsHolds ==
    Spec => BudgetEventuallyAdmits
\* Boxed where Spec is not in scope, and with no hypothesis in scope either:
\* a fact proved under one cannot be necessitated.  The guard is handled
\* outside the temporal reasoning, the property carrying it there.
<1>0. ASSUME NEW len \in Nat
      PROVE  /\ [](SafetyInvariant => MemoryAccountingExact)
             /\ []((\A cId \in CallIds :
                        \A b \in BufferIds : ~BufferOutstanding(cId, b))
                       => OutstandingPairs = {})
             /\ [](OutstandingPairs = {} /\ MemoryAccountingExact
                       => memory_used = 0)
             /\ [](memory_used = 0 /\ IsLendable(len)
                       => IsRequestAdmissible(len))
  <2>2. SafetyInvariant => MemoryAccountingExact
    BY Zenon DEF SafetyInvariant
  <2>3. (\A cId \in CallIds :
             \A b \in BufferIds : ~BufferOutstanding(cId, b))
            => OutstandingPairs = {}
    BY Zenon DEF OutstandingPairs
  <2>4. OutstandingPairs = {} /\ MemoryAccountingExact => memory_used = 0
    BY SumFunctionOnSetEmpty, Zenon
    DEF MemoryAccountingExact, BytesOutstanding
\* At zero the request is its own witness: a charge equal to the length covers
\* it and fits.  Implications throughout - PTL boxes the conjunct above, and a
\* sequent here would carry its hypotheses nowhere.
  <2>5. memory_used = 0 /\ IsLendable(len) => IsRequestAdmissible(len)
    <3>1. IsLendable(len) => len \in Sizes
      BY SMT DEF IsLendable, Sizes
    <3>2. memory_used = 0 /\ IsLendable(len)
              => CoversRequest(len, len) /\ IsMemoryAvailable(len)
      BY SMT DEF CoversRequest, IsMemoryAvailable, IsLendable
    <3>3. QED BY <3>1, <3>2, Zenon DEF IsRequestAdmissible
  <2>6. QED BY <2>2, <2>3, <2>4, <2>5, PTL
<1>1. ASSUME Spec, NEW len \in Nat, IsLendable(len)
      PROVE  ~IsRequestAdmissible(len) ~> IsRequestAdmissible(len)
  <2>1. []SafetyInvariant
    BY <1>1, SafetyTheorem, PTL
  <2>3. ASSUME NEW cId \in CallIds
        PROVE  <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b))
    <3>0. ASSUME NEW b \in BufferIds
          PROVE  <>[]~BufferOutstanding(cId, b)
      <4>1. Spec => <>[]~BufferOutstanding(cId, b)
        BY BufferSettlesOutstanding
      <4>2. QED BY <1>1, <4>1, PTL
    <3>1. \A b \in BufferIds : <>[]~BufferOutstanding(cId, b)
      BY <3>0
    <3>15. (\A b \in BufferIds : <>[]~BufferOutstanding(cId, b))
               => <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b))
      BY AllBuffersSettleFor, IsaT(600)
    <3>2. QED BY <3>1, <3>15, PTL
  <2>35. (\A cId \in CallIds :
              <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b)))
             => <>[](\A cId \in CallIds :
                         \A b \in BufferIds : ~BufferOutstanding(cId, b))
    BY AllCallsBuffersSettle, IsaT(600)
  <2>36. \A cId \in CallIds :
             <>[](\A b \in BufferIds : ~BufferOutstanding(cId, b))
    BY <2>3
  <2>4. <>[](\A cId \in CallIds :
                 \A b \in BufferIds : ~BufferOutstanding(cId, b))
    BY <2>35, <2>36, PTL
  <2>5. <>[](memory_used = 0)
    BY <1>0, <2>1, <2>4, PTL
\* The guard is a hypothesis of this step, so it holds now and, being rigid,
\* at every instant the boxed implication is read.  Handing PTL the guarded
\* implication and the eventual zero is enough.
\* The instance at this len as its own step: the cascade instantiates it here,
\* and PTL then has a boxed fact with nothing left to instantiate.
  <2>55. [](memory_used = 0 /\ IsLendable(len)
                 => IsRequestAdmissible(len))
    BY <1>0
  <2>555. [](IsLendable(len))
    <3>1. IsLendable(len)
      BY <1>1
    <3>2. QED BY <3>1, Zenon DEF IsLendable
  <2>6. <>[]IsRequestAdmissible(len)
    BY <2>5, <2>55, <2>555, PTL
  <2>7. QED BY <2>6, PTL
<1>2. QED BY <1>1, Zenon DEF BudgetEventuallyAdmits

\* What the host is actually promised: the request that was refused is granted,
\* or the call leaves the state where lending means anything, or the runtime
\* fails - the escape every inherited liveness carries.  The route is the
\* level-0 termination promise: eligibility keeps the call active, an active
\* call reaches its status or the runtime fails, and a delivered status ends
\* eligibility through the terminal-status equivalence.
THEOREM BudgetRefusalEventuallyLendsHolds ==
    Spec => BudgetRefusalEventuallyLends
\* The definitional facts, boxed where no hypothesis is in scope: a fact
\* proved under Spec cannot be necessitated.
<1>0. ASSUME NEW cId \in CallIds, NEW msg \in Messages
      PROVE  /\ [](IsBudgetRefused(cId, msg) /\ CanStillLend(cId)
                        /\ L0!NotFailed
                       => L0!TerminalWaiting(cId) /\ L0!NotFailed)
             /\ [](L0!SafetyInvariant /\ L0!NotFailed
                        /\ L0!TerminalReached(cId)
                       => ~CanStillLend(cId))
  <2>1. IsBudgetRefused(cId, msg) /\ CanStillLend(cId) /\ L0!NotFailed
            => L0!TerminalWaiting(cId) /\ L0!NotFailed
    BY Zenon DEF CanStillLend, ContemplatesLend, L0!TerminalWaiting
\* Eligibility keeps the call active, so it is a used call; the equivalence
\* then reads the delivered status as terminal, and terminal is not active.
  <2>2. L0!SafetyInvariant /\ L0!NotFailed /\ L0!TerminalReached(cId)
            => ~CanStillLend(cId)
    BY SMTT(120)
    DEF L0!SafetyInvariant, L0!SafetyCore, L0!TerminalStatusEquivalence,
        L0!TerminalReached, L0!UsedCalls, L0!IsUnusedCall,
        L0!IsTerminalCall, L0!IsActiveCall, L0!ActiveCallStates,
        CanStillLend, ContemplatesLend
  <2>3. QED BY <2>1, <2>2, PTL
<1>1. ASSUME Spec, NEW cId \in CallIds, NEW msg \in Messages
      PROVE  (IsBudgetRefused(cId, msg) /\ CanStillLend(cId) /\ L0!NotFailed)
                 ~> (IsLendGranted(cId, msg) \/ ~CanStillLend(cId)
                         \/ ~L0!NotFailed)
  <2>1. []L0!SafetyInvariant
    BY <1>1, InheritedSafety
  <2>2. (L0!TerminalWaiting(cId) /\ L0!NotFailed) ~>
            (L0!TerminalReached(cId) \/ ~L0!NotFailed)
    <3>1. L0!LivenessProperties
      BY <1>1, InheritedLivenessTheorem
    <3>2. QED BY <3>1, IsaT(600) DEF L0!LivenessProperties, L0!EventualTerminal
  <2>3. QED BY <1>0, <2>1, <2>2, PTL
<1>2. QED BY <1>1, Zenon DEF BudgetRefusalEventuallyLends

THEOREM LivenessTheorem == Spec => LivenessProperties
<1>1. QED
    BY CancellationCompletesHolds, SendsEventuallyAcquittedHolds,
       PayloadsEventuallyConsumedHolds, ShutdownEventEmittedHolds,
       DeliveryCallbacksReturnHolds, WriteDoneCallbacksReturnHolds,
       ShutdownCallbacksReturnHolds,
       ResourcesReleasedCallbacksReturnHolds, BufferEventuallyFreedHolds,
       CallEventuallyReclaimedHolds, RuntimeEventuallyQuiescentHolds,
       ResourcesReleasedEventuallyHolds, BudgetEventuallyAdmitsHolds,
       BudgetRefusalEventuallyLendsHolds,
       ZenonT(120) DEF LivenessProperties

=============================================================================

