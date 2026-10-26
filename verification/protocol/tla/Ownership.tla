----------------------------- MODULE Ownership -----------------------------
EXTENDS Naturals, FiniteSets, EnvironmentCatalog, EnvironmentPersistence
CONSTANTS Queues, Packs, Reader, Checkpoint, Mutation
ASSUME /\ Queues # {} /\ Cardinality(Packs) >= 2
       /\ Reader # Checkpoint /\ {Reader, Checkpoint} \cap Queues = {}
       /\ Mutation \in {"none", "wal_before_retain", "drop_dependency",
                         "gc_without_lock", "unlink_before_tombstone",
                         "reader_timeout", "rewind_without_retain"}
Owners == Queues \cup {Reader, Checkpoint}
VARIABLES publication, present, durable, staged, sealed, owners, phase,
          wal, acknowledged, rewound, reader, checkpoint, gc, candidates,
          tombstones, crashed
vars == <<publication, present, durable, staged, sealed, owners, phase,
          wal, acknowledged, rewound, reader, checkpoint, gc, candidates,
          tombstones, crashed>>

\* Packs is the immutable transitive closure of a publication (in the supplied
\* model, a manifest pack and its dependency pack). Durable catalog operations
\* are atomic here; queue WAL write/sync/reply and GC scan/mark/unlink are split.
Init == /\ publication = 0 /\ present = {} /\ durable = {}
        /\ staged = {} /\ sealed = {}
        /\ owners = [o \in Owners |-> {}]
        /\ phase = [q \in Queues |-> 0]
        /\ wal = {} /\ acknowledged = {} /\ rewound = {}
        /\ reader = 0 /\ checkpoint = 0
        /\ gc = "idle" /\ candidates = {} /\ tombstones = {}
        /\ crashed = FALSE
Unlocked == EnvCatalogAccess(gc, Mutation # "gc_without_lock")
Live == UNION {owners[o]: o \in Owners} \cup staged
Retained == IF Mutation = "drop_dependency" THEN {CHOOSE p \in Packs: TRUE} ELSE Packs

Write == /\ publication = 0 /\ publication' = 1 /\ present' = Packs
         /\ UNCHANGED <<durable, staged, sealed, owners, phase, wal, acknowledged,
                         rewound, reader, checkpoint, gc, candidates, tombstones, crashed>>
Sync == /\ publication = 1 /\ publication' = 2 /\ durable' = EnvSyncedState(Packs)
        /\ UNCHANGED <<present, staged, sealed, owners, phase, wal, acknowledged,
                        rewound, reader, checkpoint, gc, candidates, tombstones, crashed>>
Stage == /\ publication = 2 /\ publication' = 3 /\ staged' = Packs
         /\ UNCHANGED <<present, durable, sealed, owners, phase, wal, acknowledged,
                         rewound, reader, checkpoint, gc, candidates, tombstones, crashed>>
Seal(p) == /\ publication = 3 /\ p \notin sealed /\ sealed' = sealed \cup {p}
           /\ UNCHANGED <<publication, present, durable, staged, owners, phase, wal,
                           acknowledged, rewound, reader, checkpoint, gc, candidates,
                           tombstones, crashed>>
Consume == /\ staged # {} /\ wal # {} /\ staged' = {}
           /\ UNCHANGED <<publication, present, durable, sealed, owners, phase, wal,
                           acknowledged, rewound, reader, checkpoint, gc, candidates,
                           tombstones, crashed>>
Adopt(q) == /\ Unlocked /\ publication = 3 /\ tombstones = {} /\ present = Packs
           /\ phase[q] = 0
           /\ phase' = [phase EXCEPT ![q] = IF Mutation = "wal_before_retain" THEN 2 ELSE 1]
           /\ owners' = (IF Mutation = "wal_before_retain" THEN owners
                        ELSE [owners EXCEPT ![q] = Retained])
           /\ UNCHANGED <<publication, present, durable, staged, sealed, wal, acknowledged,
                           rewound, reader, checkpoint, gc, candidates, tombstones, crashed>>
QueueWrite(q) == /\ phase[q] = 1 /\ phase' = [phase EXCEPT ![q] = 2]
                 /\ UNCHANGED <<publication, present, durable, staged, sealed, owners,
                                 wal, acknowledged, rewound, reader, checkpoint, gc,
                                 candidates, tombstones, crashed>>
QueueSync(q) == /\ phase[q] = 2 /\ phase' = [phase EXCEPT ![q] = 3]
               /\ wal' = EnvSyncedState(wal \cup {q})
               /\ UNCHANGED <<publication, present, durable, staged, sealed, owners,
                               acknowledged, rewound, reader, checkpoint, gc,
                               candidates, tombstones, crashed>>
Reply(q) == /\ phase[q] = 3 /\ phase' = [phase EXCEPT ![q] = 4]
            /\ acknowledged' = acknowledged \cup {q}
            /\ UNCHANGED <<publication, present, durable, staged, sealed, owners, wal,
                            rewound, reader, checkpoint, gc, candidates, tombstones, crashed>>
Retire(q) == /\ Unlocked /\ phase[q] \in {3, 4}
             /\ phase' = [phase EXCEPT ![q] = 5]
             /\ owners' = [owners EXCEPT ![q] = {}]
             /\ wal' = wal \ {q} /\ acknowledged' = acknowledged \ {q}
             /\ UNCHANGED <<publication, present, durable, staged, sealed, rewound,
                             reader, checkpoint, gc, candidates, tombstones, crashed>>
