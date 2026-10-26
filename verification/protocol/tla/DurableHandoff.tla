-------------------------- MODULE DurableHandoff --------------------------
EXTENDS Naturals, FiniteSets, EnvironmentCrossLog, EnvironmentProgress, EnvironmentWal
CONSTANTS PackCount, Actors, PermanentActors, Mutation
ASSUME /\ PackCount \in 1..2 /\ Actors \subseteq {"reader", "checkpoint", "peer"}
       /\ PermanentActors \subseteq Actors
       /\ Mutation \in {"none", "queue_before_retain", "drop_dependency",
           "catalog_reply_before_sync", "catalog_skip_recovery_sync",
           "queue_reply_before_sync", "queue_skip_recovery_sync",
           "consume_before_accept_sync", "prune_before_retire_sync",
           "unlink_before_mark_sync", "receipt_before_dirsync",
           "skip_barrier_when_missing", "forget_tombstone", "ignore_fence", "stall_retry"}
Packs == 1..PackCount
Owners == Actors \cup {"queue", "stage"}
InitialCatalog == [holds |-> [o \in Owners |-> IF o = "stage" THEN Packs ELSE {}],
                   dead |-> {}, reclaimed |-> {}, consumed |-> FALSE]
NoOp == [kind |-> "none", owner |-> "queue", pack |-> 1]
VARIABLE s
vars == <<s>>
\* Entry boundary: immutable payload/closure is published, durable and sealed,
\* with a staging owner. Writer reservation recovery is a separate obligation.
Init == s = [c |-> InitialCatalog, d |-> InitialCatalog, pending |-> InitialCatalog,
             cp |-> "idle", part |-> 0, op |-> NoOp,
             q |-> 0, qd |-> 0, qp |-> "retain", qpart |-> 0, acknowledged |-> FALSE,
             actors |-> [a \in Actors |-> "new"], present |-> Packs, durableNames |-> Packs,
             gp |-> "scan", gpack |-> 1, unstable |-> TRUE]
Available == s.cp = "idle"
Unowned(p) == \A o \in Owners : p \notin s.c.holds[o]
Target(kind, owner, p) ==
    CASE kind = "retain" -> [s.c EXCEPT !.holds[owner] =
                                IF Mutation = "drop_dependency" /\ owner = "queue" THEN {1} ELSE Packs]
      [] kind = "release" -> [s.c EXCEPT !.holds[owner] = {}]
      [] kind = "consume" -> [s.c EXCEPT !.holds["stage"] = {}, !.consumed = TRUE]
      [] kind = "mark" -> [s.c EXCEPT !.dead = @ \cup {p}]
      [] kind = "receipt" -> [s.c EXCEPT !.reclaimed = @ \cup {p},
                                !.dead = IF Mutation = "forget_tombstone" THEN @ \ {p} ELSE @]
Start(kind, owner, p) ==
    /\ Available
    /\ s' = [s EXCEPT !.cp = "write", !.part = 0,
              !.pending = Target(kind, owner, p), !.op = [kind |-> kind, owner |-> owner, pack |-> p]]
QueueRetain == /\ s.qp = "retain" /\ Available
               /\ IF "queue" \in {o \in Owners : s.c.holds[o] = Packs} \/ Mutation = "queue_before_retain"
                  THEN s' = [s EXCEPT !.qp = "write_accept", !.qpart = 0]
                  ELSE /\ s.c.dead = {} /\ Start("retain", "queue", 1)
Consume == /\ s.qp = "handoff" /\ Available
           /\ IF s.c.consumed THEN s' = [s EXCEPT !.qp = "reply"]
              ELSE /\ s.c.holds["queue"] = Packs /\ Start("consume", "stage", 1)
Prune == /\ s.qp = "prune" /\ Available
         /\ IF s.c.holds["queue"] = {} THEN s' = [s EXCEPT !.qp = "finished"]
            ELSE Start("release", "queue", 1)
ActorRetain(a) == /\ a \in Actors /\ s.actors[a] = "new" /\ Available
                  /\ IF s.c.dead # {} /\ Mutation # "ignore_fence"
                     THEN s' = [s EXCEPT !.actors[a] = "denied"]
                     ELSE Start("retain", a, 1)
ActorFinished(a) == /\ a \in Actors /\ EnvCrossMayFinish(s.actors[a], a, PermanentActors)
                    /\ s' = [s EXCEPT !.actors[a] = "release"]
ActorRelease(a) == /\ a \in Actors /\ s.actors[a] = "release" /\ Available
                   /\ IF s.c.holds[a] = {} THEN s' = [s EXCEPT !.actors[a] = "done"]
                      ELSE Start("release", a, 1)
CatalogWrite == /\ s.cp = "write" /\ s.part < 3
                /\ s' = [s EXCEPT !.part = @ + 1,
                   !.c = IF s.part = 2 THEN s.pending ELSE @,
                   !.cp = IF s.part < 2 THEN "write"
                          ELSE IF Mutation = "catalog_reply_before_sync" THEN "reply" ELSE "sync"]
