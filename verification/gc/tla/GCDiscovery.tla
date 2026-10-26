--------------------------- MODULE GCDiscovery ---------------------------
EXTENDS Naturals, EnvironmentCatalog, EnvironmentProgress
CONSTANTS Managed, Remembered, Explicit, Mutation
ASSUME /\ Managed \in BOOLEAN /\ Remembered \in BOOLEAN /\ Explicit \in BOOLEAN
       /\ (Remembered => Managed)
       /\ Mutation \in {"none", "stat_discovery", "error_is_absence", "forget_required"}
\* A pool's managed/legacy mode is fixed before participants begin. An initial
\* fresh-open absence is trusted; later discovery is skipped once required.
\* Catalog retain is an atomic durable action here, not a byte/I/O refinement.
VARIABLES phase, remembered, observation, sourceOwner, destinationOwner, acknowledged, present
vars == <<phase, remembered, observation, sourceOwner, destinationOwner, acknowledged, present>>
Init == /\ phase = "discover" /\ remembered = Remembered /\ observation = "unqueried"
        /\ sourceOwner = TRUE /\ destinationOwner = FALSE /\ acknowledged = FALSE /\ present = TRUE
Known == /\ phase = "discover"
         /\ (Explicit \/ (remembered /\ Mutation # "forget_required"))
         /\ phase' = "retain" /\ remembered' = TRUE
         /\ UNCHANGED <<observation, sourceOwner, destinationOwner, acknowledged, present>>
Probe == /\ phase = "discover" /\ ~Explicit /\ (~remembered \/ Mutation = "forget_required")
         /\ observation' \in (IF Mutation = "stat_discovery" THEN EnvDiscoveryStat
                              ELSE EnvDiscoveryOpen(Managed, remembered))
         /\ phase' = CASE observation' = "present" -> "retain"
                         [] observation' = "error" /\ Mutation # "error_is_absence" -> "error"
                         [] OTHER -> "release"
         /\ remembered' = (remembered \/ observation' = "present")
         /\ acknowledged' = (phase' = "release")
         /\ UNCHANGED <<sourceOwner, destinationOwner, present>>
Retain == /\ phase = "retain" /\ destinationOwner' = TRUE /\ acknowledged' = TRUE
          /\ phase' = "release"
          /\ UNCHANGED <<remembered, observation, sourceOwner, present>>
ReleaseSource == /\ phase = "release" /\ sourceOwner' = FALSE /\ phase' = "gc"
                 /\ UNCHANGED <<remembered, observation, destinationOwner, acknowledged, present>>
Collect == /\ phase = "gc" /\ phase' = "done"
           /\ present' = (present /\ (~Managed \/ sourceOwner \/ destinationOwner))
           /\ UNCHANGED <<remembered, observation, sourceOwner, destinationOwner, acknowledged>>
Next == Known \/ Probe \/ Retain \/ ReleaseSource \/ Collect
Spec == Init /\ [][Next]_vars
        /\ EnvWeakFairness(vars, Known) /\ EnvWeakFairness(vars, Probe)
        /\ EnvWeakFairness(vars, Retain) /\ EnvWeakFairness(vars, ReleaseSource)
        /\ EnvWeakFairness(vars, Collect)
TypeOK == /\ phase \in {"discover", "retain", "release", "gc", "done", "error"}
          /\ observation \in {"unqueried", "present", "absent", "error"}
          /\ remembered \in BOOLEAN /\ sourceOwner \in BOOLEAN
          /\ destinationOwner \in BOOLEAN /\ acknowledged \in BOOLEAN /\ present \in BOOLEAN
AcknowledgedOwnership == Managed /\ acknowledged => destinationOwner
NoLostAcknowledgedData == acknowledged => present
KnownSkipsDiscovery == (Remembered \/ Explicit) => observation = "unqueried"
RememberPositive == observation = "present" => remembered
Settles == <> (phase \in {"done", "error"})
=============================================================================
