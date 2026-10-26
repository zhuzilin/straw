----------------------- MODULE EnvironmentConsumption -----------------------
\* ENV-CONSUMPTION: immutable reference keys identify truthful direct edges.
\* Readers can fail; a released manifest can disappear before a retry/reopen.
\* Byte authentication, shared physical packs and journal replay are adapters.
EnvConsumptionEdges(graph, node) == graph[node]
=============================================================================
