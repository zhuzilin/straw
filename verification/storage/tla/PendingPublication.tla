------------------------- MODULE PendingPublication -------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Mode, FailureAt, AllowCorruption, Mutation
ASSUME /\ Mode \in {"standalone", "packed"}
       /\ FailureAt \in {"before", "during", "after"} /\ AllowCorruption \in BOOLEAN
       /\ Mutation \in {"none", "remember_late", "trust_failure", "skip_retry_sync", "skip_validation"}
\* A complete frame and open catalog registration already exist. Its description
\* is process-local; process restart, rename errors and catalog handoff are separate.
VARIABLES phase, pending, confirmed, durable, complete, copies, acknowledged, otherTried, otherAccepted, unstable
vars == <<phase, pending, confirmed, durable, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
Init == /\ phase = "first" /\ pending = (Mode = "packed" \/ Mutation # "remember_late")
        /\ confirmed = FALSE /\ durable = FALSE /\ complete = TRUE /\ copies = 1
        /\ acknowledged = FALSE /\ otherTried = FALSE /\ otherAccepted = FALSE /\ unstable = TRUE
FirstFailure == /\ phase = "first" /\ phase' = "retry"
                /\ durable' \in CASE FailureAt = "before" -> {FALSE}
                                [] FailureAt = "during" -> EnvUncertainBoolean(FALSE)
                                [] OTHER -> {EnvSyncedState(TRUE)}
                /\ confirmed' = (FailureAt = "after" \/ Mutation = "trust_failure")
                /\ UNCHANGED <<pending, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
\* Authentication rejects a damaged completed file even if a prior sync succeeded.
Corrupt == /\ AllowCorruption /\ phase = "retry" /\ complete /\ complete' = FALSE
           /\ UNCHANGED <<phase, pending, confirmed, durable, copies, acknowledged, otherTried, otherAccepted, unstable>>
OtherRequest == /\ phase = "retry" /\ ~otherTried /\ otherTried' = TRUE
                /\ otherAccepted' = ~pending
                /\ phase' = IF pending THEN phase ELSE "other"
                /\ UNCHANGED <<pending, confirmed, durable, complete, copies, acknowledged, unstable>>
Retry == /\ phase = "retry"
         /\ IF ~pending
               THEN /\ copies' = 2 /\ pending' = TRUE /\ confirmed' = FALSE
                    /\ complete' = TRUE /\ durable' = FALSE /\ phase' = "sync"
               ELSE /\ phase' = IF Mutation = "skip_retry_sync" \/ (confirmed /\ Mode = "standalone")
                                THEN "publish" ELSE "sync"
                    /\ UNCHANGED <<copies, pending, confirmed, complete, durable>>
         /\ UNCHANGED <<acknowledged, otherTried, otherAccepted, unstable>>
Sync == /\ phase = "sync" /\ durable' = EnvSyncedState(TRUE) /\ confirmed' = TRUE /\ phase' = "publish"
        /\ UNCHANGED <<pending, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
SyncFails == /\ phase = "sync" /\ unstable /\ durable' \in EnvUncertainBoolean(durable)
             /\ phase' = "retry"
             /\ UNCHANGED <<pending, confirmed, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
Publish == /\ phase = "publish" /\ phase' = IF Mutation = "skip_validation" THEN "ack" ELSE "verify"
           /\ UNCHANGED <<pending, confirmed, durable, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
Verify == /\ phase = "verify" /\ phase' = IF complete THEN "ack" ELSE "error"
          /\ UNCHANGED <<pending, confirmed, durable, complete, copies, acknowledged, otherTried, otherAccepted, unstable>>
Ack == /\ phase = "ack" /\ acknowledged' = TRUE /\ phase' = "done"
       /\ UNCHANGED <<pending, confirmed, durable, complete, copies, otherTried, otherAccepted, unstable>>
StopFaults == /\ unstable /\ unstable' = FALSE
              /\ UNCHANGED <<phase, pending, confirmed, durable, complete, copies, acknowledged, otherTried, otherAccepted>>
Next == FirstFailure \/ Corrupt \/ OtherRequest \/ Retry \/ Sync \/ SyncFails \/ Publish \/ Verify \/ Ack \/ StopFaults
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, StopFaults)
        /\ EnvWeakFairness(vars, FirstFailure) /\ EnvWeakFairness(vars, Retry)
        /\ EnvWeakFairness(vars, Sync) /\ EnvWeakFairness(vars, Publish)
        /\ EnvWeakFairness(vars, Verify) /\ EnvWeakFairness(vars, Ack)
TypeOK == /\ phase \in {"first", "retry", "sync", "publish", "verify", "ack", "done", "error", "other"}
          /\ pending \in BOOLEAN /\ confirmed \in BOOLEAN /\ durable \in BOOLEAN /\ complete \in BOOLEAN
          /\ copies \in {1, 2} /\ acknowledged \in BOOLEAN /\ otherTried \in BOOLEAN
          /\ otherAccepted \in BOOLEAN /\ unstable \in BOOLEAN
OneCompletedFile == copies = 1
WriterFenced == ~otherAccepted
DurableVerifiedReturn == acknowledged => (durable /\ complete)
EventuallySettles == <> (phase \in {"done", "error", "other"})
=============================================================================
