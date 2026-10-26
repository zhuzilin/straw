------------------------- MODULE WriterRetirement ------------------------
EXTENDS Naturals, FiniteSets, EnvironmentWriterRetirement
CONSTANTS Mutation, Policy, RequireProgress
ASSUME /\ Mutation \in {"none", "skip_fence_check", "death_unlock", "skip_registry", "release_early", "receipt_early"}
       /\ Policy \in {"recover", "abandon"}
       /\ RequireProgress \in BOOLEAN
Ops == {1,2}
VARIABLES process, gate, flight, fenced, phase, active, registered, results,
          staged, abandoned, preparation, settled, partial, sealed, receipt,
          tombstone, present, lateWrite, stable
vars == <<process,gate,flight,fenced,phase,active,registered,results,staged,
          abandoned,preparation,settled,partial,sealed,receipt,tombstone,
          present,lateWrite,stable>>
Init == /\ process = "running" /\ gate = "none" /\ flight = FALSE
        /\ fenced = FALSE /\ phase = "idle" /\ active = 0
        /\ registered = {} /\ results = {} /\ staged = {} /\ abandoned = {}
        /\ preparation = FALSE /\ settled = {} /\ partial = FALSE
        /\ sealed = FALSE /\ receipt = FALSE /\ tombstone = FALSE
        /\ present = TRUE /\ lateWrite = FALSE /\ stable = FALSE
Process == /\ ~stable /\ process' \in EnvWriterNext(process)
           /\ UNCHANGED <<gate,flight,fenced,phase,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
\* Conditional liveness: this writer eventually exits, submitted I/O finishes,
\* effects are eventually scheduled, and retirement retries eventually run.
Stabilize == /\ ~stable /\ stable' = TRUE /\ process' = "dead"
             /\ UNCHANGED <<gate,flight,fenced,phase,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite>>
AcquireWriter == /\ process = "running" /\ gate = "none"
                 /\ (~fenced \/ Mutation = "skip_fence_check")
                 /\ active' \in Ops \ registered
                 /\ gate' = "writer" /\ phase' = "register"
                 /\ UNCHANGED <<process,flight,fenced,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Register == /\ process = "running" /\ gate = "writer" /\ phase = "register"
            /\ registered' = IF Mutation = "skip_registry" THEN registered ELSE registered \cup {active}
            /\ phase' = "hold"
            /\ UNCHANGED <<process,gate,flight,fenced,active,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Hold == /\ process = "running" /\ gate = "writer" /\ phase = "hold"
        /\ preparation' = TRUE /\ phase' = "write"
        /\ UNCHANGED <<process,gate,flight,fenced,active,registered,results,staged,abandoned,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Submit == /\ process = "running" /\ gate = "writer" /\ phase = "write"
          /\ flight' = TRUE /\ partial' = TRUE /\ phase' = "describe"
          /\ UNCHANGED <<process,gate,fenced,active,registered,results,staged,abandoned,preparation,settled,sealed,receipt,tombstone,present,lateWrite,stable>>
Finish == /\ EnvMayComplete(flight) /\ flight' = FALSE
          /\ lateWrite' = (lateWrite \/ tombstone) /\ present' = TRUE
          /\ UNCHANGED <<process,gate,fenced,phase,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,stable>>
Describe == /\ process = "running" /\ gate = "writer" /\ phase = "describe" /\ ~flight
            /\ results' = results \cup {active} /\ partial' = FALSE
            /\ gate' = "none" /\ phase' = "idle"
            /\ UNCHANGED <<process,flight,fenced,active,registered,staged,abandoned,preparation,settled,sealed,receipt,tombstone,present,lateWrite,stable>>
ExitUnlock == /\ gate = "writer"
              /\ (EnvGateMayPass(process,flight) \/ (Mutation = "death_unlock" /\ process = "dead"))
              /\ gate' = "none" /\ phase' = "idle"
              /\ UNCHANGED <<process,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
