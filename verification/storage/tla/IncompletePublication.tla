----------------------- MODULE IncompletePublication -----------------------
EXTENDS Naturals, EnvironmentIncomplete, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Mode, Prefix, Tail, Mutation
ASSUME /\ Mode \in {"standalone", "packed"} /\ Prefix \in Nat \ {0} /\ Tail \in Nat \ {0}
       /\ Mutation \in {"none", "wrong_floor", "skip_sync", "seal_early", "forget_guard", "reject_tombstone"}
VARIABLES phase, length, durableLength, present, durablePresent, sealed, tombstoned,
          pending, otherTried, otherAccepted, protocolError, unstable
vars == <<phase, length, durableLength, present, durablePresent, sealed, tombstoned,
          pending, otherTried, otherAccepted, protocolError, unstable>>
First == IF Mode = "standalone" THEN "unlink" ELSE "truncate"
Init == /\ phase = First /\ length = Prefix + Tail /\ durableLength \in Prefix..(Prefix + Tail)
        /\ present = TRUE /\ durablePresent \in BOOLEAN /\ sealed = FALSE /\ tombstoned = FALSE
        /\ pending = TRUE /\ otherTried = FALSE /\ otherAccepted = FALSE /\ protocolError = FALSE /\ unstable = TRUE
Truncate == /\ phase = "truncate"
            /\ length' = EnvTruncatedLength(IF Mutation = "wrong_floor" THEN 0 ELSE Prefix)
            /\ phase' = IF Mutation = "skip_sync" THEN "finish" ELSE "file_sync"
            /\ UNCHANGED <<durableLength, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
TruncateFails == /\ phase = "truncate" /\ unstable /\ length' \in EnvFailedTruncate(Prefix, length) /\ phase' = "retry"
                 /\ UNCHANGED <<durableLength, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
FileSync == /\ phase = "file_sync" /\ durableLength' = EnvSyncedState(length) /\ phase' = "finish"
            /\ UNCHANGED <<length, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
FileSyncFails == /\ phase = "file_sync" /\ unstable /\ durableLength' \in {durableLength, length} /\ phase' = "retry"
                 /\ UNCHANGED <<length, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
Unlink == /\ phase = "unlink" /\ present' = FALSE
          /\ phase' = IF Mutation = "seal_early" THEN "seal" ELSE "parent_sync"
          /\ UNCHANGED <<length, durableLength, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
UnlinkFails == /\ phase = "unlink" /\ unstable /\ present' \in EnvNamesAfterFailure(present, durablePresent) /\ phase' = "retry"
               /\ UNCHANGED <<length, durableLength, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
ParentSync == /\ phase = "parent_sync" /\ durablePresent' = EnvSyncedState(present) /\ phase' = "seal"
              /\ UNCHANGED <<length, durableLength, present, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
ParentSyncFails == /\ phase = "parent_sync" /\ unstable /\ durablePresent' \in {durablePresent, present} /\ phase' = "retry"
                   /\ UNCHANGED <<length, durableLength, present, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
Seal == /\ phase = "seal"
        /\ IF Mutation = "reject_tombstone" /\ tombstoned
              THEN /\ protocolError' = TRUE /\ phase' = "retry" /\ UNCHANGED sealed
              ELSE /\ sealed' = TRUE /\ phase' = "finish" /\ UNCHANGED protocolError
        /\ UNCHANGED <<length, durableLength, present, durablePresent, tombstoned, pending, otherTried, otherAccepted, unstable>>
SealFails == /\ phase = "seal" /\ unstable /\ sealed' \in EnvUncertainBoolean(sealed) /\ phase' = "retry"
             /\ UNCHANGED <<length, durableLength, present, durablePresent, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
GC == /\ sealed /\ ~tombstoned /\ tombstoned' = TRUE
      /\ UNCHANGED <<phase, length, durableLength, present, durablePresent, sealed, pending, otherTried, otherAccepted, protocolError, unstable>>
Retry == /\ phase = "retry" /\ phase' = First
         /\ UNCHANGED <<length, durableLength, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
Finish == /\ phase = "finish" /\ pending' = FALSE /\ phase' = "done"
          /\ UNCHANGED <<length, durableLength, present, durablePresent, sealed, tombstoned, otherTried, otherAccepted, protocolError, unstable>>
OtherRequest == /\ pending /\ ~otherTried /\ otherTried' = TRUE
                /\ otherAccepted' = (Mutation = "forget_guard")
                /\ UNCHANGED <<phase, length, durableLength, present, durablePresent, sealed, tombstoned, pending, protocolError, unstable>>
StorageCrash == /\ unstable /\ pending /\ phase' = "retry"
                /\ length' = durableLength /\ present' = durablePresent
                /\ UNCHANGED <<durableLength, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError, unstable>>
StopFaults == /\ unstable /\ unstable' = FALSE
              /\ UNCHANGED <<phase, length, durableLength, present, durablePresent, sealed, tombstoned, pending, otherTried, otherAccepted, protocolError>>
Next == Truncate \/ TruncateFails \/ FileSync \/ FileSyncFails \/ Unlink \/ UnlinkFails \/ ParentSync \/ ParentSyncFails
        \/ Seal \/ SealFails \/ GC \/ Retry \/ Finish \/ OtherRequest \/ StorageCrash \/ StopFaults
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, StopFaults)
        /\ EnvWeakFairness(vars, Truncate) /\ EnvWeakFairness(vars, FileSync)
        /\ EnvWeakFairness(vars, Unlink) /\ EnvWeakFairness(vars, ParentSync)
        /\ EnvWeakFairness(vars, Seal) /\ EnvWeakFairness(vars, Retry) /\ EnvWeakFairness(vars, Finish)
TypeOK == /\ phase \in {"truncate", "file_sync", "unlink", "parent_sync", "seal", "retry", "finish", "done"}
          /\ length \in 0..(Prefix + Tail) /\ durableLength \in 0..(Prefix + Tail)
          /\ present \in BOOLEAN /\ durablePresent \in BOOLEAN /\ sealed \in BOOLEAN /\ tombstoned \in BOOLEAN
          /\ pending \in BOOLEAN /\ otherTried \in BOOLEAN /\ otherAccepted \in BOOLEAN /\ protocolError \in BOOLEAN /\ unstable \in BOOLEAN
PrefixPreserved == Mode = "packed" => length >= Prefix /\ durableLength >= Prefix
WriterFenced == ~otherAccepted
ReusableIsClean == ~pending => IF Mode = "packed" THEN length = Prefix /\ durableLength = Prefix
                              ELSE ~present /\ ~durablePresent /\ sealed
NoEarlySeal == sealed => ~durablePresent
NoFalseRetirementError == ~protocolError
EventuallyReusable == <> (phase = "done")
=============================================================================
