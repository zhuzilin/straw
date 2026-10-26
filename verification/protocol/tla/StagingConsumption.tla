-------------------------- MODULE StagingConsumption --------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentConsumption, EnvironmentProgress
CONSTANTS Depth, Mutation
ASSUME /\ Depth \in Nat \ {0}
       /\ Mutation \in {"none", "legacy", "missing_node", "forget_replay", "skip_cached_children"}
Nodes == 1..(2 * Depth)
Root == 1
Leaf == 2 * Depth
Graph == [n \in Nodes |-> IF n <= Depth
          THEN {Depth + n} \cup (IF n < Depth THEN {n + 1} ELSE {}) ELSE {}]
Empty == [n \in {} |-> {}]
Closed(graph) == \A n \in DOMAIN graph: graph[n] \subseteq DOMAIN graph
VARIABLES phase, owners, present, cache, disk, todo, seen, pending,
          first, acknowledged, restaged, unstable
vars == <<phase, owners, present, cache, disk, todo, seen, pending,
          first, acknowledged, restaged, unstable>>
Init == /\ phase = "resolve" /\ owners = Nodes /\ present = Nodes
        /\ cache = Empty /\ disk = Empty /\ pending = Empty
        /\ todo = {Root} /\ seen = {} /\ first = TRUE
        /\ acknowledged = FALSE /\ restaged = FALSE /\ unstable = TRUE
Expand(n, children, fresh) ==
    /\ todo' = (todo \ {n}) \cup (children \ (seen \cup {n}))
    /\ seen' = seen \cup {n}
    /\ pending' = IF fresh THEN pending @@ (n :> children) ELSE pending
    /\ UNCHANGED <<phase, owners, present, cache, disk, first, acknowledged, restaged, unstable>>
Cached(n) == /\ phase = "resolve" /\ n \in todo /\ n \in DOMAIN cache
             /\ Expand(n, IF Mutation = "skip_cached_children" THEN {} ELSE cache[n], FALSE)
Read(n) == /\ phase = "resolve" /\ n \in todo \ DOMAIN cache /\ n \in present
           /\ Expand(n, EnvConsumptionEdges(Graph, n), TRUE)
ReadError == /\ phase = "resolve" /\ todo # {}
             /\ (unstable \/ \E n \in todo: n \notin DOMAIN cache /\ n \notin present)
             /\ phase' = "retry"
             /\ UNCHANGED <<owners, present, cache, disk, todo, seen, pending,
                            first, acknowledged, restaged, unstable>>
Commit == /\ phase = "resolve" /\ todo = {}
          /\ owners' = owners \ seen
          /\ disk' = CASE Mutation = "legacy" -> disk
                      [] Mutation = "missing_node" ->
                            [n \in (DOMAIN (cache @@ pending)) \ {Leaf} |-> (cache @@ pending)[n]]
                      [] OTHER -> cache @@ pending
          /\ cache' = disk'
          \* One forced lost response after an otherwise complete first frame.
          /\ phase' = IF first THEN "collect" ELSE "done"
          /\ acknowledged' = ~first
          /\ UNCHANGED <<present, todo, seen, pending, first, restaged, unstable>>
Collect == /\ phase = "collect"
           \* An independent reader keeps Leaf; other unowned nodes disappear.
           /\ present' = present \cap (owners \cup {Leaf})
           /\ first' = FALSE /\ phase' = "retry"
           /\ UNCHANGED <<owners, cache, disk, todo, seen, pending, acknowledged, restaged, unstable>>
Retry == /\ phase = "retry" /\ phase' = "resolve"
         /\ cache' = IF Mutation = "forget_replay" THEN Empty ELSE disk
         /\ todo' = {Root} /\ seen' = {} /\ pending' = Empty
         /\ UNCHANGED <<owners, present, disk, first, acknowledged, restaged, unstable>>
Restage == /\ ~first /\ ~acknowledged /\ ~restaged
           /\ owners' = owners \cup {Leaf} /\ restaged' = TRUE
           /\ UNCHANGED <<phase, present, cache, disk, todo, seen, pending, first, acknowledged, unstable>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, owners, present, cache, disk, todo, seen, pending, first, acknowledged, restaged>>
Resolve == \E n \in Nodes: Cached(n) \/ Read(n)
Next == Resolve \/ ReadError \/ Commit \/ Collect \/ Retry \/ Restage \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvFaultsEventuallyStop(vars, Stop)
        /\ EnvWeakFairness(vars, Resolve) /\ EnvWeakFairness(vars, Commit)
        /\ EnvWeakFairness(vars, ReadError) /\ EnvWeakFairness(vars, Collect)
        /\ EnvWeakFairness(vars, Retry)
TypeOK == /\ phase \in {"resolve", "retry", "collect", "done"}
          /\ owners \subseteq Nodes /\ present \subseteq Nodes
          /\ todo \subseteq Nodes /\ seen \subseteq Nodes
          /\ first \in BOOLEAN /\ acknowledged \in BOOLEAN
          /\ restaged \in BOOLEAN /\ unstable \in BOOLEAN
GraphClosed == Closed(disk)
GraphExact == \A n \in DOMAIN disk: disk[n] = Graph[n]
ReaderPreserved == Leaf \in present
ConsumeComplete == acknowledged => owners = {}
ReleasedHasRecipe == ~first => Root \in DOMAIN disk
EventuallyReturned == <> acknowledged
=============================================================================