AcquireRetire == /\ gate = "none" /\ gate' = "retirer" /\ phase' = "fence"
                 /\ UNCHANGED <<process,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Fence == /\ gate = "retirer" /\ phase = "fence"
         /\ fenced' = TRUE /\ phase' = "drain"
         /\ UNCHANGED <<process,gate,flight,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Drain == /\ gate = "retirer" /\ phase = "drain"
         /\ partial' = FALSE /\ phase' = "recover"
         /\ UNCHANGED <<process,gate,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,sealed,receipt,tombstone,present,lateWrite,stable>>
Recover == /\ gate = "retirer" /\ phase = "recover"
           /\ staged' = IF Policy = "recover" THEN staged \cup results ELSE staged
           /\ abandoned' = IF Policy = "abandon" THEN abandoned \cup results ELSE abandoned
           /\ phase' = "release"
           /\ UNCHANGED <<process,gate,flight,fenced,active,registered,results,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Release == /\ gate = "retirer"
           /\ (phase = "release" \/ (Mutation = "release_early" /\ phase = "recover"))
           /\ preparation' = FALSE /\ settled' = registered /\ phase' = "seal"
           /\ UNCHANGED <<process,gate,flight,fenced,active,registered,results,staged,abandoned,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Seal == /\ gate = "retirer" /\ phase = "seal"
        /\ sealed' = TRUE /\ phase' = "receipt"
        /\ UNCHANGED <<process,gate,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,partial,receipt,tombstone,present,lateWrite,stable>>
Receipt == /\ gate = "retirer"
           /\ (phase = "receipt" \/ Mutation = "receipt_early")
           /\ receipt' = TRUE /\ gate' = "none" /\ phase' = "idle"
           /\ UNCHANGED <<process,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,tombstone,present,lateWrite,stable>>
\* Errors/crashes can follow a successful durable side effect. Retry starts a
\* fresh attempt but never rolls back the fence, registry, staging or seals.
Retry == /\ gate = "retirer" /\ ~stable /\ gate' = "none" /\ phase' = "idle"
         /\ UNCHANGED <<process,flight,fenced,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,tombstone,present,lateWrite,stable>>
Collect == /\ sealed /\ staged = {} /\ ~preparation /\ ~tombstone
           /\ tombstone' = TRUE /\ present' = FALSE
           /\ UNCHANGED <<process,gate,flight,fenced,phase,active,registered,results,staged,abandoned,preparation,settled,partial,sealed,receipt,lateWrite,stable>>
Next == Process \/ Stabilize \/ AcquireWriter \/ Register \/ Hold \/ Submit \/ Finish \/ Describe \/ ExitUnlock \/ AcquireRetire \/ Fence \/ Drain \/ Recover \/ Release \/ Seal \/ Receipt \/ Retry \/ Collect
Spec == Init /\ [][Next]_vars
        /\ WF_vars(Stabilize) /\ WF_vars(Finish) /\ WF_vars(ExitUnlock)
        /\ WF_vars(AcquireRetire) /\ WF_vars(Fence) /\ WF_vars(Drain)
        /\ WF_vars(Recover) /\ WF_vars(Release) /\ WF_vars(Seal) /\ WF_vars(Receipt)
TypeOK == /\ process \in EnvWriterStates /\ gate \in {"none","writer","retirer"}
          /\ active \in 0..2 /\ registered \subseteq Ops /\ results \subseteq Ops
          /\ staged \subseteq Ops /\ abandoned \subseteq Ops /\ settled \subseteq Ops
          /\ flight \in BOOLEAN /\ fenced \in BOOLEAN /\ preparation \in BOOLEAN
          /\ partial \in BOOLEAN /\ sealed \in BOOLEAN /\ receipt \in BOOLEAN
          /\ tombstone \in BOOLEAN /\ present \in BOOLEAN /\ lateWrite \in BOOLEAN /\ stable \in BOOLEAN
RegisteredBeforeOwner == preparation => registered # {}
NoLiveWriterRetired == sealed => ~flight
NoWriteAfterRetirement == ~lateWrite
FenceStopsNewWrites == fenced => gate # "writer"
ReconciledBeforeRelease == results \subseteq staged \cup abandoned \cup (IF preparation THEN Ops ELSE {})
ReceiptSound == receipt => fenced /\ sealed /\ ~flight /\ ~partial /\ settled = registered
RequiredProgress == IF RequireProgress THEN <> receipt ELSE TRUE
=============================================================================
