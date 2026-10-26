-------------------------- MODULE QueueRecordWorkflow --------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentCommitCleanup, EnvironmentReaders, EnvironmentProgress
CONSTANTS ReaderCount, History, ReaderFailures, Mutation
ASSUME /\ ReaderCount > 0 /\ History \in Nat /\ ReaderFailures \in BOOLEAN
       /\ Mutation \in {"none", "lost_intent", "cancel_drops_reads", "recover_drops_reads", "early_prune", "forget_last_ack", "forget_receipts", "cleanup_failed_reader"}
Readers == 1..ReaderCount
VARIABLES phase, intent, receipts, history, replay, running, finished, registered, acknowledged,
          cancelled, owner, present, staging, unstable
vars == <<phase, intent, receipts, history, replay, running, finished, registered, acknowledged,
          cancelled, owner, present, staging, unstable>>
Init == /\ phase = "accept" /\ intent = FALSE /\ receipts = {} /\ history = 0 /\ replay = 0
        /\ running = {} /\ finished = {} /\ registered = {} /\ acknowledged = {}
        /\ cancelled = FALSE /\ owner = TRUE /\ present = TRUE /\ staging = TRUE /\ unstable = TRUE
Accept == /\ phase = "accept" /\ phase' = "recover_pending" /\ intent' = (Mutation # "lost_intent")
          /\ UNCHANGED <<replay, receipts, history, running, finished, registered, acknowledged, cancelled, owner, present, staging, unstable>>
RecoverPending == /\ phase = "recover_pending"
                  /\ IF ~intent \/ EnvHandoffRecorded(0 \in receipts)
                     THEN /\ phase' = "history" /\ UNCHANGED <<replay, receipts, staging>>
                     ELSE /\ \E response \in EnvHandoffResponses(unstable):
                                /\ receipts' = IF response = "unchanged_error" THEN receipts ELSE receipts \cup {0}
                                /\ staging' = (response = "unchanged_error")
                                /\ phase' = IF response = "ok" THEN "history" ELSE phase
                  /\ UNCHANGED <<replay, intent, history, running, finished, registered, acknowledged, cancelled, owner, present, unstable>>
HistoryStep == /\ phase = "history" /\ history < History /\ history' = history + 1
               /\ receipts' = receipts \cup {history + 1}
               /\ UNCHANGED <<replay, phase, intent, running, finished, registered, acknowledged, cancelled, owner, present, staging, unstable>>
Lease == /\ phase = "history" /\ history = History /\ phase' = "cancel"
         /\ running' = Readers /\ registered' = Readers
         /\ UNCHANGED <<replay, intent, receipts, history, finished, acknowledged, cancelled, owner, present, staging, unstable>>
Cancel == /\ phase = "cancel" /\ phase' = "recover_readers" /\ cancelled' = TRUE
          /\ registered' = IF Mutation = "cancel_drops_reads" THEN {} ELSE registered
          /\ UNCHANGED <<replay, intent, receipts, history, running, finished, acknowledged, owner, present, staging, unstable>>
ReplayKnown == (replay = 0 /\ ~intent) \/ EnvHandoffRecorded(replay \in receipts)
RecoverReaders == /\ phase = "recover_readers" /\ replay <= History /\ ReplayKnown
                  /\ replay' = replay + 1
                  /\ phase' = IF replay = History THEN "read" ELSE phase
                  /\ registered' = IF Mutation = "recover_drops_reads" THEN {} ELSE registered
                  /\ UNCHANGED <<intent, receipts, history, running, finished, acknowledged, cancelled, owner, present, staging, unstable>>
ReadSuccess == \E r \in running:
                 /\ phase = "read" /\ present /\ running' = running \ {r} /\ finished' = finished \cup {r}
                 /\ UNCHANGED <<replay, phase, intent, receipts, history, registered, acknowledged, cancelled, owner, present, staging, unstable>>
ReadFailure == /\ phase = "read" /\ ReaderFailures /\ running # {} /\ phase' = "failed"
               /\ registered' = IF Mutation = "cleanup_failed_reader" THEN {} ELSE registered
               /\ UNCHANGED <<replay, intent, receipts, history, running, finished, acknowledged, cancelled, owner, present, staging, unstable>>
Ack == \E r \in Readers \ acknowledged:
          /\ phase = "read" /\ EnvReaderMayAcknowledge(finished, r)
          /\ acknowledged' = acknowledged \cup {r}
          /\ registered' = IF Mutation = "forget_last_ack" /\ Cardinality(registered) = 1 THEN registered ELSE registered \ {r}
          /\ UNCHANGED <<replay, phase, intent, receipts, history, running, finished, cancelled, owner, present, staging, unstable>>
WantOwner == ~cancelled \/ (registered # {} /\ Mutation # "early_prune")
Prune == /\ phase \in {"read", "failed", "collect", "done"}
         /\ owner # WantOwner /\ owner' = WantOwner
         /\ UNCHANGED <<replay, phase, intent, receipts, history, running, finished, registered, acknowledged, cancelled, present, staging, unstable>>
Erase == /\ present /\ ~owner /\ ~staging /\ present' = FALSE
         /\ UNCHANGED <<replay, phase, intent, receipts, history, running, finished, registered, acknowledged, cancelled, owner, staging, unstable>>
AfterReads == /\ phase = "read" /\ acknowledged = Readers /\ phase' = "collect"
              /\ UNCHANGED <<replay, intent, receipts, history, running, finished, registered, acknowledged, cancelled, owner, present, staging, unstable>>
AfterGC == /\ phase = "collect" /\ ~present /\ phase' = "recover_final" /\ replay' = 0
           /\ receipts' = IF Mutation = "forget_receipts" THEN receipts \ {History} ELSE receipts
           /\ UNCHANGED <<intent, history, running, finished, registered, acknowledged, cancelled, owner, present, staging, unstable>>
RecoverFinal == /\ phase = "recover_final" /\ replay <= History /\ ReplayKnown
                /\ replay' = replay + 1 /\ phase' = IF replay = History THEN "done" ELSE phase
                /\ UNCHANGED <<intent, receipts, history, running, finished, registered, acknowledged, cancelled, owner, present, staging, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<replay, phase, intent, receipts, history, running, finished, registered, acknowledged, cancelled, owner, present, staging>>
Work == Accept \/ RecoverPending \/ HistoryStep \/ Lease \/ Cancel \/ RecoverReaders \/ ReadSuccess \/ ReadFailure \/ Ack \/ Prune \/ Erase \/ AfterReads \/ AfterGC \/ RecoverFinal
Spec == Init /\ [][Work \/ Stop]_vars /\ EnvWeakFairness(vars, Work) /\ EnvFaultsEventuallyStop(vars, Stop)
TypeOK == /\ phase \in {"accept", "recover_pending", "history", "cancel", "recover_readers", "read", "failed", "collect", "recover_final", "done"}
          /\ intent \in BOOLEAN /\ receipts \subseteq 0..History /\ history \in 0..History /\ replay \in 0..(History + 1)
          /\ running \subseteq Readers /\ finished \subseteq Readers /\ registered \subseteq Readers /\ acknowledged \subseteq Readers
          /\ cancelled \in BOOLEAN /\ owner \in BOOLEAN /\ present \in BOOLEAN /\ staging \in BOOLEAN /\ unstable \in BOOLEAN
NoLostReaders == running # {} => present
FailedProtected == phase = "failed" => present
NoUnconsumedStaging == phase \notin {"accept", "recover_pending"} => ~staging
RequiredReceipts == (IF intent THEN {0} ELSE {}) \cup (1..History)
ReceiptsRecorded == phase \notin {"accept", "recover_pending", "history"} => RequiredReceipts \subseteq receipts
Done == phase = "done" => ~present /\ registered = {} /\ running = {} /\ replay = History + 1
EventuallyOutcome == <> (phase = "done" \/ phase = "failed")
=============================================================================
