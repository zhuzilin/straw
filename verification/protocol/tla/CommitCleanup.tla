----------------------------- MODULE CommitCleanup -----------------------------
EXTENDS Naturals, TLC, EnvironmentCommitCleanup, EnvironmentProgress
CONSTANTS Mutation, Recover, Restage, IndependentReader, InitialCut
ASSUME /\ Mutation \in {"none", "legacy", "forget_error", "no_receipt", "reconsume", "forget_replay", "receipt_only", "early_prune"}
       /\ Recover \in BOOLEAN /\ Restage \in BOOLEAN /\ IndependentReader \in BOOLEAN
       /\ InitialCut \in {"before_release", "after_release", "success"}
VARIABLES phase, intent, pending, receipt, stage, queueOwner, present, late, unstable
vars == <<phase, intent, pending, receipt, stage, queueOwner, present, late, unstable>>
Init == /\ phase = "accept" /\ intent = FALSE /\ pending = FALSE /\ receipt = FALSE
        /\ stage = TRUE /\ queueOwner = TRUE /\ present = TRUE /\ late = FALSE /\ unstable = TRUE
Accept == /\ phase = "accept" /\ phase' = "first"
          /\ intent' = (Mutation # "legacy") /\ pending' = TRUE
          /\ UNCHANGED <<receipt, stage, queueOwner, present, late, unstable>>
First == /\ phase = "first"
         /\ phase' = IF Restage /\ InitialCut # "before_release" THEN "restage" ELSE "retry"
         /\ receipt' = (InitialCut # "before_release" /\ Mutation # "no_receipt")
         /\ stage' = (InitialCut = "before_release" \/ Mutation = "receipt_only")
         /\ pending' = (InitialCut # "success" /\ Mutation # "forget_error")
         /\ UNCHANGED <<intent, queueOwner, present, late, unstable>>
NewStaging == /\ phase = "restage" /\ phase' = "retry" /\ stage' = TRUE /\ late' = TRUE
             /\ UNCHANGED <<intent, pending, receipt, queueOwner, present, unstable>>
Retry == /\ phase = "retry" /\ phase' = "cleanup"
         /\ pending' = IF Recover THEN intent ELSE IF Mutation = "legacy" THEN FALSE ELSE pending
         /\ queueOwner' = IF Mutation = "early_prune" THEN FALSE ELSE queueOwner
         /\ UNCHANGED <<intent, receipt, stage, present, late, unstable>>
Recognized == /\ EnvHandoffRecorded(receipt)
              /\ Mutation # "reconsume"
              /\ ~(phase = "recovery" /\ Mutation = "forget_replay")
AfterCleanup == IF phase = "cleanup" THEN "adopted" ELSE "done"
Cleanup == /\ phase \in {"cleanup", "recovery"}
           /\ IF ~pending \/ Recognized
              THEN /\ phase' = AfterCleanup /\ pending' = FALSE
                   /\ UNCHANGED <<receipt, stage>>
              ELSE /\ present /\ queueOwner
                   /\ \E response \in EnvHandoffResponses(unstable):
                      /\ receipt' = IF response = "unchanged_error" THEN receipt ELSE Mutation # "no_receipt"
                      /\ stage' = IF response = "unchanged_error" \/ Mutation = "receipt_only" THEN stage ELSE FALSE
                      /\ pending' = (response # "ok" /\ Mutation # "forget_error")
                      /\ phase' = IF response = "ok" THEN AfterCleanup ELSE phase
           /\ UNCHANGED <<intent, queueOwner, present, late, unstable>>
Prune == /\ phase = "adopted" /\ phase' = "collect" /\ queueOwner' = FALSE
         /\ UNCHANGED <<intent, pending, receipt, stage, present, late, unstable>>
Collect == /\ phase = "collect" /\ phase' = "recovery"
           /\ present' = (stage \/ IndependentReader)
           /\ pending' = intent
           /\ UNCHANGED <<intent, receipt, stage, queueOwner, late, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, intent, pending, receipt, stage, queueOwner, present, late>>
Work == Accept \/ First \/ NewStaging \/ Retry \/ Cleanup \/ Prune \/ Collect
Spec == Init /\ [][Work \/ Stop]_vars
        /\ EnvWeakFairness(vars, Work) /\ EnvFaultsEventuallyStop(vars, Stop)
TypeOK == /\ phase \in {"accept", "first", "restage", "retry", "cleanup", "adopted", "collect", "recovery", "done"}
          /\ intent \in BOOLEAN /\ pending \in BOOLEAN /\ receipt \in BOOLEAN /\ stage \in BOOLEAN
          /\ queueOwner \in BOOLEAN /\ present \in BOOLEAN /\ late \in BOOLEAN /\ unstable \in BOOLEAN
CompletedConsumption == phase \in {"adopted", "collect", "recovery", "done"} => (~stage \/ late)
LateStagingPreserved == late => stage /\ present
ReaderPreserved == IndependentReader => present
PendingProtected == phase = "cleanup" /\ pending /\ ~receipt => queueOwner
EventuallyResolved == <> (phase = "done" /\ (IF late \/ IndependentReader THEN present ELSE ~present))
=============================================================================
