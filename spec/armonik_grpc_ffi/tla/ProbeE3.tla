--------------------------- MODULE ProbeE3 ---------------------------
EXTENDS DotNetBinding_defs, TLAPS

USE DEF ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
        ReaderVars, WriterVars

PayloadOwed(cId) == L1!HostOwnsSomePayload(cId)

LEMMA CancelledParseHoldsUntilItReturns ==
    ASSUME NEW cId \in CallIds, ManagedIndInv, PayloadOwed(cId),
           reader_state[cId] = "parsing_cancelled", [Next]_vars
    PROVE  \/ (reader_state[cId] = "parsing_cancelled")'
           \/ <<FinishCancelledParse(cId)>>_vars
<1>0. CASE UNCHANGED vars
    BY <1>0, SMT DEF  PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>1. CASE Passthrough
    BY <1>1, SMT DEF  Passthrough, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>2. CASE FreeRuntimeRoot
    BY <1>2, SMT DEF  FreeRuntimeRoot, PayloadOwed,
       L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
       RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
       ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
       L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
       ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
       ReaderVars, WriterVars
<1>3. CASE \E rtId \in RuntimeIds, chId \in ChannelIds :
             CreateRuntime(rtId, chId)
    BY <1>3, SMT DEF  CreateRuntime, L1!RuntimeCreate,
       L1!L0!RuntimeCreate, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>4. CASE \E chId \in ChannelIds :
             \/ AcquireLease(chId)
             \/ CreateChannel(chId)
             \/ RejectChannelCreation(chId)
             \/ BeginDisposeChannel(chId)
             \/ FinishDisposeChannel(chId)
             \/ ResolveChannelDispose(chId)
    BY <1>4, SMT DEF  AcquireLease, BeginDisposeChannel, CreateChannel,
       FinishDisposeChannel, RejectChannelCreation, ResolveChannelDispose,
       ChannelDisposeMayResolve, ChannelSettled, IsLastRelease,
       L1!ChannelCreate, L1!L0!ChannelCreate, L1!RuntimeFail,
       L1!L0!RuntimeFail, L1!ChannelStartClosing,
       L1!L0!ChannelStartClosing, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>5. CASE \E rtId \in RuntimeIds :
             \/ BeginRuntimeShutdown(rtId)
             \/ FinishDisposeRuntime(rtId)
             \/ ShutdownReturns(rtId)
             \/ ResourcesReleasedReturns(rtId)
    BY <1>5, SMT DEF  BeginRuntimeShutdown, FinishDisposeRuntime,
       ResourcesReleasedReturns, ShutdownReturns, L1!RuntimeBeginShutdown,
       L1!L0!ChannelsOf, L1!L0!RuntimeBeginShutdown, L1!RuntimeDestroy,
       L1!ResourcesReleasedCallbackReturns, L1!ShutdownCallbackReturns,
       PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
       RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
       ManagedTypeOK, ManagedMachineInv, ReaderInv,
       ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
       L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>6. CASE \E cId \in CallIds :
             \/ BeginMoveNext(cId)
             \/ BeginParseEvent(cId)
             \/ FinishConsumePayload(cId)
             \/ CancelWaiter(cId)
             \/ RequestReadCancellation(cId)
             \/ CancelWaitingRead(cId)
             \/ CancelParsingRead(cId)
             \/ FinishCancelledParse(cId)
             \/ HandoffToDrain(cId)
             \/ ConsumeHeader(cId)
             \/ BeginDisposeCall(cId)
             \/ DisposeCallForChannel(cId)
             \/ DrainRelease(cId)
             \/ FinishDisposeCall(cId)
             \/ SettleCall(cId)
             \/ CancelWriterWait(cId)
             \/ WriteDoneCompletes(cId)
             \/ CloseWriter(cId)
             \/ OnEventReturns(cId)
             \/ TerminalCallbackReturns(cId)
  <2>0. SUFFICES ASSUME NEW c2 \in CallIds,
                           BeginMoveNext(c2)
                        \/ BeginParseEvent(c2)
                        \/ FinishConsumePayload(c2)
                        \/ CancelWaiter(c2)
                        \/ RequestReadCancellation(c2)
                        \/ CancelWaitingRead(c2)
                        \/ CancelParsingRead(c2)
                        \/ FinishCancelledParse(c2)
                        \/ HandoffToDrain(c2)
                        \/ ConsumeHeader(c2)
                        \/ BeginDisposeCall(c2)
                        \/ DisposeCallForChannel(c2)
                        \/ DrainRelease(c2)
                        \/ FinishDisposeCall(c2)
                        \/ SettleCall(c2)
                        \/ CancelWriterWait(c2)
                        \/ WriteDoneCompletes(c2)
                        \/ CloseWriter(c2)
                        \/ OnEventReturns(c2)
                        \/ TerminalCallbackReturns(c2)
                 PROVE  \/ (reader_state[cId] = "parsing_cancelled")'
                        \/ <<FinishCancelledParse(cId)>>_vars
      BY <1>6
  <2>1. CASE BeginMoveNext(c2)
          BY <2>0, <2>1, SMT DEF  BeginMoveNext, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>2. CASE BeginParseEvent(c2)
          BY <2>0, <2>2, SMT DEF  BeginParseEvent, RingOccupancy, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>3. CASE FinishConsumePayload(c2)
          BY <2>0, <2>3, SMT DEF  FinishConsumePayload, ConsumingTerminal,
           ReadCancellationSettled, L1!HostConsumesEvent, L1!L0!HasStatus,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>4. CASE CancelWaiter(c2)
          BY <2>0, <2>4, SMT DEF  CancelWaiter, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>5. CASE RequestReadCancellation(c2)
          BY <2>0, <2>5, SMT DEF  RequestReadCancellation, ReadInFlight,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
           ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>6. CASE CancelWaitingRead(c2)
          BY <2>0, <2>6, SMT DEF  CancelWaitingRead,
           L1!RequestCallCancellation, L1!L0!IsUnusedCall, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>7. CASE CancelParsingRead(c2)
          BY <2>0, <2>7, SMT DEF  CancelParsingRead,
           L1!RequestCallCancellation, L1!L0!IsUnusedCall, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>8. CASE FinishCancelledParse(c2)
          BY <2>0, <2>8, SMT DEF  FinishCancelledParse, ConsumingTerminal,
           L1!HostConsumesEvent, L1!L0!HasStatus, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
           L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, ManagedStutter, vars, l1_vars,
           managed_vars, ManagedRuntimeVars, ManagedChannelVars,
           ManagedCallVars, ReaderVars, WriterVars
  <2>9. CASE HandoffToDrain(c2)
          BY <2>0, <2>9, SMT DEF  HandoffToDrain, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>10. CASE ConsumeHeader(c2)
          BY <2>0, <2>10, SMT DEF  ConsumeHeader, RingTail,
           L1!HostConsumesEvent, PayloadOwed, L1!HostOwnsSomePayload,
           L1!OwedPayloads, RingDrained, RingHead, ConsumingTerminal,
           ManagedIndInv, ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>11. CASE BeginDisposeCall(c2)
          BY <2>0, <2>11, SMT DEF  BeginDisposeCall,
           L1!RequestCallCancellation, L1!L0!IsUnusedCall, L1!L0!IsActiveCall,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
           ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>12. CASE DisposeCallForChannel(c2)
          BY <2>0, <2>12, SMT DEF  DisposeCallForChannel, BeginDisposeCall,
           L1!RequestCallCancellation, L1!L0!IsUnusedCall, L1!L0!IsActiveCall,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
           ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>13. CASE DrainRelease(c2)
          BY <2>0, <2>13, SMT DEF  DrainRelease, ConsumingTerminal,
           L1!HostConsumesEvent, L1!L0!HasStatus, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
           L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>14. CASE FinishDisposeCall(c2)
          BY <2>0, <2>14, SMT DEF  FinishDisposeCall, RingDrained,
           L1!ReleaseCallHandle, L1!L0!IsActiveCall, L1!L0!IsUnusedCall,
           L1!L0!HasStatus, PayloadOwed, L1!HostOwnsSomePayload,
           L1!OwedPayloads, RingHead, RingTail, ConsumingTerminal,
           ManagedIndInv, ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>15. CASE SettleCall(c2)
          BY <2>0, <2>15, SMT DEF  SettleCall, RingDrained,
           L1!HostHoldsNoBuffer, L1!HostOwnsNoPayload, L1!L0!IsTerminalCall,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>16. CASE CancelWriterWait(c2)
          BY <2>0, <2>16, SMT DEF  CancelWriterWait, L1!L0!IsActiveCall,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
           ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>17. CASE WriteDoneCompletes(c2)
          BY <2>0, <2>17, SMT DEF  WriteDoneCompletes, L1!WriteDoneReturns,
           PayloadOwed, L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained,
           RingHead, RingTail, ConsumingTerminal, ManagedIndInv,
           ManagedTypeOK, ManagedMachineInv, ReaderInv,
           ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv, L1!vars,
           L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>18. CASE CloseWriter(c2)
          BY <2>0, <2>18, SMT DEF  CloseWriter, BindingMayDowncall,
           L1!EndSend, L1!L0!EndSend, PayloadOwed, L1!HostOwnsSomePayload,
           L1!OwedPayloads, RingDrained, RingHead, RingTail,
           ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
           L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>19. CASE OnEventReturns(c2)
          BY <2>0, <2>19, SMT DEF  OnEventReturns, L1!DeliveryCallbackReturns,
           L1!L0!HasStatus, PayloadOwed, L1!HostOwnsSomePayload,
           L1!OwedPayloads, RingDrained, RingHead, RingTail,
           ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
           ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
           L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
           L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
           ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
           ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
  <2>20. CASE TerminalCallbackReturns(c2)
          BY <2>0, <2>20, SMT DEF  TerminalCallbackReturns,
           L1!DeliveryCallbackReturns, L1!L0!HasStatus, PayloadOwed,
           L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
           RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
           ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
           L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
           L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
           FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
           ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
           ReaderVars, WriterVars
  <2>21. QED BY <2>0,  <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8,
         <2>9, <2>10, <2>11, <2>12, <2>13, <2>14, <2>15, <2>16, <2>17,
         <2>18, <2>19, <2>20
