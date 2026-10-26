--------------------------- MODULE OfflineCleanup ---------------------------
EXTENDS Naturals, EnvironmentOffline, EnvironmentProgress
CONSTANTS Catalog, Queues, RootRef, SharedRef, Stopped, ScanOK, AddShared, Mutation
ASSUME /\ Catalog \in BOOLEAN /\ Queues \in BOOLEAN /\ RootRef \in BOOLEAN
       /\ SharedRef \in BOOLEAN /\ Stopped \in BOOLEAN /\ ScanOK \in BOOLEAN
       /\ AddShared \in BOOLEAN /\ (SharedRef => Catalog \/ Queues)
       /\ Mutation \in {"none", "stat_guard", "ignore_error", "skip_recheck",
                         "ignore_queues", "ignore_stopped", "delete_retained",
                         "ignore_scan_error", "stall"}
\* One physical pack, a root journal and independent shared ownership.
\* The immutable full root closure is abstracted by RootRef/ScanOK. Each guard
\* is a stable namespace observation; marker creation is allowed between the
\* initial report and the later stopped-run cleanup, never during that cleanup.
VARIABLES phase, catalog, queues, sharedRef, candidate, deleted, changed
vars == <<phase, catalog, queues, sharedRef, candidate, deleted, changed>>
Init == /\ phase = "inspect" /\ catalog = Catalog /\ queues = Queues
        /\ sharedRef = SharedRef /\ candidate = FALSE /\ deleted = FALSE /\ changed = FALSE
Responses(present) == IF Mutation = "stat_guard" THEN EnvOfflineStat(present) ELSE EnvOfflineOpen(present)
Absent(response) == response = "absent" \/ (Mutation = "ignore_error" /\ response = "error")
GuardAllowed(a, b) == Absent(a) /\ (Mutation = "ignore_queues" \/ Absent(b))
Guard == /\ phase \in {"inspect", "check"}
         /\ ~(phase = "check" /\ Mutation = "stall")
         /\ \E a \in Responses(catalog), b \in Responses(queues):
             phase' = IF ~GuardAllowed(a, b) THEN "blocked"
                      ELSE IF phase = "inspect" THEN "scan"
                      ELSE IF candidate \/ Mutation = "delete_retained" THEN "delete" ELSE "done"
         /\ UNCHANGED <<catalog, queues, sharedRef, candidate, deleted, changed>>
Scan == /\ phase = "scan"
        /\ candidate' = (~RootRef \/ (Mutation = "ignore_scan_error" /\ ~ScanOK))
        /\ phase' = IF ScanOK \/ Mutation = "ignore_scan_error" THEN "report" ELSE "blocked"
        /\ UNCHANGED <<catalog, queues, sharedRef, deleted, changed>>
PeerBeforeCleanup == /\ phase = "report" /\ AddShared /\ ~changed
                     /\ catalog' = TRUE /\ sharedRef' = TRUE /\ changed' = TRUE
                     /\ UNCHANGED <<phase, queues, candidate, deleted>>
BeginCleanup == /\ phase = "report"
                /\ phase' = IF ~Stopped /\ Mutation # "ignore_stopped" THEN "blocked"
                            ELSE IF Mutation = "skip_recheck" THEN IF candidate THEN "delete" ELSE "done"
                            ELSE "check"
                /\ UNCHANGED <<catalog, queues, sharedRef, candidate, deleted, changed>>
Delete == /\ phase = "delete"
          /\ \E effect \in EnvOfflineDeletion:
              /\ deleted' = effect[1]
              /\ phase' = IF effect[2] THEN "done" ELSE "blocked"
          /\ UNCHANGED <<catalog, queues, sharedRef, candidate, changed>>
Next == Guard \/ Scan \/ PeerBeforeCleanup \/ BeginCleanup \/ Delete
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"inspect", "scan", "report", "check", "delete", "done", "blocked"}
          /\ catalog \in BOOLEAN /\ queues \in BOOLEAN /\ sharedRef \in BOOLEAN
          /\ candidate \in BOOLEAN /\ deleted \in BOOLEAN /\ changed \in BOOLEAN
NoOwnedDeletion == (RootRef \/ sharedRef) => ~deleted
NoLiveDeletion == deleted => Stopped
CheckedBeforeDeletion == phase = "delete" => ~catalog /\ ~queues /\ ScanOK /\ ~RootRef
Terminates == <>(phase \in {"done", "blocked"})
=============================================================================
