---------------------------- MODULE CommitApplication ----------------------------
EXTENDS Naturals, TLC, EnvironmentApplication
CONSTANTS Mutation, Cut, Retire
ASSUME /\ Mutation \in {"none", "late_poison", "clear_error", "skip_entry_guard", "skip_rebuild", "early_prune", "late_retirement"}
       /\ Cut \in EnvApplicationCuts /\ Retire \in BOOLEAN
VARIABLES s
vars == <<s>>
Init == s = [phase |-> "retain", owner |-> FALSE, staged |-> TRUE, durable |-> FALSE,
             memory |-> FALSE, poisoned |-> FALSE, present |-> TRUE, round |-> 0]
Retain == /\ s.phase = "retain" /\ s' = [s EXCEPT !.owner = TRUE, !.phase = "append"]
\* Byte write/sync uncertainty is split in DurableHandoff/WalPrefix. This model
\* starts application only after this frame and its handoff intent are durable.
AppendWal == /\ s.phase = "append" /\ s' = [s EXCEPT !.durable = TRUE, !.phase = "begin"]
Begin == /\ s.phase = "begin"
         /\ s' = [s EXCEPT !.poisoned = (Mutation # "late_poison"), !.phase = "before"]
Hook(at, next) == /\ s.phase = at
                 /\ LET response == EnvApplicationOutcome(Cut, at)
                    IN s' = [s EXCEPT !.phase = IF response = "ok" THEN next ELSE "resume",
                              !.poisoned = IF response = "error" THEN Mutation # "clear_error" ELSE @]
Apply == /\ s.phase = "apply" /\ s' = [s EXCEPT !.memory = TRUE, !.phase = "after"]
Confirm == /\ s.phase = "confirm" /\ s' = [s EXCEPT !.poisoned = FALSE, !.phase = "consume"]
Consume == /\ s.phase = "consume" /\ s' = [s EXCEPT !.staged = FALSE, !.phase = "resume"]
\* This is a later public call after success, an Err, or a caught unwind.
Resume == /\ s.phase = "resume"
          /\ s' = [s EXCEPT !.phase = IF s.poisoned /\ Mutation # "skip_entry_guard" THEN "recover" ELSE "cleanup"]
Recover == /\ s.phase = "recover"
           /\ s' = [s EXCEPT !.memory = IF Mutation = "skip_rebuild" THEN FALSE ELSE s.durable,
                            !.poisoned = FALSE, !.phase = "cleanup"]
\* The durable intent permits staging cleanup, even if old memory lacks the
\* accepted task. This is why the admission guard must precede this cleanup.
Cleanup == /\ s.phase = "cleanup" /\ s' = [s EXCEPT !.staged = FALSE, !.phase = "prune"]
Prune == /\ s.phase = "prune"
         /\ s' = [s EXCEPT !.owner = s.memory /\ Mutation # "early_prune", !.phase = "collect"]
Collect == /\ s.phase = "collect"
           /\ s' = [s EXCEPT !.present = @ /\ (s.owner \/ s.staged),
                            !.phase = IF s.round = 0 /\ Retire THEN "retire" ELSE "done"]
\* Successful retirement commits its WAL evidence before pruning owners. Full
\* task/root extraction and per-pack physical deletion are separate refinements.
Retirement == /\ s.phase = "retire"
              /\ s' = [s EXCEPT !.durable = (Mutation = "late_retirement"), !.memory = FALSE,
                               !.round = 1, !.phase = "prune"]
Next == Retain \/ AppendWal \/ Begin \/ Hook("before", "apply") \/ Apply \/ Hook("after", "confirm")
        \/ Confirm \/ Consume \/ Resume \/ Recover \/ Cleanup \/ Prune \/ Collect \/ Retirement
Spec == Init /\ [][Next]_vars
TypeOK == /\ s.phase \in {"retain", "append", "begin", "before", "apply", "after", "confirm", "consume", "resume", "recover", "cleanup", "prune", "collect", "retire", "done"}
          /\ s.owner \in BOOLEAN /\ s.staged \in BOOLEAN /\ s.durable \in BOOLEAN
          /\ s.memory \in BOOLEAN /\ s.poisoned \in BOOLEAN /\ s.present \in BOOLEAN /\ s.round \in 0..1
AcceptedDataPresent == s.durable => s.present
RetirementOrdered == s.phase = "collect" /\ ~s.owner => ~s.durable
TerminalOwnership == s.phase = "done" => (s.present = ~Retire)
=============================================================================
