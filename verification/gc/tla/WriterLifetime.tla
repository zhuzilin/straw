-------------------------- MODULE WriterLifetime -------------------------
EXTENDS Naturals, EnvironmentWriter, EnvironmentProgress
CONSTANTS Scenario, Mutation, RequireRetirement
ASSUME /\ Scenario \in {"arbitrary", "clean", "crashed"}
       /\ Mutation \in {"none", "timeout_seal", "death_seal"}
       /\ RequireRetirement \in BOOLEAN
\* One unowned, registered open pack with an already authorized write.
\* Successful completion followed by release is abstracted into Finish;
\* ownership transfer, byte persistence and partial-file names are not composed.
\* All storage/catalog operations succeed. No timeout or I/O bound is assumed.
VARIABLES process, inFlight, closed, sealed, suspected, tombstone, present, wroteRetired
vars == <<process, inFlight, closed, sealed, suspected, tombstone, present, wroteRetired>>
Init == /\ process = IF Scenario = "crashed" THEN "dead" ELSE "running"
        /\ inFlight = TRUE /\ present = TRUE
        /\ closed = FALSE /\ sealed = FALSE /\ suspected = FALSE
        /\ tombstone = FALSE /\ wroteRetired = FALSE
ProcessStep == /\ Scenario = "arbitrary" /\ ~closed
               /\ process' \in EnvWriterSuccessors(process)
               /\ UNCHANGED <<inFlight, closed, sealed, suspected, tombstone, present, wroteRetired>>
Observe == /\ suspected' \in EnvWriterSuspicions(process)
           /\ UNCHANGED <<process, inFlight, closed, sealed, tombstone, present, wroteRetired>>
Finish == /\ EnvWriterCompletes(process, inFlight)
          /\ inFlight' = FALSE /\ wroteRetired' = tombstone
          /\ UNCHANGED <<process, closed, sealed, suspected, tombstone, present>>
\* The owner stops using this pack before allowing collection; real close,
\* pending guards and catalog serialization are adapter obligations.
Close == /\ EnvWriterRuns(process) /\ ~inFlight /\ ~closed
         /\ closed' = TRUE /\ sealed' = TRUE
         /\ UNCHANGED <<process, inFlight, suspected, tombstone, present, wroteRetired>>
\* Rejected proposal, not current straw behavior: a suspect paused writer is
\* declared sealed without fencing its already authorized operation.
TimeoutSeal == /\ Mutation = "timeout_seal" /\ suspected
               /\ process = "paused" /\ ~sealed
               /\ sealed' = TRUE
               /\ UNCHANGED <<process, inFlight, closed, suspected, tombstone, present, wroteRetired>>
\* Even certified process death is insufficient: previously authorized work
\* may still reach the filesystem. Retirement also needs a draining boundary.
DeathSeal == /\ Mutation = "death_seal" /\ process = "dead" /\ ~sealed
             /\ sealed' = TRUE
             /\ UNCHANGED <<process, inFlight, closed, suspected, tombstone, present, wroteRetired>>
Mark == /\ sealed /\ ~tombstone /\ tombstone' = TRUE
        /\ UNCHANGED <<process, inFlight, closed, sealed, suspected, present, wroteRetired>>
Delete == /\ tombstone /\ present /\ present' = FALSE
          /\ UNCHANGED <<process, inFlight, closed, sealed, suspected, tombstone, wroteRetired>>
Next == ProcessStep \/ Observe \/ Finish \/ Close \/ TimeoutSeal \/ DeathSeal \/ Mark \/ Delete
Spec == Init /\ [][Next]_vars
        /\ EnvWeakFairness(vars, Finish) /\ EnvWeakFairness(vars, Close)
        /\ EnvWeakFairness(vars, Mark) /\ EnvWeakFairness(vars, Delete)
TypeOK == /\ process \in EnvWriterStates
          /\ inFlight \in BOOLEAN /\ closed \in BOOLEAN /\ sealed \in BOOLEAN
          /\ suspected \in BOOLEAN /\ tombstone \in BOOLEAN /\ present \in BOOLEAN
          /\ wroteRetired \in BOOLEAN
NoLiveWriterRetired == tombstone => ~inFlight
NoWriteAfterRetirement == ~wroteRetired
RequiredProgress == IF RequireRetirement THEN <>[] ~present ELSE (closed ~> ~present)
=============================================================================