<1>7. CASE \E cId \in CallIds, chId \in ChannelIds : StartCall(cId, chId)
    BY <1>7, SMT DEF  StartCall, BindingMayDowncall, L1!CallStart,
       L1!L0!CallStart, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>8. CASE \E cId \in CallIds, b \in BufferIds,
           len \in L1!Sizes, charge \in L1!Sizes :
             WriteLendSucceeds(cId, b, len, charge)
    BY <1>8, SMT DEF  WriteLendSucceeds, BindingMayDowncall,
       L1!LendSendBuffer, L1!L0!IsActiveCall, PayloadOwed,
       L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
       RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
       ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
       L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
       ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
       ReaderVars, WriterVars
<1>9. CASE \E cId \in CallIds, len \in L1!Sizes, charge \in L1!CandidateCharges :
             WriteRefusedBudget(cId, len, charge)
    BY <1>9, SMT DEF  WriteRefusedBudget, BindingMayDowncall,
       L1!RefuseLendForBudget, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>10. CASE \E cId \in CallIds, len \in L1!RequestLengths :
             WriteRefusedTooLarge(cId, len)
    BY <1>10, SMT DEF  WriteRefusedTooLarge, BindingMayDowncall,
       L1!RefuseLendTooLarge, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>11. CASE \E cId \in CallIds, b \in BufferIds, charge \in L1!Sizes :
             RetryLendSucceeds(cId, b, charge)
    BY <1>11, SMT DEF  RetryLendSucceeds, BindingMayDowncall,
       L1!LendSendBuffer, L1!L0!IsActiveCall, PayloadOwed,
       L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
       RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
       ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
       L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
       ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
       ReaderVars, WriterVars
