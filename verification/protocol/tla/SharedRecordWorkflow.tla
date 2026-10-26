-------------------------- MODULE SharedRecordWorkflow --------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentConsumption, EnvironmentRead, EnvironmentProgress
CONSTANTS ReaderCount, ReaderFailures, Mutation
ASSUME /\ ReaderCount \in Nat \ {0}
       /\ Mutation \in {"none", "eager_cleanup", "early_continue", "missing_result", "close_leak", "no_recipe", "cleanup_failed_reader"}
       /\ ReaderFailures \in BOOLEAN
Readers == 1..ReaderCount
Nodes == {"payload", "manifest"}
Graph == [n \in Nodes |-> IF n = "manifest" THEN {"payload"} ELSE {}]
VARIABLES phase, owners, present, pending, recipe
vars == <<phase, owners, present, pending, recipe>>
Init == /\ phase = "prepare" /\ owners = {"source"} /\ present = {"payload"}
        /\ pending = {} /\ recipe = {}
Hold == /\ phase = "prepare" /\ owners' = owners \cup {"preparation"}
        /\ phase' = "fail"
        /\ UNCHANGED <<present, pending, recipe>>
FailBeforeComplete == /\ phase = "fail" /\ phase' = "release_source"
                      /\ owners' = IF Mutation = "eager_cleanup" THEN owners \ {"preparation"} ELSE owners
                      /\ UNCHANGED <<present, pending, recipe>>
ReleaseSource == /\ phase = "release_source" /\ owners' = owners \ {"source"}
                 /\ pending' = Readers /\ phase' = "reading"
                 /\ UNCHANGED <<present, recipe>>
ReaderOutcomes == IF ReaderFailures THEN EnvExactReadOutcomes ELSE {"complete"}
ReadResponse == /\ pending # {} /\ phase = "reading" /\ "payload" \in present
                /\ \E reader \in pending, outcome \in ReaderOutcomes:
                     /\ pending' = IF outcome = "complete" THEN pending \ {reader} ELSE pending
                     /\ phase' = IF outcome = "complete" THEN phase ELSE "failed"
                     /\ owners' = IF outcome # "complete" /\ Mutation = "cleanup_failed_reader"
                                     THEN owners \ {"preparation"} ELSE owners
                /\ UNCHANGED <<present, recipe>>
Continue == /\ phase = "reading" /\ (pending = {} \/ Mutation = "early_continue")
            /\ phase' = "retry"
            /\ UNCHANGED <<owners, present, pending, recipe>>
RetryHandoff == /\ phase = "retry" /\ "payload" \in present
                /\ owners' = (owners \ {"preparation"}) \cup (IF Mutation = "missing_result" THEN {} ELSE {"result"})
                /\ present' = present \cup {"manifest"} /\ phase' = "abandon"
                /\ UNCHANGED <<pending, recipe>>
AnotherFailure == /\ phase = "abandon" /\ owners' = owners \cup {"abandoned"}
                  /\ phase' = "close"
                  /\ UNCHANGED <<present, pending, recipe>>
Close == /\ phase = "close" /\ phase' = "consume"
         /\ owners' = IF Mutation = "close_leak" THEN owners ELSE owners \ {"abandoned"}
         /\ UNCHANGED <<present, pending, recipe>>
ConsumeLostResponse == /\ phase = "consume"
                       /\ owners' = owners \ {"result"}
                       /\ recipe' = IF Mutation = "no_recipe" THEN {} ELSE {"manifest"} \cup EnvConsumptionEdges(Graph, "manifest")
                       /\ phase' = "collect"
                       /\ UNCHANGED <<present, pending>>
Live == (IF owners = {} THEN {} ELSE {"payload"}) \cup (IF "result" \in owners THEN {"manifest"} ELSE {})
GC == /\ \E node \in present \ Live: present' = present \ {node}
      /\ UNCHANGED <<phase, owners, pending, recipe>>
CollectDone == /\ phase = "collect" /\ present = {} /\ phase' = "recover"
               /\ UNCHANGED <<owners, present, pending, recipe>>
Recover == /\ phase = "recover" /\ recipe = Nodes /\ phase' = "done"
           /\ UNCHANGED <<owners, present, pending, recipe>>
Work == Hold \/ FailBeforeComplete \/ ReleaseSource \/ Continue \/ RetryHandoff \/ AnotherFailure \/ Close \/ ConsumeLostResponse \/ CollectDone \/ Recover
Next == Work \/ ReadResponse \/ GC
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Work) /\ EnvWeakFairness(vars, ReadResponse)
        /\ EnvWeakFairness(vars, GC)
TypeOK == /\ phase \in {"prepare", "fail", "release_source", "reading", "retry", "abandon", "close", "consume", "collect", "recover", "done", "failed"}
          /\ owners \subseteq {"source", "preparation", "result", "abandoned"}
          /\ present \subseteq Nodes /\ pending \subseteq Readers /\ recipe \subseteq Nodes
NoLostReaders == pending # {} => "payload" \in present
NoLostRetryInput == phase \in {"reading", "retry"} => "payload" \in present
NoLostResult == phase \in {"abandon", "close", "consume"} => Nodes \subseteq present
NoAbandonedOwner == phase \in {"consume", "collect", "recover", "done"} => "abandoned" \notin owners
ReleasedHasRecipe == phase \in {"collect", "recover", "done"} => recipe = Nodes
Finished == phase = "done" => owners = {} /\ present = {} /\ pending = {}
FailedProtected == phase = "failed" => "preparation" \in owners /\ "payload" \in present
EventuallyOutcome == IF ReaderFailures THEN <> (phase \in {"done", "failed"}) ELSE <> (phase = "done")
=============================================================================
