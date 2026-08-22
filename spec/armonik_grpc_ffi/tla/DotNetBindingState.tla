-------------------------- MODULE DotNetBindingState --------------------------
(***************************************************************************)
(* The level-2 state: the shared level-0/level-1 state plus the managed    *)
(* variables of the .NET binding.                                          *)
(*                                                                         *)
(* Declarations only, following FfiGrpcState: DotNetBinding defines the    *)
(* level-2 machinery over it and reaches the level-1 machinery through an  *)
(* INSTANCE of FfiGrpcTheorems.                                            *)
(*                                                                         *)
(* No new constants.  The managed side adds discipline, not capacity: the  *)
(* pipelining depths, the byte ceiling and the identity spaces are the     *)
(* ABI's and arrived at level 1.  The retry cadence is deliberately not a  *)
(* constant - no property of the model reads it, and the one liveness      *)
(* about the retry loop is conditional on cancellation, not on time.      *)
(*                                                                         *)
(* The ring is deliberately NOT here.  Both of its indexes are level-1     *)
(* state read through level-2 names: the trampoline publishes inside the   *)
(* delivery callback, so the published prefix IS events_delivered and the  *)
(* head is its length; a release is ak_event_consumed, so the tail IS      *)
(* payloads_consumed_by_host.  One counter standing for the outstanding    *)
(* set is also what makes release order structural: advancing a counter    *)
(* can only release the oldest.  What level 2 adds about the ring is who   *)
(* consumes it, which is consumer_phase below.                             *)
(***************************************************************************)

EXTENDS FfiGrpcState

VARIABLES
    (***********************************************************************)
    (* Roots and identity plumbing.  What must hold of callback, call_ctx  *)
    (* and runtime_ctx - the arguments level 1 does not model - is that    *)
    (* they resolve to live objects whenever a callback runs.              *)
    (***********************************************************************)
    call_token_published,   \* [CallIds -> BOOLEAN]: GCHandle.Alloc done
    call_root_live,         \* [CallIds -> BOOLEAN]: the call's GC root
    runtime_root_live,      \* BOOLEAN: the invoker's GC root

    (***********************************************************************)
    (* The ring's consumer, in phases: the header prologue owning slot 0,  *)
    (* the application while the call is live, the drain after dispose,    *)
    (* done when the call is disposed.  One value per call is the          *)
    (* single-consumer discipline made structural.                         *)
    (***********************************************************************)
    consumer_phase,         \* [CallIds -> {"prologue", "application",
                            \*              "drain", "done"}]

    (***********************************************************************)
    (* Completion asynchrony.  A callback completes a TCS and returns; the *)
    (* continuation runs as its own later step.  Modelling the two as      *)
    (* separate actions is what makes ContinuationsAsync structural: no    *)
    (* behaviour exists in which user code runs inside the callback.       *)
    (***********************************************************************)
    pending_continuations,  \* [CallIds -> Nat]: completions signaled and
                            \* not yet run

    (***********************************************************************)
    (* The retry protocol.  One flag per call - retrying or not - carries  *)
    (* BudgetCancellationStopsRetry and MessageTooLargeIsNotRetried with   *)
    (* no counter anywhere, and RetryingCallHoldsNoBuffer is the guard on  *)
    (* entering the waiting state.                                         *)
    (***********************************************************************)
    retry_state,            \* [CallIds -> {"idle", "awaiting_budget"}]

    (***********************************************************************)
    (* Dispose machines.  The call's drives the drain; the runtime's       *)
    (* orders every teardown downcall before ak_runtime_destroy.           *)
    (***********************************************************************)
    call_dispose_state,     \* [CallIds -> {"active", "draining",
                            \*              "disposed"}]
    runtime_dispose_state   \* {"active", "destroying", "destroyed"}

managed_vars == <<call_token_published, call_root_live, runtime_root_live,
                  consumer_phase, pending_continuations,
                  retry_state, call_dispose_state, runtime_dispose_state>>

===============================================================================
