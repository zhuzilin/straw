-------------------------- MODULE CatalogReplay --------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Mutation, Tail, ReuseConfirmed
ASSUME /\ Mutation \in {"none", "keep_cache", "keep_owners", "confirm_before_sync",
                        "late_prepare", "unconditional_invalidation", "reuse_dirty"}
       /\ Tail \in BOOLEAN /\ ReuseConfirmed \in BOOLEAN
\* Prefix 1 is confirmed. Complete frame 2 adds an owner, but its writer failed
\* before sync. Storage may lose that unconfirmed tail while this client lives.
\* One serialized refresh is modeled; byte framing and peer appends are outside.
VARIABLES phase, written, durable, known, offset, cachedOwner, rebuild, unstable
vars == <<phase, written, durable, known, offset, cachedOwner, rebuild, unstable>>
Init == /\ phase = "refresh" /\ written = (IF Tail THEN 2 ELSE 1) /\ durable = 1 /\ known = 1
        /\ offset = 1 /\ cachedOwner = FALSE /\ rebuild = FALSE /\ unstable = TRUE
Begin == /\ phase = "refresh"
         /\ LET reuse == ReuseConfirmed /\ written = known /\ offset = known
                         /\ (~rebuild \/ Mutation = "reuse_dirty")
            IN /\ phase' = IF written < known THEN "error" ELSE IF reuse THEN "done" ELSE "read"
               /\ offset' = IF rebuild /\ ~reuse THEN 0 ELSE offset
               /\ cachedOwner' = IF rebuild /\ ~reuse /\ Mutation # "keep_owners" THEN FALSE ELSE cachedOwner
         /\ UNCHANGED <<written, durable, known, rebuild, unstable>>
\* ReuseConfirmed models an exclusively owned writer registry's fast path.
\* Its caller separately checks the gate/fence, exact log identity and length.
\* A caught partial apply never qualifies, even if the physical length matches.
Read == /\ phase = "read"
        /\ phase' = IF offset > written THEN "error" ELSE "sync"
        /\ offset' = IF offset > written THEN offset ELSE written
        /\ cachedOwner' = (cachedOwner \/ (offset < written /\ written = 2))
        /\ known' = IF Mutation = "confirm_before_sync" THEN offset' ELSE known
        /\ rebuild' = (rebuild \/ (offset < written /\ Mutation # "keep_cache"))
        /\ UNCHANGED <<written, durable, unstable>>
\* A tentative event may be rejected after changing an index but before moving
\* the offset. This branch explicitly includes external removal of that failed
\* unconfirmed tail. It does NOT model Straw silently repairing corrupt frames.
PartialApplyAndRepair == /\ phase = "read" /\ unstable /\ offset < written
                        /\ cachedOwner' = (cachedOwner \/ written = 2)
                        /\ rebuild' = (rebuild \/ Mutation \notin {"keep_cache", "late_prepare"})
                        /\ written' = durable /\ phase' = "refresh"
                        /\ UNCHANGED <<durable, known, offset, unstable>>
Sync == /\ phase = "sync" /\ durable' = EnvSyncedState(written)
        /\ known' = offset /\ rebuild' = FALSE /\ phase' = "done"
        /\ UNCHANGED <<written, offset, cachedOwner, unstable>>
FailAndRecoverStorage == /\ phase = "sync" /\ unstable
                         /\ written' \in EnvRecoveredFrames(durable, written, 0)
                         /\ phase' = "refresh"
                         /\ rebuild' = (rebuild \/ Mutation = "unconditional_invalidation")
                         /\ UNCHANGED <<durable, known, offset, cachedOwner, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, written, durable, known, offset, cachedOwner, rebuild>>
Next == Begin \/ Read \/ PartialApplyAndRepair \/ Sync \/ FailAndRecoverStorage \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Begin) /\ EnvWeakFairness(vars, Read)
        /\ EnvWeakFairness(vars, Sync)
TypeOK == /\ phase \in {"refresh", "read", "sync", "done", "error"}
          /\ written \in 1..2 /\ durable \in 1..2 /\ known \in 1..2
          /\ offset \in 0..2 /\ cachedOwner \in BOOLEAN
          /\ rebuild \in BOOLEAN /\ unstable \in BOOLEAN
ConfirmedPrefix == known <= durable
NoFalseCorruption == phase # "error"
NoUnneededRebuild == Tail \/ ~rebuild
CoherentReady == phase = "done" =>
                 (offset = written /\ durable = written /\ cachedOwner = (written = 2))
EventuallyReady == <> (phase = "done")
=============================================================================