CatalogSync == /\ s.cp = "sync"
               /\ s' = [s EXCEPT !.d = EnvCrossCatalogSynced(s.c), !.cp = "reply"]
CatalogReply ==
    /\ s.cp = "reply"
    /\ LET kind == s.op.kind
           owner == s.op.owner
       IN s' = [s EXCEPT !.cp = "idle", !.part = 0, !.pending = InitialCatalog, !.op = NoOp,
             !.qp = IF kind = "retain" /\ owner = "queue" THEN "write_accept"
                    ELSE IF kind = "consume" THEN "reply"
                    ELSE IF kind = "release" /\ owner = "queue" THEN "finished" ELSE @,
             !.qpart = IF kind = "retain" /\ owner = "queue" THEN 0 ELSE @,
             !.actors = IF owner \in Actors THEN
                           [@ EXCEPT ![owner] = IF kind = "retain" THEN "active" ELSE "done"] ELSE @,
             !.gp = IF kind = "mark" THEN "unlink" ELSE IF kind = "receipt" THEN "scan" ELSE @,
             !.gpack = IF kind \in {"mark", "receipt"} THEN s.op.pack ELSE @]
CatalogRecover == /\ s.cp = "recover" /\ Mutation # "stall_retry"
                  /\ s' = [s EXCEPT !.cp = "idle",
                           !.d = IF Mutation = "catalog_skip_recovery_sync" THEN @ ELSE EnvCrossCatalogSynced(s.c)]
QueueWrite == /\ s.qp \in {"write_accept", "write_retire"} /\ s.qpart < 3
              /\ LET accepting == s.qp = "write_accept"
                 IN s' = [s EXCEPT !.qpart = @ + 1,
                      !.q = IF s.qpart = 2 THEN IF accepting THEN 1 ELSE 2 ELSE @,
                      !.qp = IF s.qpart < 2 THEN @
                             ELSE IF accepting THEN
                               IF Mutation = "queue_reply_before_sync" THEN "reply"
                               ELSE IF Mutation = "consume_before_accept_sync" THEN "handoff" ELSE "sync_accept"
                             ELSE IF Mutation = "prune_before_retire_sync" THEN "prune" ELSE "sync_retire"]
QueueSync == /\ s.qp \in {"sync_accept", "sync_retire"}
             /\ s' = [s EXCEPT !.qd = EnvCrossQueueSynced(s.q), !.qpart = 0,
                             !.qp = IF s.qp = "sync_accept" THEN "handoff" ELSE "prune"]
QueueReply == /\ s.qp = "reply" /\ s' = [s EXCEPT !.acknowledged = TRUE, !.qp = "active"]
Retire == /\ s.qp = "active" /\ s' = [s EXCEPT !.qp = "write_retire", !.qpart = 0]
QueueRecover == /\ s.qp = "recover" /\ Mutation # "stall_retry"
                /\ s' = [s EXCEPT !.qd = IF Mutation = "queue_skip_recovery_sync" THEN @ ELSE EnvCrossQueueSynced(s.q),
                                 !.qp = IF s.q = 0 THEN "retain" ELSE IF s.q = 1 THEN "handoff" ELSE "prune"]
GCScan == /\ s.gp = "scan" /\ Available
          /\ IF s.c.reclaimed = Packs THEN s' = [s EXCEPT !.gp = "done"]
             ELSE \E p \in Packs \ s.c.reclaimed:
                  /\ Unowned(p)
                  /\ IF p \in s.c.dead \/ Mutation = "unlink_before_mark_sync"
                     THEN s' = [s EXCEPT !.gp = "unlink", !.gpack = p]
                     ELSE Start("mark", "queue", p)
Unlink == /\ s.gp = "unlink" /\ Available
          /\ s' = [s EXCEPT !.present = @ \ {s.gpack},
                   !.gp = IF Mutation = "skip_barrier_when_missing" /\ s.gpack \notin s.present
                          THEN "receipt" ELSE "barrier"]
DirectorySync == /\ s.gp = "barrier" /\ Available
                 /\ s' = [s EXCEPT !.gp = "receipt", !.durableNames =
                    IF Mutation = "receipt_before_dirsync" THEN @ ELSE EnvCrossDirectorySynced(@, s.gpack)]
Receipt == /\ s.gp = "receipt" /\ Start("receipt", "queue", s.gpack)
\* Failed calls may already have complete or durable effects. They never
\* authorize reuse of an uncertain append without a new recovery barrier.
CatalogFail == /\ s.unstable /\ s.cp \in {"write", "sync", "reply"}
               /\ \E d \in (IF s.cp = "sync" THEN {s.d, s.c} ELSE {s.d}):
                    s' = [s EXCEPT !.d = d, !.cp = "recover", !.part = 0,
                                   !.pending = InitialCatalog, !.op = NoOp]
