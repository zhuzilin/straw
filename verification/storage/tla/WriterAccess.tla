------------------------------- MODULE WriterAccess -------------------------------
EXTENDS Naturals, TLC, EnvironmentLocalAccess, EnvironmentProgress
CONSTANTS Mutation, PoisonedOnRetry
ASSUME /\ Mutation \in {"none", "blocking", "mutate_busy", "ignore_poison"}
       /\ PoisonedOnRetry \in BOOLEAN
VARIABLES outer, caller, held, nested, open, published, poisoned, response
vars == <<outer, caller, held, nested, open, published, poisoned, response>>
Init == /\ outer = "writing" /\ caller = "idle" /\ held = TRUE
        /\ nested = FALSE /\ open = TRUE /\ published = FALSE
        /\ poisoned = FALSE /\ response = "none"
Callback == /\ outer = "writing" /\ outer' = "callback" /\ caller' = "request"
            /\ UNCHANGED <<held, nested, open, published, poisoned, response>>
Request == /\ outer = "callback" /\ caller = "request"
           /\ IF Mutation = "blocking"
                 THEN /\ caller' = "blocked" /\ UNCHANGED <<nested, open, response>>
                 ELSE /\ caller' = "returned"
                      /\ \E result \in EnvLocalAccess(held, poisoned):
                            /\ response' = result /\ nested' = (result = "acquired")
                            /\ open' = IF Mutation = "mutate_busy" THEN FALSE ELSE open
           /\ UNCHANGED <<outer, held, published, poisoned>>
Resume == /\ outer = "callback" /\ caller = "returned" /\ ~nested
          /\ outer' = "finishing"
          /\ UNCHANGED <<caller, held, nested, open, published, poisoned, response>>
Finish == /\ outer = "finishing" /\ open
          /\ outer' = "done" /\ held' = FALSE /\ published' = TRUE
          /\ poisoned' = PoisonedOnRetry /\ caller' = "retry"
          /\ UNCHANGED <<nested, open, response>>
Retry == /\ outer = "done" /\ caller = "retry"
         /\ \E result \in EnvLocalAccess(held, poisoned):
               /\ response' = result
               /\ nested' = (result = "acquired" \/ (Mutation = "ignore_poison" /\ result = "poisoned"))
         /\ caller' = "finalize"
         /\ UNCHANGED <<outer, held, open, published, poisoned>>
Return == /\ caller = "finalize" /\ caller' = "done" /\ nested' = FALSE
          /\ open' = IF nested THEN FALSE ELSE open
          /\ UNCHANGED <<outer, held, published, poisoned, response>>
\* A callback cannot return until its synchronous nested call returns. The
\* equivalent two-thread case has the writer waiting for that caller's signal.
Wait == /\ caller = "blocked" /\ UNCHANGED vars
Work == Callback \/ Request \/ Resume \/ Finish \/ Retry \/ Return
Next == Work \/ Wait
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Work)
TypeOK == /\ outer \in {"writing", "callback", "finishing", "done"}
          /\ caller \in {"idle", "request", "blocked", "returned", "retry", "finalize", "done"}
          /\ held \in BOOLEAN /\ nested \in BOOLEAN /\ open \in BOOLEAN
          /\ published \in BOOLEAN /\ poisoned \in BOOLEAN
          /\ response \in {"none", "busy", "poisoned", "acquired"}
ExclusiveAccess == ~(held /\ nested)
BusyPreservesWriter == outer # "done" => open
PoisonDenied == poisoned => ~nested
PublishedBeforeClose == ~open => published
EventuallyFinished == <> (caller = "done" /\ published)
=============================================================================
