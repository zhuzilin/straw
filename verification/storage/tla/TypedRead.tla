------------------------------- MODULE TypedRead -------------------------------
EXTENDS Naturals, Sequences, EnvironmentTyped, EnvironmentProgress
CONSTANTS Length, AllocationFailure, Mutation
ASSUME /\ Length \in Nat /\ AllocationFailure \in BOOLEAN
       /\ Mutation \in {"none", "late_check", "empty_bypass", "skip_compare", "reject_valid", "retry_mismatch"}
Actual == [length |-> Length, shape |-> <<Length>>, dtype |-> "u8"]
Layouts == {Actual, [Actual EXCEPT !.shape = <<1, Length>>],
            [Actual EXCEPT !.dtype = "i8"], [Actual EXCEPT !.length = Length + 1]}
VARIABLES requested, start, stop, phase, observed, allocated, readPayload, error
vars == <<requested, start, stop, phase, observed, allocated, readPayload, error>>
Init == /\ requested \in Layouts /\ start \in 0..Length /\ stop \in start..Length
        /\ phase = "begin"
        /\ observed = Actual /\ allocated = FALSE /\ readPayload = FALSE /\ error = "none"
Begin == /\ phase = "begin"
         /\ phase' = IF Mutation = "empty_bypass" /\ start = stop THEN "done" ELSE "metadata"
         /\ UNCHANGED <<requested, start, stop, observed, allocated, readPayload, error>>
Metadata == /\ phase = "metadata" /\ observed' = EnvObservedLayout(Actual, requested)
            /\ phase' = IF Mutation = "late_check" THEN "payload" ELSE "compare"
            /\ UNCHANGED <<requested, start, stop, allocated, readPayload, error>>
Compare == /\ phase \in {"compare", "late_compare"}
           /\ LET match == (observed = requested \/ Mutation = "skip_compare") /\ Mutation # "reject_valid"
              IN /\ phase' = IF ~match THEN IF Mutation = "retry_mismatch" THEN "metadata" ELSE "error"
                             ELSE IF phase = "late_compare" \/ start = stop THEN "done" ELSE "payload"
                 /\ error' = IF ~match /\ Mutation # "retry_mismatch" THEN "typed" ELSE error
           /\ UNCHANGED <<requested, start, stop, observed, allocated, readPayload>>
Payload == /\ phase = "payload"
           /\ \E outcome \in IF AllocationFailure /\ start < stop THEN EnvPayloadAllocationOutcomes ELSE {"ok"}:
              /\ allocated' = (allocated \/ start < stop)
              /\ readPayload' = (readPayload \/ (start < stop /\ outcome = "ok"))
              /\ phase' = IF outcome = "error" THEN "error"
                          ELSE IF Mutation = "late_check" THEN "late_compare" ELSE "done"
              /\ error' = IF outcome = "error" THEN "allocation" ELSE error
           /\ UNCHANGED <<requested, start, stop, observed>>
Next == Begin \/ Metadata \/ Compare \/ Payload
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ requested \in Layouts /\ observed \in Layouts
          /\ start \in 0..Length /\ stop \in start..Length
          /\ phase \in {"begin", "metadata", "compare", "late_compare", "payload", "done", "error"}
          /\ allocated \in BOOLEAN /\ readPayload \in BOOLEAN
          /\ error \in {"none", "typed", "allocation"}
MismatchNoPayloadWork == requested # Actual => ~allocated /\ ~readPayload
ReturnedMatches == phase = "done" => requested = Actual
MismatchError == requested # Actual /\ phase = "error" => error = "typed"
NoSpuriousTypeError == requested = Actual => error # "typed"
EmptyNoPayload == start = stop => ~allocated /\ ~readPayload
Terminates == <>(phase \in {"done", "error"})
=============================================================================
