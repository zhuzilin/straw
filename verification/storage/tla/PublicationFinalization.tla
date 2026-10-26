------------------------ MODULE PublicationFinalization ------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Standalone, InitiallyUnstable, Mutation
ASSUME /\ Standalone \in BOOLEAN /\ InitiallyUnstable \in BOOLEAN
       /\ Mutation \in {"none", "skip_seal", "seal_early", "ignore_seal_error", "seal_packed"}
\* Start with durable data in a registered open file. Catalog responses abstract
\* the durable result seen after serialized recovery; no byte replay is modeled.
VARIABLES phase, staged, sealed, present, acknowledged, unstable
vars == <<phase, staged, sealed, present, acknowledged, unstable>>
Init == /\ phase = IF Mutation = "seal_early" THEN "seal" ELSE "stage"
        /\ staged = FALSE /\ sealed = FALSE /\ present = TRUE
        /\ acknowledged = FALSE /\ unstable = InitiallyUnstable
NeedsSeal == Standalone \/ Mutation = "seal_packed"
Stage == /\ phase = "stage" /\ present /\ staged' = TRUE
         /\ phase' = IF NeedsSeal /\ Mutation \notin {"skip_seal", "seal_early"} THEN "seal" ELSE "ack"
         /\ UNCHANGED <<sealed, present, acknowledged, unstable>>
StageFails == /\ phase = "stage" /\ unstable /\ present
              /\ staged' \in EnvUncertainBoolean(staged) /\ phase' = "failed"
              /\ UNCHANGED <<sealed, present, acknowledged, unstable>>
Seal == /\ phase = "seal" /\ sealed' = TRUE
        /\ phase' = IF Mutation = "seal_early" THEN "stage" ELSE "ack"
        /\ UNCHANGED <<staged, present, acknowledged, unstable>>
SealFails == /\ phase = "seal" /\ unstable
             /\ sealed' \in EnvUncertainBoolean(sealed)
             /\ phase' = IF Mutation = "ignore_seal_error" THEN "ack" ELSE "failed"
             /\ UNCHANGED <<staged, present, acknowledged, unstable>>
Retry == /\ phase = "failed" /\ phase' = IF Mutation = "seal_early" THEN "seal" ELSE "stage"
         /\ UNCHANGED <<staged, sealed, present, acknowledged, unstable>>
Ack == /\ phase = "ack" /\ acknowledged' = TRUE /\ phase' = "published"
       /\ UNCHANGED <<staged, sealed, present, unstable>>
\* Packed mode closes explicitly after publication, before release/GC.
ClosePack == /\ phase = "published" /\ ~Standalone /\ ~sealed /\ sealed' = TRUE
             /\ UNCHANGED <<phase, staged, present, acknowledged, unstable>>
Release == /\ phase = "published" /\ (Standalone \/ sealed)
           /\ staged' = FALSE /\ phase' = "collect"
           /\ UNCHANGED <<sealed, present, acknowledged, unstable>>
Collect == /\ phase = "collect" /\ phase' = "done" /\ present' = (present /\ ~sealed)
           /\ UNCHANGED <<staged, sealed, acknowledged, unstable>>
CollectEarly == /\ phase \notin {"collect", "done"} /\ sealed /\ ~staged /\ present
                /\ present' = FALSE
                /\ UNCHANGED <<phase, staged, sealed, acknowledged, unstable>>
StopFaults == /\ unstable /\ unstable' = FALSE
              /\ UNCHANGED <<phase, staged, sealed, present, acknowledged>>
Next == Stage \/ StageFails \/ Seal \/ SealFails \/ Retry \/ Ack \/ ClosePack \/ Release \/ Collect \/ CollectEarly \/ StopFaults
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, StopFaults)
        /\ EnvWeakFairness(vars, Stage) /\ EnvWeakFairness(vars, Seal)
        /\ EnvWeakFairness(vars, Retry) /\ EnvWeakFairness(vars, Ack)
        /\ EnvWeakFairness(vars, ClosePack) /\ EnvWeakFairness(vars, Release)
        /\ EnvWeakFairness(vars, Collect)
TypeOK == /\ phase \in {"stage", "seal", "failed", "ack", "published", "collect", "done"}
          /\ staged \in BOOLEAN /\ sealed \in BOOLEAN /\ present \in BOOLEAN
          /\ acknowledged \in BOOLEAN /\ unstable \in BOOLEAN
NoEarlyLoss == phase \notin {"collect", "done"} => present
AcknowledgedStandaloneSealed == Standalone /\ acknowledged => sealed
PackedWritableBeforeReturn == ~Standalone /\ ~acknowledged => ~sealed
ReleasedReclaimed == phase = "done" => ~present
EventuallyReclaimed == <> (phase = "done" /\ ~present)
=============================================================================