Rewind(q) == /\ Unlocked /\ phase[q] = 5 /\ q \notin rewound
             /\ checkpoint = 1 /\ present = Packs /\ tombstones = {}
             /\ rewound' = rewound \cup {q}
             /\ phase' = [phase EXCEPT ![q] = IF Mutation = "rewind_without_retain" THEN 3 ELSE 1]
             /\ owners' = (IF Mutation = "rewind_without_retain" THEN owners
                          ELSE [owners EXCEPT ![q] = Packs])
             /\ wal' = (IF Mutation = "rewind_without_retain" THEN wal \cup {q} ELSE wal)
             /\ UNCHANGED <<publication, present, durable, staged, sealed, acknowledged,
                             reader, checkpoint, gc, candidates, tombstones, crashed>>
Pin == /\ Unlocked /\ publication = 3 /\ present = Packs /\ tombstones = {} /\ reader = 0
       /\ reader' = 1 /\ owners' = [owners EXCEPT ![Reader] = Packs]
       /\ UNCHANGED <<publication, present, durable, staged, sealed, phase, wal,
                       acknowledged, rewound, checkpoint, gc, candidates, tombstones, crashed>>
Unpin == /\ Unlocked /\ reader = 1
         /\ reader' = (IF Mutation = "reader_timeout" THEN 1 ELSE 2)
         /\ owners' = [owners EXCEPT ![Reader] = {}]
         /\ UNCHANGED <<publication, present, durable, staged, sealed, phase, wal,
                         acknowledged, rewound, checkpoint, gc, candidates, tombstones, crashed>>
SaveCheckpoint == /\ Unlocked /\ publication = 3 /\ present = Packs /\ tombstones = {}
                  /\ checkpoint = 0 /\ checkpoint' = 1
                  /\ owners' = [owners EXCEPT ![Checkpoint] = Packs]
                  /\ UNCHANGED <<publication, present, durable, staged, sealed, phase, wal,
                                  acknowledged, rewound, reader, gc, candidates, tombstones, crashed>>
DropCheckpoint == /\ Unlocked /\ checkpoint = 1 /\ checkpoint' = 2
                  /\ owners' = [owners EXCEPT ![Checkpoint] = {}]
                  /\ UNCHANGED <<publication, present, durable, staged, sealed, phase, wal,
                                  acknowledged, rewound, reader, gc, candidates, tombstones, crashed>>
Scan == /\ gc = "idle" /\ sealed \ (Live \cup tombstones) # {}
        /\ candidates' = sealed \ (Live \cup tombstones) /\ gc' = "scan"
        /\ UNCHANGED <<publication, present, durable, staged, sealed, owners, phase, wal,
                        acknowledged, rewound, reader, checkpoint, tombstones, crashed>>
Mark == /\ gc = "scan" /\ gc' = "marked"
        /\ tombstones' = (IF Mutation = "unlink_before_tombstone" THEN tombstones
                         ELSE tombstones \cup candidates)
        /\ present' = (IF Mutation = "unlink_before_tombstone" THEN present \ candidates ELSE present)
        /\ UNCHANGED <<publication, durable, staged, sealed, owners, phase, wal,
                        acknowledged, rewound, reader, checkpoint, candidates, crashed>>
Unlink == /\ gc = "marked" /\ gc' = "idle" /\ candidates' = {}
          /\ present' = present \ tombstones
          /\ UNCHANGED <<publication, durable, staged, sealed, owners, phase, wal,
                          acknowledged, rewound, reader, checkpoint, tombstones, crashed>>
Crash == /\ ~crashed /\ crashed' = TRUE
         /\ phase' = [q \in Queues |-> IF phase[q] \in {1, 2} THEN 5 ELSE phase[q]]
         /\ owners' = [o \in Owners |-> IF o \in Queues THEN
                          (IF phase[o] \in {1, 2} THEN {} ELSE owners[o]) ELSE owners[o]]
         /\ gc' = (IF gc = "scan" THEN "idle" ELSE gc)
         /\ candidates' = (IF gc = "scan" THEN {} ELSE candidates)
         /\ UNCHANGED <<publication, present, durable, staged, sealed, wal,
                         acknowledged, rewound, reader, checkpoint, tombstones>>
Next == Write \/ Sync \/ Stage \/ Consume \/ Pin \/ Unpin \/ SaveCheckpoint \/ DropCheckpoint
        \/ Scan \/ Mark \/ Unlink \/ Crash \/ (\E p \in Packs: Seal(p))
        \/ (\E q \in Queues: Adopt(q) \/ QueueWrite(q) \/ QueueSync(q) \/ Reply(q) \/ Retire(q) \/ Rewind(q))
Spec == Init /\ [][Next]_vars
TypeOK == /\ publication \in 0..3 /\ present \subseteq Packs /\ durable \subseteq Packs
          /\ staged \subseteq Packs /\ sealed \subseteq Packs
          /\ owners \in [Owners -> SUBSET Packs] /\ phase \in [Queues -> 0..5]
          /\ wal \subseteq Queues /\ acknowledged \subseteq Queues /\ rewound \subseteq Queues
          /\ reader \in 0..2 /\ checkpoint \in 0..2 /\ crashed \in BOOLEAN
          /\ gc \in {"idle", "scan", "marked"} /\ candidates \subseteq Packs
          /\ tombstones \subseteq Packs
QueueProtection == \A q \in Queues: phase[q] \in {3, 4} => owners[q] = Packs
ReaderProtection == reader = 1 => owners[Reader] = Packs
CheckpointProtection == checkpoint = 1 => owners[Checkpoint] = Packs
NoDanglingReferences == Live \subseteq (present \cap durable) \ tombstones
DeleteAfterTombstone == (publication > 0 => Packs \ present \subseteq tombstones)
NoOpenPackDeletion == tombstones \subseteq sealed
AcknowledgedDurable == acknowledged \subseteq wal
=============================================================================
