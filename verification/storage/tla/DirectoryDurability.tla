------------------------ MODULE DirectoryDurability ------------------------
EXTENDS Naturals, FiniteSets, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Depth, Exists, Durable, InitialPrefix, InitiallyUnstable, Mutation
ASSUME /\ Depth \in Nat \ {0} /\ Exists \in BOOLEAN /\ Durable \in BOOLEAN
       /\ (Durable => Exists)
       /\ InitialPrefix \in 0..Depth
       /\ InitiallyUnstable \in BOOLEAN
       /\ Mutation \in {"none", "existence_is_durable", "skip_root", "ignore_sync_error"}
\* Entry i names directory i in its parent i-1; root 0 is a stable anchor.
\* A later file fsync and leaf-directory fsync cannot persist ancestor entries.
VARIABLES phase, present, durable, cursor, floor, unstable, acknowledged, payload, crashed
vars == <<phase, present, durable, cursor, floor, unstable, acknowledged, payload, crashed>>
Init == /\ phase = "begin" /\ present = (IF Exists THEN Depth ELSE InitialPrefix)
        /\ durable = (IF Durable THEN 1..Depth ELSE {})
        /\ cursor = Depth /\ floor = 0 /\ unstable = InitiallyUnstable /\ acknowledged = FALSE
        /\ payload = FALSE /\ crashed = FALSE
Begin == /\ phase = "begin"
         /\ phase' = IF Mutation = "existence_is_durable" /\ present = Depth
                     THEN "publish" ELSE "create"
         /\ UNCHANGED <<present, durable, cursor, floor, unstable, acknowledged, payload, crashed>>
Create == /\ phase = "create" /\ present' = Depth /\ phase' = "sync" /\ cursor' = Depth
          \* The old recursive helper stopped at an existing ancestor. It
          \* could sync that directory's new children but not its own name.
          /\ floor' = IF Mutation = "existence_is_durable" THEN present ELSE 0
          /\ UNCHANGED <<durable, unstable, acknowledged, payload, crashed>>
CreateFails == /\ unstable /\ phase = "create" /\ present' \in EnvCreatedDepth(present, Depth)
               /\ phase' = "error"
               /\ UNCHANGED <<durable, cursor, floor, unstable, acknowledged, payload, crashed>>
Synced == IF cursor < Depth THEN EnvDirectoryEntrySynced(durable, cursor + 1) ELSE durable
Sync == /\ phase = "sync"
        /\ durable' = IF Mutation = "skip_root" /\ cursor = 0 THEN durable ELSE Synced
        /\ phase' = IF cursor = floor THEN "publish" ELSE "sync"
        /\ cursor' = IF cursor = floor THEN 0 ELSE cursor - 1
        /\ UNCHANGED <<present, floor, unstable, acknowledged, payload, crashed>>
SyncFails == /\ unstable /\ phase = "sync" /\ durable' \in {durable, Synced}
             /\ phase' = IF Mutation = "ignore_sync_error" THEN "publish" ELSE "error"
             /\ UNCHANGED <<present, cursor, floor, unstable, acknowledged, payload, crashed>>
Retry == /\ phase = "error" /\ phase' = "begin"
         /\ UNCHANGED <<present, durable, cursor, floor, unstable, acknowledged, payload, crashed>>
Publish == /\ phase = "publish" /\ present = Depth
           /\ phase' = "done" /\ acknowledged' = TRUE /\ payload' = TRUE
           /\ UNCHANGED <<present, durable, cursor, floor, unstable, crashed>>
Crash == /\ ~crashed /\ crashed' = TRUE
         /\ present' = Cardinality(EnvReachableDirectories(durable, Depth))
         /\ durable' = EnvReachableDirectories(durable, Depth)
         /\ payload' = (payload /\ 1..Depth \subseteq durable)
         /\ phase' = IF acknowledged THEN "done" ELSE "begin"
         /\ UNCHANGED <<cursor, floor, unstable, acknowledged>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, present, durable, cursor, floor, acknowledged, payload, crashed>>
Next == Begin \/ Create \/ CreateFails \/ Sync \/ SyncFails \/ Retry \/ Publish \/ Crash \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Begin) /\ EnvWeakFairness(vars, Create)
        /\ EnvWeakFairness(vars, Sync) /\ EnvWeakFairness(vars, Retry)
        /\ EnvWeakFairness(vars, Publish)
TypeOK == /\ phase \in {"begin", "create", "sync", "error", "publish", "done"}
          /\ present \in 0..Depth /\ durable \subseteq 1..Depth /\ cursor \in 0..Depth
          /\ floor \in 0..Depth
          /\ unstable \in BOOLEAN /\ acknowledged \in BOOLEAN /\ payload \in BOOLEAN /\ crashed \in BOOLEAN
AcknowledgedPathDurable == acknowledged => 1..Depth \subseteq durable
NoLostAcknowledgedData == acknowledged => payload
EventuallyPublished == <> acknowledged
=============================================================================
