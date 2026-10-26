------------------------ MODULE CrossLogComposition -------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentCrossComposition
CONSTANTS QueueCount, RequestCount, PackCount, HolderCount, GenerationMax, Mutation
ASSUME /\ QueueCount \in 1..2 /\ RequestCount \in 1..3 /\ PackCount \in 1..2
       /\ HolderCount \in 0..1 /\ GenerationMax \in 1..2
       /\ (RequestCount > 1 => QueueCount = 2)
       /\ Mutation \in {"none", "accept_without_retain", "drop_dependency", "duplicate_accept",
           "prune_before_retire", "prune_peer", "consume_restage", "mark_owned", "receipt_before_sync",
           "forget_tombstone", "ignore_conflict", "forget_recovery", "prune_offline", "ignore_epoch", "ignore_attempt", "restore_without_retain"}
Queues == 1..QueueCount
Keys == IF RequestCount = 1 THEN {<<1,1>>}
        ELSE IF RequestCount = 2 THEN {<<1,1>>,<<2,1>>} ELSE {<<1,1>>,<<2,1>>,<<1,2>>}
Packs == 1..PackCount
Holders == 1..HolderCount
Stages == IF RequestCount = 3 THEN {1,2} ELSE {1}
Recipe(t) == IF t = 1 THEN Packs ELSE {PackCount}
Source(k) == IF k[2] = 1 THEN 1 ELSE 2
VARIABLE s
vars == <<s>>
Init == s = [queues |-> [q \in Queues |-> {}], holders |-> [h \in Holders |-> {}],
             stages |-> [t \in Stages |-> 0], generation |-> [t \in Stages |-> 0],
             accepted |-> [k \in Keys |-> 0], cached |-> [k \in Keys |-> 0], count |-> [k \in Keys |-> 0],
             retired |-> {}, consumed |-> {}, ready |-> {}, present |-> Packs, durableNames |-> Packs,
             dead |-> {}, reclaimed |-> {}, oldReplaySafe |-> TRUE, retryCorrect |-> TRUE, retainComplete |-> TRUE,
             epoch |-> [q \in Queues |-> 1], attempt |-> [k \in Keys |-> 1], leaseSafe |-> TRUE]
Active(k) == s.accepted[k] # 0 /\ k \notin s.retired
Protected(p) == (\E q \in Queues : p \in s.queues[q])
              \/ (\E h \in Holders : p \in s.holders[h])
              \/ (\E t \in Stages : s.stages[t] # 0 /\ p \in Recipe(t))
Available(roots) == roots \subseteq s.present /\ roots \cap s.dead = {}
Keep(q) == UNION {Recipe(Source(k)) : k \in {key \in Keys : key[1] = q /\ Active(key)}}
Retain(q,t) == /\ q \in s.ready /\ Available(Recipe(t))
               /\ LET held == s.queues[q] \cup (IF Mutation = "drop_dependency" THEN {1} ELSE Recipe(t))
                  IN s' = [s EXCEPT !.queues[q] = held, !.retainComplete = Recipe(t) \subseteq held]
Accept(k,digest,applied,epoch,attempt) ==
    /\ k[1] \in s.ready /\ (s.accepted[k] = 0 \/ Mutation = "duplicate_accept")
    /\ (Recipe(Source(k)) \subseteq s.queues[k[1]] \/ Mutation = "accept_without_retain")
    /\ (epoch = s.epoch[k[1]] \/ Mutation = "ignore_epoch")
    /\ (attempt = s.attempt[k] \/ Mutation = "ignore_attempt")
    /\ s' = [s EXCEPT !.accepted[k] = digest, !.count[k] = @ + 1,
             !.leaseSafe = @ /\ epoch = s.epoch[k[1]] /\ attempt = s.attempt[k],
             !.cached[k] = IF applied THEN digest ELSE @,
             !.ready = IF applied THEN @ ELSE @ \ {k[1]}]
\* New admissions require the currently durable lease; a receipt replay below
\* is checked first and stays valid after that lease has expired or changed.
Revoke(k) == /\ k[1] \in s.ready /\ s.attempt[k] < 2
             /\ s' = [s EXCEPT !.attempt[k] = @ + 1]
NewEpoch(q) == /\ q \in s.ready /\ s.epoch[q] < 2
               /\ s' = [s EXCEPT !.epoch[q] = @ + 1]
Retire(k) == /\ k[1] \in s.ready /\ s.accepted[k] # 0
             /\ s' = [s EXCEPT !.retired = @ \cup {k}]
\* A checkpoint/consumer rewind makes a previously retired receipt live.
\* Its retain is the independent Retain action; this WAL step cannot precede
\* it. Source holders may release immediately after the restore commits.
Restore(k) == /\ k[1] \in s.ready /\ k \in s.retired
              /\ Available(Recipe(Source(k)))
              /\ (Recipe(Source(k)) \subseteq s.queues[k[1]] \/ Mutation = "restore_without_retain")
              /\ s' = [s EXCEPT !.retired = @ \ {k}]
Prune(q) == /\ (q \in s.ready \/ Mutation = "prune_offline")
            /\ LET target == IF Mutation = "prune_peer" THEN 3-q ELSE q
                   keep == IF Mutation = "prune_before_retire" \/
                              (Mutation = "prune_offline" /\ q \notin s.ready) THEN {} ELSE Keep(q)
               IN /\ target \in Queues
                  /\ s' = [s EXCEPT !.queues[target] = @ \cap keep]
