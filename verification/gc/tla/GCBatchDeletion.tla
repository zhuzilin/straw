------------------------- MODULE GCBatchDeletion -------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANT Mutation
ASSUME Mutation \in {"none", "early_sync", "missing_parent", "drop_parent"}
\* Three permanently unowned packs; two share a directory, one does not.
\* Stable parent identities and durable tombstones fence concurrent recreation.
Packs == 1..3
Dirs == 1..2
Parent(p) == IF p = 3 THEN 2 ELSE 1
Members(d) == {p \in Packs : Parent(p) = d}
Plan(names) == IF Mutation = "missing_parent" THEN {Parent(p) : p \in names}
              ELSE IF Mutation = "drop_parent" THEN {1} ELSE Dirs
VARIABLES marked, present, durable, unlinked, synced, planned, completed, unstable
vars == <<marked, present, durable, unlinked, synced, planned, completed, unstable>>
Init == /\ marked = FALSE /\ present = Packs /\ durable = Packs
        /\ unlinked = {} /\ synced = {} /\ planned = Plan(Packs)
        /\ completed = FALSE /\ unstable = TRUE
Mark == /\ ~marked /\ marked' = TRUE
        /\ UNCHANGED <<present, durable, unlinked, synced, planned, completed, unstable>>
Unlink(p) == /\ marked /\ ~completed /\ p \notin unlinked
             /\ present' = present \ {p} /\ unlinked' = unlinked \cup {p}
             /\ UNCHANGED <<marked, durable, synced, planned, completed, unstable>>
Sync(d) == /\ marked /\ ~completed /\ d \in planned \ synced
           /\ (unlinked = Packs \/ Mutation = "early_sync")
           /\ durable' = EnvSyncedDirectory(present, durable, Members(d))
           /\ synced' = synced \cup {d}
           /\ UNCHANGED <<marked, present, unlinked, planned, completed, unstable>>
Complete == /\ marked /\ ~completed /\ unlinked = Packs /\ synced = planned
            /\ completed' = TRUE
            /\ UNCHANGED <<marked, present, durable, unlinked, synced, planned, unstable>>
\* A failed call can leave partial effects, or a crash can restore an unsynced
\* name. A fresh call retries every tombstone and plans every parent again.
Retry == /\ unstable /\ ~completed
         /\ \E names \in SUBSET Packs :
              /\ \A p \in Packs : (p \in names) \in EnvNamesAfterFailure(p \in present, p \in durable)
              /\ present' = names /\ planned' = Plan(names)
         /\ unlinked' = {} /\ synced' = {}
         /\ UNCHANGED <<marked, durable, completed, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<marked, present, durable, unlinked, synced, planned, completed>>
Next == Mark \/ (\E p \in Packs : Unlink(p)) \/ (\E d \in Dirs : Sync(d))
        \/ Complete \/ Retry \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Mark) /\ EnvWeakFairness(vars, Complete)
        /\ (\A p \in Packs : EnvWeakFairness(vars, Unlink(p)))
        /\ (\A d \in Dirs : EnvWeakFairness(vars, Sync(d)))
TypeOK == /\ marked \in BOOLEAN /\ present \subseteq Packs /\ durable \subseteq Packs
          /\ unlinked \subseteq Packs /\ synced \subseteq planned /\ planned \subseteq Dirs
          /\ completed \in BOOLEAN /\ unstable \in BOOLEAN
DeleteAfterMark == present # Packs => marked
DurableCompletion == completed => durable = {}
EventuallyReclaimed == <> (completed /\ durable = {})
=============================================================================
