------------------------ MODULE DependencyTraversal ------------------------
EXTENDS Naturals, FiniteSets, EnvironmentGraph, EnvironmentProgress
CONSTANTS Nodes, Roots, Mutation
ASSUME /\ Roots \subseteq Nodes
       /\ Mutation \in {"none", "drop_successor", "repeat_node", "incomplete_cache"}
VARIABLES graph, cached, pending, seen, visited, protected, finished, duplicate
vars == <<graph, cached, pending, seen, visited, protected, finished, duplicate>>
Reach(roots) == EnvReach(graph, roots, Cardinality(Nodes))
Init == /\ graph \in EnvGraphs(Nodes) /\ cached \in SUBSET Nodes
        /\ pending = Roots /\ seen = Roots /\ visited = {} /\ protected = {}
        /\ finished = FALSE /\ duplicate = FALSE
Expand == /\ ~finished
          /\ \E node \in pending:
              LET children == IF node \in cached THEN {} ELSE graph[node]
                  fresh == IF Mutation = "drop_successor" THEN {}
                           ELSE IF Mutation = "repeat_node" THEN children ELSE children \ seen
                  coverage == IF node \in cached /\ Mutation # "incomplete_cache"
                              THEN Reach({node}) ELSE {node}
              IN /\ pending' = (pending \ {node}) \cup fresh
                 /\ seen' = seen \cup fresh
                 /\ visited' = visited \cup {node}
                 /\ protected' = protected \cup coverage
                 /\ duplicate' = (duplicate \/ node \in visited)
          /\ UNCHANGED <<graph, cached, finished>>
Finish == /\ ~finished /\ pending = {} /\ finished' = TRUE
          /\ UNCHANGED <<graph, cached, pending, seen, visited, protected, duplicate>>
Next == Expand \/ Finish
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ graph \in EnvGraphs(Nodes) /\ cached \subseteq Nodes
          /\ pending \subseteq Nodes /\ seen \subseteq Nodes
          /\ visited \subseteq Nodes /\ protected \subseteq Nodes
          /\ finished \in BOOLEAN /\ duplicate \in BOOLEAN
CompleteClosure == finished => protected = Reach(Roots)
NoDuplicateVisit == ~duplicate
Terminates == <>finished
=============================================================================
