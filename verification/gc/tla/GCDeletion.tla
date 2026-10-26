-------------------------- MODULE GCDeletion --------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANT Mutation
ASSUME Mutation \in {"none", "skip_missing_sync", "unlink_before_mark"}
VARIABLES phase, tombstoned, present, durablePresent, completed, unstable
vars == <<phase, tombstoned, present, durablePresent, completed, unstable>>

\* One sealed pack has permanently lost its last owner. Ownership and late
\* retain fencing are checked separately by Ownership.tla and DurableHandoff.tla.
\* An operation may fail after producing its volatile or durable side effect.
Init == /\ phase = "scan" /\ tombstoned = FALSE /\ present = TRUE /\ durablePresent = TRUE
        /\ completed = FALSE /\ unstable = TRUE
Mark == /\ phase = "scan" /\ phase' = "unlink"
        /\ tombstoned' = (Mutation # "unlink_before_mark")
        /\ UNCHANGED <<present, durablePresent, completed, unstable>>
Unlink == /\ phase = "unlink"
          /\ IF Mutation = "skip_missing_sync" /\ ~present
                THEN /\ phase' = "done" /\ completed' = TRUE
                ELSE /\ phase' = "sync" /\ completed' = completed
          /\ present' = FALSE
          /\ UNCHANGED <<tombstoned, durablePresent, unstable>>
Sync == /\ phase = "sync" /\ phase' = "done"
        /\ durablePresent' = EnvSyncedState(present) /\ completed' = TRUE
        /\ UNCHANGED <<tombstoned, present, unstable>>
\* Retry after a failed call need not undo unlink. A crash may instead restore
\* the unsynced directory entry. Failures can recur arbitrarily before Stop.
Retry == /\ unstable /\ phase \in {"unlink", "sync"}
         /\ phase' = "scan" /\ present' \in EnvNamesAfterFailure(present, durablePresent)
         /\ UNCHANGED <<tombstoned, durablePresent, completed, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, tombstoned, present, durablePresent, completed>>
Next == Mark \/ Unlink \/ Sync \/ Retry \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop) /\ EnvWeakFairness(vars, Mark)
        /\ EnvWeakFairness(vars, Unlink) /\ EnvWeakFairness(vars, Sync)
TypeOK == /\ phase \in {"scan", "unlink", "sync", "done"}
          /\ tombstoned \in BOOLEAN /\ present \in BOOLEAN
          /\ durablePresent \in BOOLEAN /\ completed \in BOOLEAN /\ unstable \in BOOLEAN
DeleteAfterMark == ~present => tombstoned
DurableCompletion == completed => ~durablePresent
EventuallyReclaimed == <> (completed /\ ~durablePresent)
=============================================================================
