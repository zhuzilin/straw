----------------------- MODULE EnvironmentRequest -----------------------
\* A previously accepted request can be retried after its referenced data has
\* been legitimately released and collected. Its durable receipt remains.
\* No latency bound, data resurrection, or automatic checkpoint restore follows.
EnvRequestReadable(present, id) == present[id]
=============================================================================
