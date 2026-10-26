------------------------- MODULE UsageCache -------------------------
EXTENDS Naturals, Integers
CONSTANTS WordMax, Mode
ASSUME /\ WordMax >= 2 /\ Mode \in {"fixed", "legacy"}
Tasks == {"a", "b"}
VARIABLES active, accepted, cache
vars == <<active, accepted, cache>>
Total(a, c) == a["a"] + a["b"] + c
Represent(n) == IF n <= WordMax THEN n ELSE -1
Update(n) == IF Mode = "fixed" THEN Represent(n) ELSE n % (WordMax + 1)
Init == /\ active = [t \in Tasks |-> 0] /\ accepted = 0 /\ cache = 0
Lease(t, amount) == /\ active[t] = 0
                   /\ active' = [active EXCEPT ![t] = amount]
                   /\ cache' = (IF cache = -1 THEN -1 ELSE Update(cache + amount))
                   /\ UNCHANGED accepted
Cancel(t) == /\ active[t] > 0
             /\ active' = [active EXCEPT ![t] = 0]
             /\ cache' = (IF cache = -1 THEN -1 ELSE cache - active[t])
             /\ UNCHANGED accepted
Complete(t, amount) == /\ active[t] > 0 /\ accepted + amount <= WordMax
                       /\ accepted' = accepted + amount
                       /\ active' = [active EXCEPT ![t] = 0]
                       /\ cache' = (IF cache = -1 THEN -1 ELSE Update(cache - active[t] + amount))
Query == /\ cache = -1 /\ cache' = Represent(Total(active, accepted))
         /\ UNCHANGED <<active, accepted>>
Acknowledge == /\ accepted > 0 /\ accepted' = 0 /\ cache' = -1
               /\ UNCHANGED active
Next == (\E t \in Tasks, amount \in {1, WordMax}: Lease(t, amount) \/ Complete(t, amount))
        \/ (\E t \in Tasks: Cancel(t)) \/ Query \/ Acknowledge
Spec == Init /\ [][Next]_vars
TypeOK == /\ active \in [Tasks -> {0, 1, WordMax}]
          /\ accepted \in 0..WordMax /\ cache \in (-1)..WordMax
CacheExactOrAbsent == cache = -1 \/ cache = Total(active, accepted)
=============================================================================
