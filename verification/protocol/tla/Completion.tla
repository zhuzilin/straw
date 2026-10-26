---------------------------- MODULE Completion ----------------------------
EXTENDS Naturals, Sequences, FiniteSets, EnvironmentInputs, EnvironmentClock, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Submissions, Digests, Mutation, NoReceipt
ASSUME /\ Submissions # {} /\ Digests # {}
       /\ Mutation \in {"none", "lease_before_receipt", "ignore_lease", "ack_before_sync", "lose_recovery", "ignore_conflict"}
Requests == EnvCompletionRequests(Submissions, Digests, 0..1, 1..2)
Empty == NoReceipt
VARIABLES epoch, attempt, expired, task, phase, request, written, durable,
          applied, acknowledged, reply, admitted, crashed
vars == <<epoch, attempt, expired, task, phase, request, written, durable,
          applied, acknowledged, reply, admitted, crashed>>
Init == /\ epoch = 0 /\ attempt = 0 /\ expired = FALSE /\ task = "pending"
        /\ phase = "idle" /\ request = Empty /\ written = Empty /\ durable = Empty
        /\ applied = Empty /\ acknowledged = {} /\ reply = Empty /\ admitted = FALSE /\ crashed = FALSE
Current(r) == task = "leased" /\ r.epoch = epoch /\ r.attempt = attempt /\ ~expired
Existing(r) == applied # Empty /\ applied.submission = r.submission
Same(r) == Existing(r) /\ applied = r
Acquire == /\ phase = "idle" /\ task = "pending" /\ attempt < 2
           /\ task' = "leased" /\ attempt' = attempt + 1 /\ expired' = FALSE
           /\ UNCHANGED <<epoch, phase, request, written, durable, applied, acknowledged, reply, admitted, crashed>>
Expire == /\ phase = "idle"
          /\ EnvLeaseExpires(task = "leased", expired, expired')
          /\ UNCHANGED <<epoch, attempt, task, phase, request, written, durable, applied, acknowledged, reply, admitted, crashed>>
Reassign == /\ phase = "idle" /\ task = "leased" /\ expired /\ attempt < 2
            /\ task' = "pending"
            /\ UNCHANGED <<epoch, attempt, expired, phase, request, written, durable, applied, acknowledged, reply, admitted, crashed>>
Call(r) == /\ phase = "idle" /\ request' = r /\ phase' = "check" /\ reply' = Empty /\ admitted' = FALSE
           /\ UNCHANGED <<epoch, attempt, expired, task, written, durable, applied, acknowledged, crashed>>
Check == /\ phase = "check"
         /\ LET receiptFirst == Mutation # "lease_before_receipt" \/ Current(request)
                replay == receiptFirst /\ Existing(request)
                match == Same(request) \/ Mutation = "ignore_conflict"
                valid == Current(request) \/ Mutation = "ignore_lease"
            IN /\ reply' = IF replay THEN (IF match THEN "replay" ELSE "conflict")
                           ELSE IF valid THEN Empty ELSE "stale"
               /\ phase' = IF ~replay /\ valid THEN "write" ELSE "idle"
               /\ admitted' = (~replay /\ valid)
         /\ UNCHANGED <<epoch, attempt, expired, task, request, written, durable, applied, acknowledged, crashed>>
Write == /\ phase = "write" /\ written' = request
         /\ phase' = IF Mutation = "ack_before_sync" THEN "reply" ELSE "sync"
         /\ UNCHANGED <<epoch, attempt, expired, task, request, durable, applied, acknowledged, reply, admitted, crashed>>
Sync == /\ phase = "sync" /\ durable' = EnvSyncedState(written) /\ phase' = "apply"
        /\ UNCHANGED <<epoch, attempt, expired, task, request, written, applied, acknowledged, reply, admitted, crashed>>
Apply == /\ phase = "apply" /\ applied' = durable /\ task' = "completed" /\ phase' = "reply"
         /\ UNCHANGED <<epoch, attempt, expired, request, written, durable, acknowledged, reply, admitted, crashed>>
Reply == /\ phase = "reply" /\ reply' = "accepted" /\ phase' = "idle"
         /\ acknowledged' = acknowledged \cup {request}
         /\ UNCHANGED <<epoch, attempt, expired, task, request, written, durable, applied, admitted, crashed>>
Crash == /\ ~crashed /\ epoch = 0
         /\ phase' = "recover" /\ crashed' = TRUE /\ reply' = Empty
         /\ UNCHANGED <<epoch, attempt, expired, task, request, written, durable, applied, acknowledged, admitted>>
\* A complete frame whose sync/reply was interrupted can survive. Recovery
\* either keeps that frame and syncs it, or keeps the previous durable prefix.
Recover == /\ phase = "recover"
           /\ durable' \in (IF Mutation = "lose_recovery" THEN {Empty}
                            ELSE EnvRecoveredFrames(durable, written, Empty))
           /\ applied' = durable' /\ written' = durable' /\ epoch' = 1
           /\ task' = IF durable' = Empty THEN "pending" ELSE "completed"
           /\ expired' = FALSE /\ phase' = "idle" /\ admitted' = FALSE
           /\ UNCHANGED <<attempt, request, acknowledged, reply, crashed>>
Next == Acquire \/ Expire \/ Reassign \/ (\E r \in Requests : Call(r))
        \/ Check \/ Write \/ Sync \/ Apply \/ Reply \/ Crash \/ Recover
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ epoch \in 0..1 /\ attempt \in 0..2 /\ expired \in BOOLEAN
          /\ task \in {"pending", "leased", "completed"}
          /\ phase \in {"idle", "check", "write", "sync", "apply", "reply", "recover"}
          /\ {request, written, durable, applied} \subseteq Requests \cup {Empty}
          /\ acknowledged \subseteq Requests /\ admitted \in BOOLEAN /\ crashed \in BOOLEAN
          /\ reply \in {Empty, "accepted", "replay", "conflict", "stale"}
AcknowledgedDurable == acknowledged \subseteq {durable}
UniqueTaskCompletion == Cardinality(acknowledged) <= 1
OnlyCurrentAdmission == phase = "write" /\ admitted => Current(request)
ReplayAfterRecovery == reply = "stale" => ~Same(request)
NoConflictingReplay == reply = "replay" => Same(request)
ReturnOrRecover == (phase # "idle") ~> (phase = "idle")
=============================================================================
