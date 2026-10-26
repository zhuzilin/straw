--------------------------- MODULE GCReceipt ---------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANT Mutation
ASSUME Mutation \in {"none", "record_before_sync", "absence_is_receipt", "forget_tombstone",
                    "skip_unlink_after_stat_missing"}
\* One permanently unowned, durably tombstoned pack. A receipt is an optional
\* optimization: losing an uncertain receipt causes a retry, never lost fencing.
VARIABLES phase, present, durablePresent, written, durableReceipt, cachedDone, tombstoned, unstable, statPresent
vars == <<phase, present, durablePresent, written, durableReceipt, cachedDone, tombstoned, unstable, statPresent>>
Init == /\ phase = "stat" /\ present = TRUE /\ durablePresent = TRUE /\ statPresent = FALSE
        /\ written = FALSE /\ durableReceipt = FALSE /\ cachedDone = FALSE
        /\ tombstoned = TRUE /\ unstable = TRUE
Stat == /\ phase = "stat" /\ statPresent' \in EnvStatPresence /\ phase' = "unlink"
        /\ UNCHANGED <<present, durablePresent, written, durableReceipt, cachedDone, tombstoned, unstable>>
Unlink == /\ phase = "unlink" /\ phase' = "sync"
          /\ present' = IF Mutation = "skip_unlink_after_stat_missing" /\ ~statPresent
                         THEN present ELSE FALSE
          /\ UNCHANGED <<durablePresent, written, durableReceipt, cachedDone, tombstoned, unstable, statPresent>>
Sync == /\ phase = "sync" /\ durablePresent' = EnvSyncedState(present) /\ phase' = "record"
        /\ UNCHANGED <<present, written, durableReceipt, cachedDone, tombstoned, unstable, statPresent>>
WriteReceipt == /\ (phase = "record" \/ (Mutation = "record_before_sync" /\ phase = "sync"))
                /\ written' = TRUE /\ phase' = "receipt_sync"
                /\ UNCHANGED <<present, durablePresent, durableReceipt, cachedDone, tombstoned, unstable, statPresent>>
SyncReceipt == /\ phase = "receipt_sync" /\ durableReceipt' = EnvSyncedState(written)
               /\ cachedDone' = TRUE /\ phase' = "done"
               /\ tombstoned' = (Mutation # "forget_tombstone")
               /\ UNCHANGED <<present, durablePresent, written, unstable, statPresent>>
Crash == /\ unstable
         /\ present' \in EnvNamesAfterFailure(present, durablePresent)
         /\ written' \in EnvRecoveredFrames(durableReceipt, written, FALSE)
         /\ cachedDone' = FALSE /\ phase' = "recover"
         /\ UNCHANGED <<durablePresent, durableReceipt, tombstoned, unstable, statPresent>>
\* Replay syncs a surviving complete frame before another mutation depends
\* on it. Torn/missing frames retain the deletion obligation.
Recover == /\ phase = "recover" /\ durableReceipt' = EnvSyncedState(written)
           /\ cachedDone' = (written \/ (Mutation = "absence_is_receipt" /\ ~present))
           /\ phase' = IF cachedDone' THEN "done" ELSE "stat"
           /\ UNCHANGED <<present, durablePresent, written, tombstoned, unstable, statPresent>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, present, durablePresent, written, durableReceipt, cachedDone, tombstoned, statPresent>>
Next == Stat \/ Unlink \/ Sync \/ WriteReceipt \/ SyncReceipt \/ Crash \/ Recover \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Stat) /\ EnvWeakFairness(vars, Unlink) /\ EnvWeakFairness(vars, Sync)
        /\ EnvWeakFairness(vars, WriteReceipt) /\ EnvWeakFairness(vars, SyncReceipt)
        /\ EnvWeakFairness(vars, Recover)
TypeOK == /\ phase \in {"stat", "unlink", "sync", "record", "receipt_sync", "done", "recover"}
          /\ present \in BOOLEAN /\ durablePresent \in BOOLEAN /\ written \in BOOLEAN
          /\ durableReceipt \in BOOLEAN /\ cachedDone \in BOOLEAN
          /\ tombstoned \in BOOLEAN /\ unstable \in BOOLEAN /\ statPresent \in BOOLEAN
ReceiptIsSafe == (written \/ durableReceipt) => ~durablePresent
SkipRequiresReceipt == cachedDone => (durableReceipt /\ ~durablePresent)
ReceiptKeepsFence == (written \/ durableReceipt) => tombstoned
EventuallyRetired == <>[] (cachedDone /\ ~durablePresent)
=============================================================================
