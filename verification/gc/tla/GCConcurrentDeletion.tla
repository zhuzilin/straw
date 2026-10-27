----------------------- MODULE GCConcurrentDeletion -----------------------
EXTENDS Naturals, FiniteSets, EnvironmentPersistence
CONSTANT Mutation
ASSUME Mutation \in {"none", "unmarked_plan", "early_receipt", "receipt_all_pending", "forget_tombstone", "retain_marked"}
\* Mark and receipt are separately locked durable catalog transactions. WAL
\* fragments are checked below this boundary. Every unlink and parent barrier
\* here can interleave with another collector or a new ownership transaction.
Packs == 1..2
Collectors == 1..2
VARIABLES phase, plan, unlinked, synced, owners, marked, pending, receipts,
          present, durable, scoped
vars == <<phase,plan,unlinked,synced,owners,marked,pending,receipts,present,durable,scoped>>
Init == /\ phase = [c \in Collectors |-> "idle"]
        /\ plan = [c \in Collectors |-> {}]
        /\ unlinked = [c \in Collectors |-> {}] /\ synced = [c \in Collectors |-> {}]
        /\ owners = {} /\ marked = {} /\ pending = {} /\ receipts = {}
        /\ present = Packs /\ durable = Packs /\ scoped = TRUE
Start(c) == /\ phase[c] = "idle"
            /\ \E selected \in (SUBSET ((Packs \ (owners \cup marked)) \cup pending)) \ {{}}:
                 /\ plan' = [plan EXCEPT ![c] = selected]
                 /\ marked' = IF Mutation = "unmarked_plan" THEN marked ELSE marked \cup selected
                 /\ pending' = pending \cup selected
            /\ phase' = [phase EXCEPT ![c] = "delete"]
            /\ UNCHANGED <<unlinked,synced,owners,receipts,present,durable,scoped>>
Unlink(c,p) == /\ phase[c] = "delete" /\ p \in plan[c] \ unlinked[c]
               /\ present' = present \ {p}
               /\ unlinked' = [unlinked EXCEPT ![c] = @ \cup {p}]
               /\ UNCHANGED <<phase,plan,synced,owners,marked,pending,receipts,durable,scoped>>
Sync(c,p) == /\ phase[c] = "delete" /\ unlinked[c] = plan[c] /\ p \in plan[c] \ synced[c]
             /\ durable' = EnvSyncedDirectory(present,durable,{p})
             /\ synced' = [synced EXCEPT ![c] = @ \cup {p}]
             /\ UNCHANGED <<phase,plan,unlinked,owners,marked,pending,receipts,present,scoped>>
Receipt(c) == /\ phase[c] = "delete"
              /\ ((unlinked[c] = plan[c] /\ synced[c] = plan[c]) \/ Mutation = "early_receipt")
              /\ LET selected == IF Mutation = "receipt_all_pending" THEN pending ELSE plan[c]
                 IN /\ receipts' = receipts \cup selected /\ pending' = pending \ selected
                    /\ scoped' = (scoped /\ selected \subseteq plan[c])
                    /\ marked' = IF Mutation = "forget_tombstone" THEN marked \ selected ELSE marked
              /\ phase' = [phase EXCEPT ![c] = "done"]
              /\ UNCHANGED <<plan,unlinked,synced,owners,present,durable>>
Retry(c) == /\ phase[c] # "idle"
            /\ phase' = [phase EXCEPT ![c] = "idle"]
            /\ plan' = [plan EXCEPT ![c] = {}]
            /\ unlinked' = [unlinked EXCEPT ![c] = {}] /\ synced' = [synced EXCEPT ![c] = {}]
            /\ UNCHANGED <<owners,marked,pending,receipts,present,durable,scoped>>
Retain(p) == /\ p \in present /\ (p \notin marked \/ Mutation = "retain_marked")
             /\ owners' = owners \cup {p}
             /\ UNCHANGED <<phase,plan,unlinked,synced,marked,pending,receipts,present,durable,scoped>>
Release(p) == /\ owners' = owners \ {p}
              /\ UNCHANGED <<phase,plan,unlinked,synced,marked,pending,receipts,present,durable,scoped>>
Crash == /\ present' \in {names \in SUBSET Packs : \A p \in Packs :
                             (p \in names) \in EnvNamesAfterFailure(p \in present,p \in durable)}
         /\ phase' = [c \in Collectors |-> "idle"] /\ plan' = [c \in Collectors |-> {}]
         /\ unlinked' = [c \in Collectors |-> {}] /\ synced' = [c \in Collectors |-> {}]
         /\ UNCHANGED <<owners,marked,pending,receipts,durable,scoped>>
Next == (\E c \in Collectors : Start(c) \/ Receipt(c) \/ Retry(c))
        \/ (\E c \in Collectors, p \in Packs : Unlink(c,p) \/ Sync(c,p))
        \/ (\E p \in Packs : Retain(p) \/ Release(p)) \/ Crash
Spec == Init /\ [][Next]_vars
TypeOK == /\ phase \in [Collectors -> {"idle","delete","done"}]
          /\ plan \in [Collectors -> SUBSET Packs] /\ unlinked \in [Collectors -> SUBSET Packs]
          /\ synced \in [Collectors -> SUBSET Packs]
          /\ owners \subseteq Packs /\ marked \subseteq Packs /\ pending \subseteq Packs
          /\ receipts \subseteq Packs /\ present \subseteq Packs /\ durable \subseteq Packs /\ scoped \in BOOLEAN
PlanFenced == \A c \in Collectors : phase[c] # "idle" => plan[c] \subseteq marked
OwnedPresent == owners \subseteq present /\ owners \cap marked = {}
ReceiptDurable == receipts \cap durable = {} /\ receipts \cap present = {}
ReceiptKeepsFence == receipts \subseteq marked
ReceiptScoped == scoped
PendingComplete == marked \ receipts \subseteq pending
=============================================================================
