--------------------------- MODULE OfflineRoots ---------------------------
EXTENDS Naturals, FiniteSets, EnvironmentGraph, EnvironmentBindings, EnvironmentProgress
CONSTANTS Depth, Cycle, ScanOK, ValidationOK, DeleteOK, Mutation
ASSUME /\ Depth \in Nat \ {0} /\ Depth < 10
       /\ Cycle \in BOOLEAN /\ ScanOK \in BOOLEAN
       /\ ValidationOK \in BOOLEAN /\ DeleteOK \in BOOLEAN
       /\ Mutation \in {"none", "omit_identity", "omit_journal", "drop_child",
                         "forget_binding", "ignore_scan_error", "ignore_validation",
                         "skip_validation", "delete_during_scan", "recursive_limit", "stall"}
Identity == Depth + 1
Journal == Depth + 2
Nodes == 0..Journal
AllRoots == {1, Identity, Journal}
Roots == AllRoots \ (CASE Mutation = "omit_identity" -> {Identity}
                        [] Mutation = "omit_journal" -> {Journal}
                        [] OTHER -> {})
Graph == [n \in Nodes |-> IF n \in 1..Depth THEN
                           IF n < Depth THEN {n + 1} ELSE IF Cycle THEN {1} ELSE {}
                         ELSE {}]
Bindings == [n \in Nodes |-> {n, n + 100}]
Reach == EnvReach(Graph, AllRoots, Cardinality(Nodes))
Required == UNION {Bindings[n] : n \in Reach}
Candidates == UNION {Bindings[n] : n \in Nodes}
VARIABLES phase, pending, seen, protected, deleted, validated, reason
vars == <<phase, pending, seen, protected, deleted, validated, reason>>
Init == /\ phase = "scan" /\ pending = Roots /\ seen = {}
        /\ protected = {} /\ deleted = {} /\ validated = FALSE /\ reason = "none"
Expand == /\ phase = "scan" /\ pending # {}
          /\ \E n \in pending:
              /\ seen' = seen \cup {n}
              /\ pending' = (pending \ {n}) \cup
                     (IF Mutation = "drop_child" THEN {} ELSE Graph[n] \ (seen \cup {n}))
              /\ protected' = protected \cup
                     (IF Mutation = "forget_binding" THEN {n} ELSE EnvReferenceBindings(Bindings, n))
              /\ phase' = IF Mutation = "recursive_limit" /\ n = 3 THEN "blocked" ELSE phase
              /\ reason' = IF Mutation = "recursive_limit" /\ n = 3 THEN "stack" ELSE reason
          /\ UNCHANGED <<deleted, validated>>
ScanEnd == /\ phase = "scan" /\ pending = {}
           /\ phase' = IF ~ScanOK /\ Mutation # "ignore_scan_error" THEN "blocked"
                       ELSE IF Mutation = "skip_validation" THEN "delete" ELSE "validate"
           /\ reason' = IF ~ScanOK /\ Mutation # "ignore_scan_error" THEN "scan" ELSE reason
           /\ UNCHANGED <<pending, seen, protected, deleted, validated>>
Validate == /\ phase = "validate"
            /\ validated' = ValidationOK
            /\ phase' = IF ValidationOK \/ Mutation = "ignore_validation" THEN "delete" ELSE "blocked"
            /\ reason' = IF ~ValidationOK THEN "invalid" ELSE reason
            /\ UNCHANGED <<pending, seen, protected, deleted>>
Delete == /\ (phase = "delete" \/ (phase = "scan" /\ Mutation = "delete_during_scan"))
          /\ Mutation # "stall"
          /\ \E n \in Candidates \ (protected \cup deleted):
              /\ deleted' = deleted \cup {n}
              /\ phase' = IF DeleteOK THEN phase ELSE "blocked"
              /\ reason' = IF DeleteOK THEN reason ELSE "delete"
          /\ UNCHANGED <<pending, seen, protected, validated>>
Finish == /\ phase = "delete" /\ Candidates \ (protected \cup deleted) = {}
          /\ phase' = "done" /\ UNCHANGED <<pending, seen, protected, deleted, validated, reason>>
Next == Expand \/ ScanEnd \/ Validate \/ Delete \/ Finish
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"scan", "validate", "delete", "blocked", "done"}
          /\ pending \subseteq Nodes /\ seen \subseteq Nodes
          /\ protected \subseteq Candidates /\ deleted \subseteq Candidates
          /\ validated \in BOOLEAN /\ reason \in {"none", "stack", "scan", "invalid", "delete"}
RecoveryReadable == (Bindings[Identity] \cup Bindings[Journal]) \cap deleted = {}
DataReadable == UNION {Bindings[n] : n \in EnvReach(Graph, {1}, Cardinality(Nodes))} \cap deleted = {}
ValidatedBeforeDeletion == deleted # {} => validated /\ ScanOK
NoArtificialDepthLimit == reason # "stack"
NoRepeatedExpansion == pending \cap seen = {}
CompleteSuccessfulCleanup == phase = "done" => Candidates \ Required \subseteq deleted
Terminates == <>(phase \in {"done", "blocked"})
=============================================================================
