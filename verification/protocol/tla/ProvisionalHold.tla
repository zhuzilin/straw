---------------------------- MODULE ProvisionalHold ----------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Failure, Mutation
ASSUME /\ Failure \in {"hold", "build", "complete"}
       /\ Mutation \in {"none", "legacy", "late_guard", "forget_error", "clean_complete", "rotation", "eager_error", "overwrite_pending"}
Names == {"A", "B"}
VARIABLES phase, current, pending, guard, owners, complete, staged, injected,
          unstable, resume, confirmed, present, source
vars == <<phase, current, pending, guard, owners, complete, staged, injected,
          unstable, resume, confirmed, present, source>>
Init == /\ phase = "begin" /\ current = "A" /\ pending = {} /\ guard = "none" /\ owners = {}
        /\ complete = FALSE /\ staged = FALSE /\ injected = FALSE /\ unstable = TRUE
        /\ resume = FALSE /\ confirmed = FALSE /\ present = TRUE /\ source = TRUE
Begin == /\ phase = "begin" /\ phase' = "hold"
         /\ pending' = IF Mutation \in {"legacy", "late_guard"} THEN pending ELSE IF Mutation = "overwrite_pending" THEN {current} ELSE pending \cup {current}
         /\ UNCHANGED <<current, guard, owners, complete, staged, injected, unstable, resume, confirmed, present, source>>
Hold == /\ phase = "hold"
        /\ IF current = "A" /\ Failure = "hold" /\ ~injected
              THEN /\ phase' = "abandoned" /\ injected' = TRUE
                   /\ \E effect \in EnvUncertainBoolean(FALSE):
                         owners' = IF effect THEN owners \cup {current} ELSE owners
                   /\ UNCHANGED <<pending, confirmed>>
              ELSE /\ phase' = "build" /\ owners' = owners \cup {current}
                   /\ pending' = IF Mutation = "late_guard" THEN pending \cup {current} ELSE pending
                   /\ confirmed' = TRUE /\ UNCHANGED injected
        /\ UNCHANGED <<current, guard, complete, staged, unstable, resume, present, source>>
Build == /\ phase = "build"
         /\ IF current = "A" /\ Failure = "build" /\ ~injected
               THEN /\ phase' = "abandoned" /\ injected' = TRUE
                    /\ pending' = IF Mutation = "eager_error" THEN pending \ {current} ELSE pending
                    /\ UNCHANGED complete
               ELSE /\ phase' = "stage" /\ complete' = TRUE
                    /\ pending' = IF Mutation = "clean_complete" THEN pending ELSE pending \ {current}
                    /\ UNCHANGED injected
         /\ owners' = IF Mutation = "eager_error" /\ current = "A" /\ Failure = "build" /\ ~injected
                        THEN owners \ {current} ELSE owners
         /\ UNCHANGED <<current, guard, staged, unstable, resume, confirmed, present, source>>
AnotherRequest == /\ phase = "abandoned" /\ current = "A"
                  /\ phase' = "begin" /\ current' = "B" /\ confirmed' = FALSE
                  /\ UNCHANGED <<pending, guard, owners, complete, staged, injected, unstable, resume, present, source>>
Close == /\ phase = "abandoned" /\ phase' = "cleanup"
         /\ UNCHANGED <<current, pending, guard, owners, complete, staged, injected, unstable, resume, confirmed, present, source>>
LostComplete == /\ phase = "stage" /\ current = "A" /\ Failure = "complete" /\ ~injected
                /\ phase' = IF Mutation = "clean_complete" THEN "cleanup" ELSE "retry"
                /\ injected' = TRUE /\ resume' = TRUE
                /\ UNCHANGED <<current, pending, guard, owners, complete, staged, unstable, confirmed, present, source>>
Retry == /\ phase = "retry" /\ phase' = "stage"
         /\ UNCHANGED <<current, pending, guard, owners, complete, staged, injected, unstable, resume, confirmed, present, source>>
