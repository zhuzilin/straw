-------------------------- MODULE PublicationRetry --------------------------
EXTENDS Naturals, FiniteSets, EnvironmentInputs, EnvironmentProgress
CONSTANTS Packs, Original, EmptyRetry, Mutation
ASSUME /\ Original # {} /\ Original \subseteq Packs /\ EmptyRetry \in BOOLEAN
       /\ Mutation \in {"none", "replace_hold", "release_on_error", "release_before_stage"}
\* Start after a durable internal hold and an interrupted publication. Its
\* original source has released; other retry roots still have their sources.
\* Catalog actions are atomic/durable. These are pack closures, not JSON refs.
VARIABLES phase, held, staged, present, tombstones, incoming
vars == <<phase, held, staged, present, tombstones, incoming>>
Init == /\ phase = "retry" /\ held = Original /\ staged = {}
        /\ present = Packs /\ tombstones = {}
        /\ incoming \in (IF EmptyRetry THEN {{}} ELSE EnvConflictingRoots(Packs, Original))
Sources == Packs \ Original
RetryPins == /\ phase = "retry" /\ incoming \subseteq present
             /\ held' = IF Mutation = "replace_hold" THEN incoming ELSE held \cup incoming
             /\ phase' = "reject"
             /\ UNCHANGED <<staged, present, tombstones, incoming>>
Reject == /\ phase = "reject" /\ phase' = "resume"
          /\ held' = IF Mutation = "release_on_error" THEN {} ELSE held
          /\ UNCHANGED <<staged, present, tombstones, incoming>>
Resume == /\ phase = "resume" /\ Original \subseteq present
          /\ held' = held \cup Original /\ phase' = "stage"
          /\ UNCHANGED <<staged, present, tombstones, incoming>>
Stage == /\ phase = "stage"
         /\ IF Mutation = "release_before_stage"
               THEN /\ held' = {} /\ staged' = staged /\ phase' = "window"
               ELSE /\ held' = held /\ staged' = Original /\ phase' = "release"
         /\ UNCHANGED <<present, tombstones, incoming>>
StageLate == /\ phase = "window" /\ Original \subseteq present
             /\ staged' = Original /\ phase' = "release"
             /\ UNCHANGED <<held, present, tombstones, incoming>>
Release == /\ phase = "release" /\ held' = {} /\ phase' = "done"
           /\ UNCHANGED <<staged, present, tombstones, incoming>>
Collect(p) == /\ p \in present \ (held \cup staged \cup Sources)
              /\ present' = present \ {p} /\ tombstones' = tombstones \cup {p}
              /\ UNCHANGED <<phase, held, staged, incoming>>
Next == RetryPins \/ Reject \/ Resume \/ Stage \/ StageLate \/ Release \/ (\E p \in Packs: Collect(p))
Spec == Init /\ [][Next]_vars
        /\ EnvWeakFairness(vars, RetryPins) /\ EnvWeakFairness(vars, Reject)
        /\ EnvWeakFairness(vars, Resume) /\ EnvWeakFairness(vars, Stage)
        /\ EnvWeakFairness(vars, StageLate) /\ EnvWeakFairness(vars, Release)
TypeOK == /\ phase \in {"retry", "reject", "resume", "stage", "window", "release", "done"}
          /\ held \subseteq Packs /\ staged \subseteq Packs /\ present \subseteq Packs
          /\ tombstones \subseteq Packs /\ incoming \subseteq Packs
PriorHoldPreserved == phase \in {"reject", "resume"} => Original \subseteq held
NoLostDependency == Original \subseteq present
CompletedHandoff == phase = "done" => (held = {} /\ staged = Original)
EventuallyResolved == <> (phase = "done")
=============================================================================
