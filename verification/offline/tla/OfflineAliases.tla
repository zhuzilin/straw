--------------------------- MODULE OfflineAliases ---------------------------
EXTENDS Naturals, FiniteSets, EnvironmentBindings, EnvironmentNamespace, EnvironmentOffline, EnvironmentProgress
CONSTANTS Mode, CanonicalRoot, ResolutionOK, Mutation
ASSUME /\ Mode \in {"file", "chain", "directory", "backtrack"}
       /\ CanonicalRoot \in BOOLEAN /\ ResolutionOK \in BOOLEAN
       /\ Mutation \in {"none", "raw_names", "final_only", "ancestors_only",
                         "partial_unconditional", "unlink_target", "ignore_error", "stall"}
\* A finite stopped single-queue fixture. "partial" denotes a file or directory
\* alias depending on Mode. "detour" is traversed then left through '..'.
Names == {"pack", "alias", "writer", "partial", "detour", "orphan", "orphanAlias", "spare", "dangling", "missing"}
Targets == [n \in Names |-> CASE n = "alias" -> "pack"
                           [] n = "partial" -> CASE Mode = "chain" -> "pack"
                                                    [] Mode = "directory" -> "writer"
                                                    [] OTHER -> "detour"
                           [] n = "orphanAlias" -> "spare"
                           [] n = "dangling" -> "missing"
                           [] OTHER -> n]
Required == {"pack", "alias", "writer"} \cup
            (IF Mode = "file" THEN {} ELSE {"partial"}) \cup
            (IF Mode = "backtrack" THEN {"detour"} ELSE {})
Bindings == [r \in {"alias", "pack"} |-> IF r = "alias" THEN Required ELSE {"pack", "writer"}]
Roots == {"alias"} \cup (IF CanonicalRoot THEN {"pack"} ELSE {})
Candidates == {"pack", "orphan", "orphanAlias", "dangling"} \cup (IF Mode = "file" THEN {} ELSE {"partial"})
Identity(n) == EnvResolvedIdentity(Targets, n)
Protected == CASE Mutation = "raw_names" -> Roots
                  [] Mutation = "final_only" -> {Identity(r) : r \in Roots}
                  [] Mutation = "ancestors_only" -> {"pack", "writer"}
                  [] OTHER -> UNION {EnvReferenceBindings(Bindings, r) : r \in Roots}
Deletable == {n \in Candidates : (n \notin Protected /\ Identity(n) \notin Protected)
                  \/ (Mutation = "partial_unconditional" /\ n = "partial")}
VARIABLES phase, pending, live
vars == <<phase, pending, live>>
Init == /\ phase = "scan" /\ pending = {} /\ live = Names \ {"missing"}
Scan == /\ phase = "scan"
        /\ pending' = IF ResolutionOK THEN Deletable ELSE Candidates
        /\ phase' = IF ResolutionOK \/ Mutation = "ignore_error" THEN "delete" ELSE "blocked"
        /\ UNCHANGED live
Delete == /\ phase = "delete" /\ pending # {} /\ Mutation # "stall"
          /\ \E n \in pending, effect \in EnvOfflineDeletion:
              /\ live' = IF effect[1] THEN live \ {IF Mutation = "unlink_target" THEN Identity(n) ELSE n} ELSE live
              /\ pending' = pending \ {n}
              /\ phase' = IF effect[2] THEN "delete" ELSE "blocked"
Finish == /\ phase = "delete" /\ pending = {} /\ phase' = "done"
          /\ UNCHANGED <<pending, live>>
Next == Scan \/ Delete \/ Finish
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"scan", "delete", "blocked", "done"}
          /\ pending \subseteq Candidates /\ live \subseteq Names
AcceptedReadable == Required \subseteq live
NoTargetSubstitution == "spare" \in live
NoUnknownDeletion == ~ResolutionOK => live = Names \ {"missing"}
SuccessfulCleanup == phase = "done" => {"orphan", "orphanAlias", "dangling"} \cap live = {}
Terminates == <>(phase \in {"done", "blocked"})
=============================================================================
