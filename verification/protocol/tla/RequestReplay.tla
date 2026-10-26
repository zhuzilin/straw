---------------------------- MODULE RequestReplay ----------------------------
EXTENDS Naturals, FiniteSets, EnvironmentRequest
CONSTANTS Ids, Digests, NoReceipt, Mutation
ASSUME /\ Ids # {} /\ Digests # {} /\ NoReceipt \notin Digests
       /\ Mutation \in {"none", "validate_first", "ignore_receipt", "ignore_conflict", "forget_recovery"}
VARIABLES durable, cached, present, writes, request, phase, reply, sameAtCall
vars == <<durable, cached, present, writes, request, phase, reply, sameAtCall>>
Empty == [id \in Ids |-> NoReceipt]
Requests == [id: Ids, digest: Digests]
Init == /\ durable = Empty /\ cached = Empty
        /\ present = [id \in Ids |-> TRUE] /\ writes = [id \in Ids |-> 0]
        /\ request \in Requests /\ phase = "idle" /\ reply = "none" /\ sameAtCall = FALSE
Call(r) == /\ phase = "idle" /\ request' = r /\ reply' = "none"
           /\ sameAtCall' = (cached[r.id] = r.digest)
           /\ phase' = IF Mutation = "validate_first" THEN "preflight" ELSE "check"
           /\ UNCHANGED <<durable, cached, present, writes>>
Preflight == /\ phase = "preflight"
             /\ phase' = IF EnvRequestReadable(present, request.id) THEN "check" ELSE "done"
             /\ reply' = IF EnvRequestReadable(present, request.id) THEN "none" ELSE "unavailable"
             /\ UNCHANGED <<durable, cached, present, writes, request, sameAtCall>>
Check == /\ phase = "check"
         /\ LET known == cached[request.id] # NoReceipt /\ Mutation # "ignore_receipt"
                same == cached[request.id] = request.digest \/ Mutation = "ignore_conflict"
            IN /\ phase' = IF known THEN "done" ELSE "validate"
               /\ reply' = IF known THEN (IF same THEN "replay" ELSE "conflict") ELSE "none"
         /\ UNCHANGED <<durable, cached, present, writes, request, sameAtCall>>
Validate == /\ phase = "validate"
            /\ phase' = IF EnvRequestReadable(present, request.id) THEN "append" ELSE "done"
            /\ reply' = IF EnvRequestReadable(present, request.id) THEN "none" ELSE "unavailable"
            /\ UNCHANGED <<durable, cached, present, writes, request, sameAtCall>>
\* The lower WAL append/sync/apply is abstracted here; its refinement and
\* independently durable catalog effects are not silently assumed proved.
Accept == /\ phase = "append"
          /\ durable' = [durable EXCEPT ![request.id] = request.digest]
          /\ cached' = durable' /\ writes' = [writes EXCEPT ![request.id] = @ + 1]
          /\ phase' = "reply"
          /\ UNCHANGED <<present, request, reply, sameAtCall>>
Reply == /\ phase = "reply" /\ phase' = "done" /\ reply' = "accepted"
         /\ UNCHANGED <<durable, cached, present, writes, request, sameAtCall>>
Finish == /\ phase = "done" /\ phase' = "idle" /\ reply' = "none"
          /\ UNCHANGED <<durable, cached, present, writes, request, sameAtCall>>
Retire(id) == /\ durable[id] # NoReceipt /\ present[id]
              /\ present' = [present EXCEPT ![id] = FALSE]
              /\ UNCHANGED <<durable, cached, writes, request, phase, reply, sameAtCall>>
Crash == /\ phase # "recover" /\ phase' = "recover" /\ cached' = Empty /\ reply' = "none"
         /\ UNCHANGED <<durable, present, writes, request, sameAtCall>>
Recover == /\ phase = "recover" /\ phase' = "idle"
           /\ cached' = IF Mutation = "forget_recovery" THEN Empty ELSE durable
           /\ UNCHANGED <<durable, present, writes, request, reply, sameAtCall>>
Next == (\E r \in Requests : Call(r)) \/ Preflight \/ Check \/ Validate \/ Accept \/ Reply \/ Finish
        \/ (\E id \in Ids : Retire(id)) \/ Crash \/ Recover
Spec == Init /\ [][Next]_vars
TypeOK == /\ durable \in [Ids -> Digests \cup {NoReceipt}] /\ cached \in [Ids -> Digests \cup {NoReceipt}]
          /\ present \in [Ids -> BOOLEAN] /\ writes \in [Ids -> Nat]
          /\ request \in Requests /\ sameAtCall \in BOOLEAN
          /\ phase \in {"idle", "preflight", "check", "validate", "append", "reply", "done", "recover"}
          /\ reply \in {"none", "accepted", "replay", "conflict", "unavailable"}
OneAcceptance == \A id \in Ids : writes[id] <= 1
RecoveredReceipts == phase # "recover" => cached = durable
CachedReply == phase = "done" /\ sameAtCall => reply = "replay"
CorrectReplay == reply = "replay" => cached[request.id] = request.digest
=============================================================================
