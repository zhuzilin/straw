-------------------------- MODULE GCParentRepair --------------------------
EXTENDS Naturals, EnvironmentParentRepair, EnvironmentPersistence, EnvironmentProgress
CONSTANTS InitiallyPresent, InitiallyUnstable, Mutation
ASSUME /\ InitiallyPresent \in BOOLEAN /\ InitiallyUnstable \in BOOLEAN
       /\ Mutation \in {"none", "legacy", "missing_is_done", "skip_repair_sync"}
\* One sealed, permanently unowned/tombstoned name. A missing parent has never
\* held this file; existing private directory identities cannot be replaced.
VARIABLES phase, parent, durableParent, present, durablePresent, receipt, unstable
vars == <<phase, parent, durableParent, present, durablePresent, receipt, unstable>>
Init == /\ phase = "unlink" /\ parent = InitiallyPresent /\ durableParent = InitiallyPresent
        /\ present = InitiallyPresent /\ durablePresent = InitiallyPresent
        /\ receipt = FALSE /\ unstable = InitiallyUnstable
Unlink == /\ phase = "unlink" /\ present' = FALSE /\ phase' = "open"
          /\ UNCHANGED <<parent, durableParent, durablePresent, receipt, unstable>>
UnlinkFails == /\ phase = "unlink" /\ unstable /\ present' \in {present, FALSE} /\ phase' = "retry"
               /\ UNCHANGED <<parent, durableParent, durablePresent, receipt, unstable>>
Open == /\ phase = "open"
        /\ \E response \in EnvParentOpen(parent, unstable):
             phase' = CASE response = "opened" -> "sync"
                        [] response = "error" -> "retry"
                        [] Mutation = "legacy" -> "retry"
                        [] Mutation = "missing_is_done" -> "receipt"
                        [] OTHER -> "mkdir"
        /\ UNCHANGED <<parent, durableParent, present, durablePresent, receipt, unstable>>
Create == /\ phase = "mkdir" /\ parent' = EnvParentCreated(parent)
          /\ phase' = IF Mutation = "skip_repair_sync" THEN "receipt" ELSE "sync"
          /\ UNCHANGED <<durableParent, present, durablePresent, receipt, unstable>>
CreateFails == /\ phase = "mkdir" /\ unstable /\ parent' \in EnvParentAfterFailedCreate(parent)
               /\ phase' = "retry" /\ durablePresent' \in {durablePresent, FALSE}
               /\ UNCHANGED <<durableParent, present, receipt, unstable>>
\* Repair can fail after creation or some barriers. Retry may find the parent
\* present; it must still sync deletion. Durability of empty placeholders is
\* not required for absence and is not the property proved here.
Sync == /\ phase = "sync" /\ parent /\ durablePresent' = EnvSyncedState(present)
        /\ phase' = "receipt" /\ durableParent' \in {durableParent, parent}
        /\ UNCHANGED <<parent, present, receipt, unstable>>
SyncFails == /\ phase = "sync" /\ unstable /\ durablePresent' \in {durablePresent, present}
             /\ phase' = "retry" /\ UNCHANGED <<parent, durableParent, present, receipt, unstable>>
RecordReceipt == /\ phase = "receipt" /\ receipt' = TRUE /\ phase' = "done"
                 /\ UNCHANGED <<parent, durableParent, present, durablePresent, unstable>>
Retry == /\ phase = "retry" /\ phase' = "unlink"
         /\ UNCHANGED <<parent, durableParent, present, durablePresent, receipt, unstable>>
Crash == /\ unstable /\ parent' = durableParent /\ present' = durablePresent
         /\ phase' = IF receipt THEN "done" ELSE "unlink"
         /\ UNCHANGED <<durableParent, durablePresent, receipt, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, parent, durableParent, present, durablePresent, receipt>>
Next == Unlink \/ UnlinkFails \/ Open \/ Create \/ CreateFails \/ Sync \/ SyncFails \/ RecordReceipt \/ Retry \/ Crash \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Unlink) /\ EnvWeakFairness(vars, Open)
        /\ EnvWeakFairness(vars, Create) /\ EnvWeakFairness(vars, Sync)
        /\ EnvWeakFairness(vars, RecordReceipt) /\ EnvWeakFairness(vars, Retry)
TypeOK == /\ phase \in {"unlink", "open", "mkdir", "sync", "receipt", "retry", "done"}
          /\ parent \in BOOLEAN /\ durableParent \in BOOLEAN /\ present \in BOOLEAN
          /\ durablePresent \in BOOLEAN /\ receipt \in BOOLEAN /\ unstable \in BOOLEAN
NamespaceOK == /\ (present => parent) /\ (durablePresent => durableParent)
ReceiptIsSafe == receipt => ~durablePresent
NoReturnedFile == receipt => ~present
EventuallyRetired == <>[] receipt
=============================================================================