<1>12. CASE \E cId \in CallIds, msg \in Messages, b \in BufferIds :
             CommitWrite(cId, msg, b)
    BY <1>12, SMT DEF  CommitWrite, BindingMayDowncall, L1!SendMessage,
       L1!L0!SendMessage, PayloadOwed, L1!HostOwnsSomePayload,
       L1!OwedPayloads, RingDrained, RingHead, RingTail,
       ConsumingTerminal, ManagedIndInv, ManagedTypeOK, ManagedMachineInv,
       ReaderInv, ConsumerPhaseMatchesDispose, L1!TypeOK, L1!IndInv,
       L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars, L1!L0!RuntimeVars,
       L1!L0!ChannelVars, L1!L0!CallVars, FinishCancelledParse,
       ManagedStutter, vars, l1_vars, managed_vars, ManagedRuntimeVars,
       ManagedChannelVars, ManagedCallVars, ReaderVars, WriterVars
<1>13. CASE \E cId \in CallIds, b \in BufferIds : WriteAborted(cId, b)
    BY <1>13, SMT DEF  WriteAborted, L1!HostReturnsBuffer, PayloadOwed,
       L1!HostOwnsSomePayload, L1!OwedPayloads, RingDrained, RingHead,
       RingTail, ConsumingTerminal, ManagedIndInv, ManagedTypeOK,
       ManagedMachineInv, ReaderInv, ConsumerPhaseMatchesDispose,
       L1!TypeOK, L1!IndInv, L1!vars, L1!l0_vars, L1!ffi_vars, L1!L0!vars,
       L1!L0!RuntimeVars, L1!L0!ChannelVars, L1!L0!CallVars,
       FinishCancelledParse, ManagedStutter, vars, l1_vars, managed_vars,
       ManagedRuntimeVars, ManagedChannelVars, ManagedCallVars,
       ReaderVars, WriterVars
<1>q. QED
    BY <1>0, <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12, <1>13 DEF Next

===============================================================================
