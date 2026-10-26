------------------------- MODULE EnvironmentGraph -------------------------
EXTENDS Naturals, FiniteSets
\* ENV-GRAPH: finite immutable references; expansion returns all successors.
\* A cache entry denotes a complete transitive closure, not one local path.
EnvGraphs(nodes) == [nodes -> SUBSET nodes]
RECURSIVE EnvReach(_, _, _)
EnvReach(graph, roots, depth) ==
    IF depth = 0 THEN roots
    ELSE LET previous == EnvReach(graph, roots, depth - 1)
         IN previous \cup UNION {graph[n] : n \in previous}
=============================================================================
