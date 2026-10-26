------------------------- MODULE NamespaceIdentity -------------------------
EXTENDS Naturals, EnvironmentNamespace, EnvironmentPersistence, EnvironmentProgress
CONSTANTS WriterName, ReaderName, ReplayOwner, ReplayTombstone, Mutation
Names == {"file", "writer", "reader"}
Targets == [name \in Names |-> "file"]
Identity(name) == EnvResolvedIdentity(Targets, name)
ASSUME /\ WriterName \in Names /\ ReaderName \in Names
       /\ ReplayOwner \in BOOLEAN /\ ReplayTombstone \in BOOLEAN
       /\ ~(ReplayOwner /\ ReplayTombstone)
       /\ Mutation \in {"none", "raw_owner", "raw_pack", "raw_replay", "raw_fence", "raw_tombstone", "raw_receipt"}
\* One sealed regular file, stable in-pool aliases and atomic durable catalog
\* steps. Replay fixtures contain legacy path spellings. Real path resolution,
\* byte replay, graph closure and arbitrary crash interleavings are not composed.
VARIABLES phase, pack, owner, held, staged, present, durable, deleted, pending, receipt, lateAccepted
vars == <<phase, pack, owner, held, staged, present, durable, deleted, pending, receipt, lateAccepted>>
Init == /\ phase = IF ReplayTombstone THEN "late" ELSE IF ReplayOwner THEN "drop" ELSE "retain"
        /\ pack = IF Mutation = "raw_pack" THEN WriterName ELSE Identity(WriterName)
        /\ owner = IF ReplayOwner THEN IF Mutation = "raw_replay" THEN ReaderName ELSE Identity(ReaderName) ELSE "none"
        /\ held = ReplayOwner /\ staged = ~ReplayTombstone
        /\ present = TRUE /\ durable = TRUE /\ receipt = FALSE /\ lateAccepted = FALSE
        /\ deleted = IF ReplayTombstone THEN {IF Mutation = "raw_tombstone" THEN WriterName ELSE Identity(WriterName)} ELSE {}
        /\ pending = deleted
Retain == /\ phase = "retain" /\ phase' = "drop" /\ held' = TRUE
          /\ owner' = IF Mutation = "raw_owner" THEN ReaderName ELSE Identity(ReaderName)
          /\ UNCHANGED <<pack, staged, present, durable, deleted, pending, receipt, lateAccepted>>
DropStaging == /\ phase = "drop" /\ staged' = FALSE /\ phase' = "first_gc"
               /\ UNCHANGED <<pack, owner, held, present, durable, deleted, pending, receipt, lateAccepted>>
FirstCollect == /\ phase = "first_gc"
                /\ phase' = IF ~staged /\ pack # owner THEN "early_unlink" ELSE "release"
                /\ deleted' = IF ~staged /\ pack # owner THEN deleted \cup {pack} ELSE deleted
                /\ pending' = IF ~staged /\ pack # owner THEN pending \cup {pack} ELSE pending
                /\ UNCHANGED <<pack, owner, held, staged, present, durable, receipt, lateAccepted>>
EarlyUnlink == /\ phase = "early_unlink" /\ present' = FALSE /\ phase' = "release"
               /\ UNCHANGED <<pack, owner, held, staged, durable, deleted, pending, receipt, lateAccepted>>
Release == /\ phase = "release" /\ held' = FALSE /\ owner' = "none" /\ phase' = "mark"
           /\ UNCHANGED <<pack, staged, present, durable, deleted, pending, receipt, lateAccepted>>
Mark == /\ phase = "mark" /\ deleted' = {pack} /\ pending' = {pack} /\ phase' = "late"
        /\ UNCHANGED <<pack, owner, held, staged, present, durable, receipt, lateAccepted>>
LateRetain == /\ phase = "late" /\ phase' = "unlink"
              /\ LET key == IF Mutation = "raw_fence" THEN ReaderName ELSE Identity(ReaderName)
                 IN /\ lateAccepted' = (key \notin deleted)
                    /\ held' = (key \notin deleted)
                    /\ owner' = IF key \notin deleted THEN key ELSE "none"
              /\ UNCHANGED <<pack, staged, present, durable, deleted, pending, receipt>>
Unlink == /\ phase = "unlink" /\ pending # {} /\ present' = FALSE /\ phase' = "sync"
          /\ UNCHANGED <<pack, owner, held, staged, durable, deleted, pending, receipt, lateAccepted>>
Sync == /\ phase = "sync" /\ durable' = EnvSyncedState(present) /\ phase' = "receipt"
        /\ UNCHANGED <<pack, owner, held, staged, present, deleted, pending, receipt, lateAccepted>>
ReceiptReplay == /\ phase = "receipt" /\ receipt' = TRUE /\ phase' = "done"
                 /\ pending' = pending \ {IF Mutation = "raw_receipt" THEN WriterName ELSE Identity(WriterName)}
                 /\ UNCHANGED <<pack, owner, held, staged, present, durable, deleted, lateAccepted>>
Next == Retain \/ DropStaging \/ FirstCollect \/ EarlyUnlink \/ Release \/ Mark \/ LateRetain \/ Unlink \/ Sync \/ ReceiptReplay
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"retain", "drop", "first_gc", "early_unlink", "release", "mark", "late", "unlink", "sync", "receipt", "done"}
          /\ pack \in Names /\ owner \in Names \cup {"none"}
          /\ held \in BOOLEAN /\ staged \in BOOLEAN /\ present \in BOOLEAN /\ durable \in BOOLEAN
          /\ deleted \subseteq Names /\ pending \subseteq deleted /\ receipt \in BOOLEAN /\ lateAccepted \in BOOLEAN
NoLostOwnedData == held => present
TombstoneFencesAliases == ~lateAccepted
ReceiptRetiresIdentity == receipt => (pending = {} /\ ~durable)
EventuallyReclaimed == <> (phase = "done" /\ pending = {} /\ ~present /\ ~held)
=============================================================================