Hold(h,t) == /\ Available(Recipe(t)) /\ s' = [s EXCEPT !.holders[h] = Recipe(t)]
Release(h) == s' = [s EXCEPT !.holders[h] = {}]
Stage(t) == /\ s.generation[t] < GenerationMax /\ Available(Recipe(t))
            /\ s' = [s EXCEPT !.generation[t] = @ + 1, !.stages[t] = s.generation[t] + 1]
Consume(k) ==
    /\ s.accepted[k] # 0 /\ (k \in s.consumed \/ Active(k))
    /\ LET remove == k \notin s.consumed \/ Mutation = "consume_restage"
           after == IF remove THEN [s.stages EXCEPT ![Source(k)] = 0] ELSE s.stages
       IN s' = [s EXCEPT !.consumed = @ \cup {k}, !.stages = after,
                !.oldReplaySafe = k \notin s.consumed \/ after = s.stages]
Mark(p) == /\ (~Protected(p) \/ Mutation = "mark_owned")
           /\ s' = [s EXCEPT !.dead = @ \cup {p}]
Unlink(p) == /\ p \in s.dead /\ s' = [s EXCEPT !.present = @ \ {p}]
DirectorySync(p) == /\ p \notin s.present
                    /\ s' = [s EXCEPT !.durableNames = EnvCompositionSynced(@,p)]
Receipt(p) == /\ p \in s.dead /\ (p \notin s.durableNames \/ Mutation = "receipt_before_sync")
              /\ s' = [s EXCEPT !.reclaimed = @ \cup {p},
                         !.dead = IF Mutation = "forget_tombstone" THEN @ \ {p} ELSE @]
CrashStorage == \E restored \in EnvCompositionRestored(s.present,s.durableNames):
                    s' = [s EXCEPT !.present = restored]
CrashQueue(q) == s' = [s EXCEPT !.ready = @ \ {q}]
RecoverQueue(q) == s' = [s EXCEPT !.ready = @ \cup {q},
    !.cached = [k \in Keys |-> IF k[1] = q THEN
        (IF Mutation = "forget_recovery" THEN 0 ELSE EnvCompositionReceipt(s.accepted,k)) ELSE s.cached[k]]]
Retry(k,digest) ==
    /\ k[1] \in s.ready
    /\ LET reply == IF s.cached[k] = 0 THEN "new" ELSE
                        IF s.cached[k] = digest \/ Mutation = "ignore_conflict" THEN "cached" ELSE "conflict"
           expected == IF s.accepted[k] = 0 THEN "new" ELSE
                        IF s.accepted[k] = digest THEN "cached" ELSE "conflict"
       IN s' = [s EXCEPT !.retryCorrect = reply = expected]
Next == (\E q \in Queues, t \in Stages: Retain(q,t))
        \/ (\E k \in Keys, d \in {1,2}, a \in BOOLEAN, e \in {1,2}, t \in {1,2}: Accept(k,d,a,e,t))
        \/ (\E k \in Keys: Retire(k) \/ Restore(k) \/ Consume(k) \/ Revoke(k))
        \/ (\E q \in Queues: Prune(q) \/ NewEpoch(q) \/ CrashQueue(q) \/ RecoverQueue(q))
        \/ (\E h \in Holders, t \in Stages: Hold(h,t)) \/ (\E h \in Holders: Release(h))
        \/ (\E t \in Stages: Stage(t))
        \/ (\E p \in Packs: Mark(p) \/ Unlink(p) \/ DirectorySync(p) \/ Receipt(p))
        \/ CrashStorage \/ (\E k \in Keys, d \in {1,2}: Retry(k,d))
Spec == Init /\ [][Next]_vars
TypeOK == /\ s.queues \in [Queues -> SUBSET Packs] /\ s.holders \in [Holders -> SUBSET Packs]
          /\ s.stages \in [Stages -> 0..GenerationMax] /\ s.generation \in [Stages -> 0..GenerationMax]
          /\ s.accepted \in [Keys -> 0..2] /\ s.cached \in [Keys -> 0..2] /\ s.count \in [Keys -> Nat]
          /\ s.retired \subseteq Keys /\ s.consumed \subseteq Keys /\ s.ready \subseteq Queues
          /\ s.present \subseteq Packs /\ s.durableNames \subseteq Packs
          /\ s.dead \subseteq Packs /\ s.reclaimed \subseteq Packs
          /\ s.epoch \in [Queues -> 1..2] /\ s.attempt \in [Keys -> 1..2] /\ s.leaseSafe \in BOOLEAN
          /\ s.oldReplaySafe \in BOOLEAN /\ s.retryCorrect \in BOOLEAN /\ s.retainComplete \in BOOLEAN
QueueCoverage == \A k \in Keys: Active(k) => Recipe(Source(k)) \subseteq s.queues[k[1]]
ProtectedPresent == \A p \in Packs: Protected(p) => p \in s.present /\ p \notin s.dead
AtMostOnce == \A k \in Keys: s.count[k] = IF s.accepted[k] = 0 THEN 0 ELSE 1
CachedConsistent == \A k \in Keys: /\ s.cached[k] # 0 => s.cached[k] = s.accepted[k]
                                   /\ k[1] \in s.ready => s.cached[k] = s.accepted[k]
ReclaimedDurable == /\ s.present \subseteq s.durableNames /\ s.reclaimed \cap s.durableNames = {}
NoLateRetain == s.reclaimed \subseteq s.dead
OldHandoffNoEffect == s.oldReplaySafe
CorrectRetry == s.retryCorrect
RetainedClosure == s.retainComplete
CurrentLeaseOnly == s.leaseSafe
=============================================================================