Stage == /\ phase = "stage" /\ (current = "B" \/ Failure # "complete" \/ injected)
         /\ phase' = "finalize" /\ staged' = TRUE
         /\ UNCHANGED <<current, pending, guard, owners, complete, injected, unstable, resume, confirmed, present, source>>
Finalize == /\ phase = "finalize" /\ owners' = owners \ {current}
            /\ phase' = "cleanup" /\ resume' = FALSE
            /\ UNCHANGED <<current, pending, guard, complete, staged, injected, unstable, confirmed, present, source>>
Select == /\ phase = "cleanup" /\ guard = "none" /\ pending # {}
          /\ \E n \in pending: guard' = n /\ pending' = pending \ {n}
          /\ UNCHANGED <<phase, current, owners, complete, staged, injected, unstable, resume, confirmed, present, source>>
CleanupSuccess == /\ phase = "cleanup" /\ guard # "none"
                  /\ owners' = owners \ {guard} /\ guard' = "none"
                  /\ UNCHANGED <<phase, current, pending, complete, staged, injected, unstable, resume, confirmed, present, source>>
CleanupError == /\ phase = "cleanup" /\ guard # "none" /\ unstable
                /\ \E effect \in EnvUncertainBoolean(FALSE):
                      owners' = IF effect THEN owners \ {guard} ELSE owners
                /\ guard' = IF Mutation = "forget_error" THEN "none" ELSE guard
                /\ UNCHANGED <<phase, current, pending, complete, staged, injected, unstable, resume, confirmed, present, source>>
FinishCleanup == /\ phase = "cleanup" /\ guard = "none" /\ pending = {}
                 /\ phase' = IF resume THEN "stage" ELSE "done"
                 /\ UNCHANGED <<current, pending, guard, owners, complete, staged, injected, unstable, resume, confirmed, present, source>>
Rotate == /\ phase = "build" /\ Mutation = "rotation" /\ current \in pending
          /\ owners' = owners \ {current} /\ pending' = pending \ {current}
          /\ UNCHANGED <<phase, current, guard, complete, staged, injected, unstable, resume, confirmed, present, source>>
ReleaseSource == /\ source /\ confirmed /\ source' = FALSE
                 /\ UNCHANGED <<phase, current, pending, guard, owners, complete, staged, injected, unstable, resume, confirmed, present>>
GC == /\ ~source /\ owners = {} /\ ~staged /\ complete /\ present
      /\ present' = FALSE
      /\ UNCHANGED <<phase, current, pending, guard, owners, complete, staged, injected, unstable, resume, confirmed, source>>
Stop == /\ injected /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, current, pending, guard, owners, complete, staged, injected, resume, confirmed, present, source>>
Work == Begin \/ Hold \/ Build \/ AnotherRequest \/ Close \/ LostComplete \/ Retry \/ Stage \/ Finalize \/ Select \/ CleanupSuccess \/ FinishCleanup
Next == Work \/ CleanupError \/ Rotate \/ ReleaseSource \/ GC \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Work) /\ EnvFaultsEventuallyStop(vars, Stop)
TypeOK == /\ phase \in {"begin", "hold", "build", "abandoned", "stage", "retry", "finalize", "cleanup", "done"}
          /\ current \in Names /\ guard \in Names \cup {"none"} /\ owners \subseteq Names /\ pending \subseteq Names
          /\ complete \in BOOLEAN /\ staged \in BOOLEAN /\ injected \in BOOLEAN
          /\ unstable \in BOOLEAN /\ resume \in BOOLEAN /\ confirmed \in BOOLEAN
          /\ present \in BOOLEAN /\ source \in BOOLEAN
NoAbandonedHold == phase = "done" => owners = {}
TrackedOwners == owners \subseteq pending \cup (IF guard = "none" THEN {} ELSE {guard}) \cup (IF complete THEN {current} ELSE {})
CompleteProtected == complete /\ ~staged => current \in owners
PreparationProtected == phase \in {"build", "abandoned"} /\ confirmed => current \in owners
NoLostComplete == complete => present
EventuallyFinished == <> (phase = "done")
=============================================================================
