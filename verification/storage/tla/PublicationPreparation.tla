---------------------- MODULE PublicationPreparation ----------------------
EXTENDS Naturals, EnvironmentPreparation, EnvironmentPersistence, EnvironmentProgress
CONSTANTS InitiallyUnstable, Mutation
ASSUME /\ InitiallyUnstable \in BOOLEAN
       /\ Mutation \in {"none", "legacy", "early_seal", "forget_after_error"}
\* One fresh private name. Guard installation precedes catalog registration.
\* Parent repair/barriers and catalog steps are abstract effects; actual path
\* resolution, error classification and the successful write handoff are not
\* refined here. The client survives modeled storage crashes.
VARIABLES phase, guard, registered, present, durablePresent, sealed, unstable
vars == <<phase, guard, registered, present, durablePresent, sealed, unstable>>
Init == /\ phase = "register" /\ guard = (Mutation # "legacy")
        /\ registered = FALSE /\ present = FALSE /\ durablePresent = FALSE
        /\ sealed = FALSE /\ unstable = InitiallyUnstable
Register == /\ phase = "register" /\ registered' = TRUE /\ phase' = "open"
            /\ UNCHANGED <<guard, present, durablePresent, sealed, unstable>>
RegisterFails == /\ phase = "register" /\ unstable
                 /\ registered' \in EnvPreparationFailure(registered)
                 /\ phase' = IF guard THEN "unlink" ELSE "done"
                 /\ UNCHANGED <<guard, present, durablePresent, sealed, unstable>>
Open == /\ phase = "open" /\ present' = TRUE /\ durablePresent' \in BOOLEAN
        /\ phase' = "offset" /\ UNCHANGED <<guard, registered, sealed, unstable>>
OpenFails == /\ phase = "open" /\ unstable
             /\ present' \in EnvPreparationFailure(present) /\ durablePresent' \in {FALSE, present'}
             /\ phase' = IF guard THEN "unlink" ELSE "done"
             /\ UNCHANGED <<guard, registered, sealed, unstable>>
Capture == /\ phase = "offset" /\ phase' = "ready"
           /\ UNCHANGED <<guard, registered, present, durablePresent, sealed, unstable>>
OffsetFails == /\ phase = "offset" /\ unstable
               /\ phase' = IF guard THEN "unlink" ELSE "done"
               /\ UNCHANGED <<guard, registered, present, durablePresent, sealed, unstable>>
Unlink == /\ phase = "unlink" /\ present' = EnvRemovedPrivate
          /\ phase' = IF Mutation = "early_seal" THEN "seal" ELSE "parent"
          /\ UNCHANGED <<guard, registered, durablePresent, sealed, unstable>>
ParentBarrier == /\ phase = "parent" /\ durablePresent' = EnvSyncedState(present)
                 /\ phase' = "seal" /\ UNCHANGED <<guard, registered, present, sealed, unstable>>
Seal == /\ phase = "seal" /\ sealed' = TRUE /\ registered' = TRUE /\ phase' = "finish"
        /\ UNCHANGED <<guard, present, durablePresent, unstable>>
CleanupFails == /\ phase \in {"unlink", "parent", "seal"} /\ unstable
                /\ present' \in {present, FALSE}
                /\ durablePresent' \in {durablePresent, present'}
                /\ sealed' \in IF phase = "seal" THEN {sealed, TRUE} ELSE {sealed}
                /\ registered' = IF sealed' THEN TRUE ELSE registered
                /\ phase' = IF Mutation = "forget_after_error" THEN "done" ELSE "retry"
                /\ guard' = (Mutation # "forget_after_error") /\ UNCHANGED unstable
Retry == /\ phase = "retry" /\ phase' = "unlink"
         /\ UNCHANGED <<guard, registered, present, durablePresent, sealed, unstable>>
Finish == /\ phase = "finish" /\ guard' = FALSE /\ phase' = "done"
          /\ UNCHANGED <<registered, present, durablePresent, sealed, unstable>>
StorageCrash == /\ unstable /\ phase \in {"unlink", "parent", "seal", "retry"}
                /\ present' = durablePresent /\ phase' = "unlink"
                /\ UNCHANGED <<guard, registered, durablePresent, sealed, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, guard, registered, present, durablePresent, sealed>>
Next == Register \/ RegisterFails \/ Open \/ OpenFails \/ Capture \/ OffsetFails
        \/ Unlink \/ ParentBarrier \/ Seal \/ CleanupFails \/ Retry \/ Finish \/ StorageCrash \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Register) /\ EnvWeakFairness(vars, Open) /\ EnvWeakFairness(vars, Capture)
        /\ EnvWeakFairness(vars, Unlink) /\ EnvWeakFairness(vars, ParentBarrier)
        /\ EnvWeakFairness(vars, Seal) /\ EnvWeakFairness(vars, Retry) /\ EnvWeakFairness(vars, Finish)
TypeOK == /\ phase \in {"register", "open", "offset", "ready", "unlink", "parent", "seal", "finish", "retry", "done"}
          /\ guard \in BOOLEAN /\ registered \in BOOLEAN /\ present \in BOOLEAN
          /\ durablePresent \in BOOLEAN /\ sealed \in BOOLEAN /\ unstable \in BOOLEAN
NoLostReservation == phase = "done" => (~registered \/ sealed)
NoAbandonedPrivateFile == phase = "done" => ~present /\ ~durablePresent
NoEarlySeal == sealed => ~durablePresent
CleanupIsGuarded == phase \in {"unlink", "parent", "seal", "retry", "finish"} => guard
EventuallyResolved == <> (phase \in {"ready", "done"})
=============================================================================
