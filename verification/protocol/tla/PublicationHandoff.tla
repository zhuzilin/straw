------------------------ MODULE PublicationHandoff ------------------------
EXTENDS Naturals, FiniteSets, EnvironmentHandoff, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Api, Standalone, Mutation
ASSUME /\ Api \in {"records", "record_set", "publish"} /\ Standalone \in BOOLEAN
       /\ Mutation \in {"none", "legacy", "early_ack", "forget_on_error", "wrong_api", "skip_release"}
\* Begin after data durability, before the first caller-visible success.
\* The exact file description is process-local. Input hashing, byte I/O and
\* complete ownership graphs are not mechanically composed with this model.
VARIABLES phase, active, remembered, acknowledged, rawStage, rootStage, held,
          sealed, present, offered, unstable
vars == <<phase, active, remembered, acknowledged, rawStage, rootStage, held,
          sealed, present, offered, unstable>>
AfterInner == IF Api = "record_set" THEN "outer" ELSE IF Api = "publish" THEN "release" ELSE "ack"
Init == /\ phase = "stage" /\ active = (Mutation # "legacy")
        /\ remembered = TRUE /\ acknowledged = FALSE
        /\ rawStage = FALSE /\ rootStage = FALSE /\ held = (Api # "records")
        /\ sealed = FALSE /\ present = TRUE /\ offered = {} /\ unstable = TRUE
Stage == /\ phase = "stage"
         /\ rawStage' = IF Api = "record_set" THEN EnvStagedRoot ELSE rawStage
         /\ rootStage' = IF Api # "record_set" THEN EnvStagedRoot ELSE rootStage
         /\ phase' = IF Standalone THEN "seal" ELSE AfterInner
         /\ active' = IF Mutation = "early_ack" /\ ~Standalone THEN FALSE ELSE active
         /\ UNCHANGED <<remembered, acknowledged, held, sealed, present, offered, unstable>>
Seal == /\ phase = "seal" /\ sealed' = TRUE /\ phase' = AfterInner
        /\ active' = IF Mutation = "early_ack" THEN FALSE ELSE active
        /\ UNCHANGED <<remembered, acknowledged, rawStage, rootStage, held, present, offered, unstable>>
OuterStage == /\ phase = "outer" /\ rootStage' = EnvStagedRoot /\ phase' = "consume"
              /\ UNCHANGED <<active, remembered, acknowledged, rawStage, held, sealed, present, offered, unstable>>
Consume == /\ phase = "consume" /\ rawStage' = FALSE /\ phase' = "release"
           /\ UNCHANGED <<active, remembered, acknowledged, rootStage, held, sealed, present, offered, unstable>>
Release == /\ phase = "release" /\ held' = IF Mutation = "skip_release" THEN held ELSE FALSE
           /\ phase' = "ack"
           /\ UNCHANGED <<active, remembered, acknowledged, rawStage, rootStage, sealed, present, offered, unstable>>
Fail == /\ phase \in {"stage", "seal", "outer", "consume", "release"} /\ unstable
        /\ rawStage' \in CASE phase = "stage" /\ Api = "record_set" -> EnvUncertainBoolean(rawStage)
                            [] phase = "consume" -> {rawStage, FALSE}
                            [] OTHER -> {rawStage}
        /\ rootStage' \in IF phase = "outer" \/ (phase = "stage" /\ Api # "record_set")
                            THEN EnvUncertainBoolean(rootStage) ELSE {rootStage}
        /\ sealed' \in IF phase = "seal" THEN EnvUncertainBoolean(sealed) ELSE {sealed}
        /\ held' \in IF phase = "release" THEN {held, FALSE} ELSE {held}
        /\ active' = IF Mutation = "forget_on_error" THEN FALSE ELSE active
        /\ phase' = "retry" /\ UNCHANGED <<remembered, acknowledged, present, offered, unstable>>
Retry == /\ phase = "retry" /\ phase' = "stage"
         /\ UNCHANGED <<active, remembered, acknowledged, rawStage, rootStage, held, sealed, present, offered, unstable>>
Ack == /\ phase = "ack" /\ acknowledged' = TRUE /\ active' = FALSE /\ phase' = "done"
       /\ UNCHANGED <<remembered, rawStage, rootStage, held, sealed, present, offered, unstable>>
\* Acceptance of a different request overwrites the old retry cache. The
\* original file can remain protected yet unreachable to its caller (a leak).
Offer(request) == /\ request \in EnvHandoffRequests \ offered
                  /\ offered' = offered \cup {request}
                  /\ remembered' = IF request \in {"other_id", "other_api"}
                                         /\ (~active \/ (Mutation = "wrong_api" /\ request = "other_api"))
                                      THEN FALSE ELSE remembered
                  /\ UNCHANGED <<phase, active, acknowledged, rawStage, rootStage, held, sealed, present, unstable>>
Collect == /\ sealed /\ ~rawStage /\ ~rootStage /\ present /\ present' = FALSE
           /\ UNCHANGED <<phase, active, remembered, acknowledged, rawStage, rootStage, held, sealed, offered, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, active, remembered, acknowledged, rawStage, rootStage, held, sealed, present, offered>>
Next == Stage \/ Seal \/ OuterStage \/ Consume \/ Release \/ Fail \/ Retry \/ Ack
        \/ (\E request \in EnvHandoffRequests: Offer(request)) \/ Collect \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Stage) /\ EnvWeakFairness(vars, Seal)
        /\ EnvWeakFairness(vars, OuterStage) /\ EnvWeakFairness(vars, Consume)
        /\ EnvWeakFairness(vars, Release) /\ EnvWeakFairness(vars, Retry) /\ EnvWeakFairness(vars, Ack)
TypeOK == /\ phase \in {"stage", "seal", "outer", "consume", "release", "retry", "ack", "done"}
          /\ active \in BOOLEAN /\ remembered \in BOOLEAN /\ acknowledged \in BOOLEAN
          /\ rawStage \in BOOLEAN /\ rootStage \in BOOLEAN /\ held \in BOOLEAN
          /\ sealed \in BOOLEAN /\ present \in BOOLEAN /\ offered \subseteq EnvHandoffRequests /\ unstable \in BOOLEAN
NoForgottenUnreturnedFile == ~acknowledged => remembered
NoLostUnreturnedFile == ~acknowledged => present
CompletedHandoff == acknowledged => rootStage /\ ~held /\ (Api = "record_set" => ~rawStage)
StandaloneSealed == acknowledged /\ Standalone => sealed
EventuallyAcknowledged == <> acknowledged
=============================================================================