QueueFail == /\ s.unstable /\ s.qp \in {"write_accept", "sync_accept", "reply", "write_retire", "sync_retire"}
             /\ \E d \in (IF s.qp \in {"sync_accept", "sync_retire"} THEN {s.qd, s.q} ELSE {s.qd}):
                  s' = [s EXCEPT !.qd = d, !.qp = "recover", !.qpart = 0]
UnlinkFail == /\ s.unstable /\ s.gp = "unlink" /\ Available
              /\ \E names \in {s.present, s.present \ {s.gpack}}:
                   s' = [s EXCEPT !.present = names, !.gp = "scan"]
DirectoryFail == /\ s.unstable /\ s.gp = "barrier" /\ Available
                 /\ \E names \in {s.durableNames, EnvCrossDirectorySynced(s.durableNames, s.gpack)}:
                      s' = [s EXCEPT !.durableNames = names, !.gp = "scan"]
Crash == /\ s.unstable
         /\ \E c \in EnvCrossCatalogRecovered(s.d, s.c, InitialCatalog),
               q \in EnvCrossQueueRecovered(s.qd, s.q), names \in {s.present, s.durableNames}:
              s' = [s EXCEPT !.c = c, !.cp = "recover", !.part = 0, !.pending = InitialCatalog, !.op = NoOp,
                             !.q = q, !.qp = "recover", !.qpart = 0, !.present = names, !.gp = "scan"]
Stop == /\ s.unstable /\ s' = [s EXCEPT !.unstable = FALSE]
Work == QueueRetain \/ Consume \/ Prune \/ CatalogWrite \/ CatalogSync \/ CatalogReply \/ CatalogRecover
        \/ QueueWrite \/ QueueSync \/ QueueReply \/ Retire \/ QueueRecover
        \/ GCScan \/ Unlink \/ DirectorySync \/ Receipt
        \/ (\E a \in Actors : ActorRetain(a) \/ ActorFinished(a) \/ ActorRelease(a))
Next == Work \/ CatalogFail \/ QueueFail \/ UnlinkFail \/ DirectoryFail \/ Crash \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop) /\ EnvWeakFairness(vars, Work)
CatalogType == [holds : [Owners -> SUBSET Packs], dead : SUBSET Packs, reclaimed : SUBSET Packs, consumed : BOOLEAN]
TypeOK == /\ s.c \in CatalogType /\ s.d \in CatalogType /\ s.pending \in CatalogType
          /\ s.cp \in {"idle", "write", "sync", "reply", "recover"} /\ s.part \in 0..3
          /\ s.op \in [kind : {"none", "retain", "release", "consume", "mark", "receipt"}, owner : Owners, pack : Packs]
          /\ s.q \in 0..2 /\ s.qd \in 0..2 /\ s.qpart \in 0..3
          /\ s.qp \in {"retain", "write_accept", "sync_accept", "handoff", "reply", "active", "write_retire", "sync_retire", "prune", "finished", "recover"}
          /\ s.actors \in [Actors -> {"new", "active", "release", "done", "denied"}]
          /\ s.present \subseteq Packs /\ s.durableNames \subseteq Packs
          /\ s.gp \in {"scan", "unlink", "barrier", "receipt", "done"} /\ s.gpack \in Packs
          /\ s.acknowledged \in BOOLEAN /\ s.unstable \in BOOLEAN
AcceptedProtected == s.qd = 1 =>
                      (s.d.holds["queue"] = Packs /\ Packs \subseteq s.present /\ Packs \subseteq s.durableNames)
ReadersProtected == \A a \in Actors : s.actors[a] = "active" =>
                     (s.d.holds[a] = Packs /\ Packs \subseteq s.present /\ Packs \subseteq s.durableNames)
AcknowledgedSurvives == s.acknowledged => (s.q >= 1 /\ s.qd >= 1)
ReplyAfterCleanup == s.acknowledged => s.d.consumed
StageReleaseOrdered == s.d.consumed => s.qd >= 1
DeletedAfterMark == Packs \ s.present \subseteq s.d.dead
NoOwnedDeletion == \A p \in Packs \ s.present : \A o \in Owners : p \notin s.d.holds[o]
RetainFenced == \A p \in s.c.dead : \A o \in Owners : p \notin s.c.holds[o]
ReceiptDurable == s.d.reclaimed \subseteq (Packs \ s.durableNames) \cap s.d.dead
CatalogReadyDurable == Available => s.c = s.d
Resolved == s.qp = "finished" /\ (\A a \in Actors : s.actors[a] \in {"done", "denied"})
            /\ s.gp = "done" /\ s.present = {} /\ s.durableNames = {} /\ s.d.reclaimed = Packs
\* Global reclamation requires application owners to finish. Independently,
\* every permanently unowned pack must be reclaimed, even in a workload that
\* permits a permanent reader. Safety never requires owners to finish.
NoOwners(p) == \A o \in Owners : p \notin s.c.holds[o] /\ p \notin s.d.holds[o]
EventuallyReclaimed == /\ (PermanentActors = {} => <> Resolved)
                       /\ \A p \in Packs : (<>[]NoOwners(p)) => <> (p \in s.d.reclaimed)
=============================================================================
