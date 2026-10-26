------------------------- MODULE ReactivationRace -------------------------
EXTENDS Naturals, FiniteSets, EnvironmentCatalog, EnvironmentProgress
CONSTANTS Packs, Mutation, CacheContext, OwnerRootMatches
ASSUME /\ Packs # {}
       /\ CacheContext \in {"same", "pool", "run", "namespace", "root"}
       /\ OwnerRootMatches \in BOOLEAN
       /\ Mutation \in {"none", "stale_snapshot", "unlocked_commit", "missing_source", "unbound_proof", "wrong_cache", "wrong_owner_root", "drop_dependency"}
VARIABLES phase, source, present, tombstones, restored, proofMatches,
          verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound
vars == <<phase, source, present, tombstones, restored, proofMatches,
          verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
Init == /\ phase = "prepare" /\ source = Packs /\ present = Packs
        /\ tombstones = {} /\ restored = {} /\ proofMatches \in BOOLEAN
        /\ verified = FALSE /\ locked = FALSE
        /\ admittedFromSource = FALSE /\ admittedWithProof = FALSE
        /\ closure = {} /\ cacheBound = FALSE /\ ownerBound = FALSE
Protected == Packs \subseteq source /\ Packs \cap tombstones = {}
Prepare == /\ phase = "prepare"
           /\ phase' = IF Protected THEN "verify" ELSE "error"
           /\ UNCHANGED <<source, present, tombstones, restored, proofMatches,
                           verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
\* An open descriptor may still finish after unlink. Failures are also allowed.
\* Completion authenticates only the proof's immutable pool/reference identity.
Verify == /\ phase = "verify" /\ \E success \in BOOLEAN:
            /\ phase' = IF success THEN "recheck" ELSE "error"
            /\ verified' = success
            /\ closure' = IF success THEN
                  (IF Mutation = "drop_dependency" THEN {CHOOSE p \in Packs : TRUE} ELSE Packs) ELSE {}
            /\ cacheBound' = (CacheContext = "same" \/ Mutation # "wrong_cache")
          /\ UNCHANGED <<source, present, tombstones, restored, proofMatches,
                          locked, admittedFromSource, admittedWithProof, ownerBound>>
Recheck == /\ phase = "recheck"
           /\ LET guard == verified /\ (proofMatches \/ Mutation = "unbound_proof")
                            /\ (OwnerRootMatches \/ Mutation = "wrong_owner_root")
                            /\ (Mutation = "stale_snapshot" \/
                                (Packs \cap tombstones = {} /\
                                 (Packs \subseteq source \/ Mutation = "missing_source")))
              IN /\ phase' = IF guard THEN "checked" ELSE "error"
                 /\ locked' = (guard /\ Mutation # "unlocked_commit")
           /\ UNCHANGED <<source, present, tombstones, restored, proofMatches,
                           verified, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
Append == /\ phase = "checked" /\ phase' = "done" /\ restored' = closure
          /\ admittedFromSource' = (Packs \subseteq source)
          /\ admittedWithProof' = (verified /\ proofMatches /\ cacheBound)
          /\ ownerBound' = OwnerRootMatches
          /\ locked' = FALSE
          /\ UNCHANGED <<source, present, tombstones, proofMatches, verified, closure, cacheBound>>
PeerAccess == EnvCatalogAccess(IF locked THEN "scan" ELSE "idle", TRUE)
Release == /\ PeerAccess /\ \E p \in source: source' = source \ {p}
           /\ UNCHANGED <<phase, present, tombstones, restored, proofMatches,
                           verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
RetainSource == /\ PeerAccess /\ \E p \in present \ tombstones: source' = source \cup {p}
                /\ UNCHANGED <<phase, present, tombstones, restored, proofMatches,
                                verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
Mark == /\ PeerAccess /\ \E p \in present \ (source \cup restored \cup tombstones):
            tombstones' = tombstones \cup {p}
        /\ UNCHANGED <<phase, source, present, restored, proofMatches,
                        verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
Unlink == /\ PeerAccess /\ \E p \in tombstones \cap present: present' = present \ {p}
          /\ UNCHANGED <<phase, source, tombstones, restored, proofMatches,
                          verified, locked, admittedFromSource, admittedWithProof, closure, cacheBound, ownerBound>>
Controller == Prepare \/ Verify \/ Recheck \/ Append
Next == Controller \/ Release \/ RetainSource \/ Mark \/ Unlink
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Controller)
TypeOK == /\ phase \in {"prepare", "verify", "recheck", "checked", "done", "error"}
          /\ source \subseteq Packs /\ present \subseteq Packs /\ tombstones \subseteq Packs
          /\ restored \subseteq Packs /\ proofMatches \in BOOLEAN /\ verified \in BOOLEAN
          /\ closure \subseteq Packs /\ cacheBound \in BOOLEAN /\ ownerBound \in BOOLEAN
          /\ locked \in BOOLEAN /\ admittedFromSource \in BOOLEAN /\ admittedWithProof \in BOOLEAN
NoDanglingRestore == restored \subseteq present /\ restored \cap tombstones = {}
AuthorizedFromSource == phase = "done" => admittedFromSource
AuthenticatedRoot == phase = "done" => admittedWithProof
NoPartialRestore == phase # "done" => restored = {}
CompleteAuthorizedClosure == phase = "done" => restored = Packs /\ ownerBound
Terminates == <>(phase \in {"done", "error"})
\* Serial durable catalog steps, immutable identity and compliant peers are
\* assumptions. This model permits read completion after GC; no bytes escape
\* to an application and authorization is always a fresh locked decision.
=============================================================================
