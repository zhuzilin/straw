------------------------ MODULE EnvironmentInputs ------------------------
EXTENDS Naturals
\* ENV-INPUT: filters can contain duplicates and be empty. MaxFilter and the
\* finite identity sets are TLC exploration bounds, not public API quotas.
EnvFilterInputs(tasks, maxFilter) == UNION {[1..n -> tasks] : n \in 0..maxFilter}
EnvCompletionRequests(submissions, digests, epochs, attempts) ==
    [submission : submissions, digest : digests, epoch : epochs, attempt : attempts]
\* ENV-STREAM: providers may return short/long traversals, empty chunks,
\* changed advertised lengths and callback errors. Bounds only limit TLC.
EnvStreamLengths(maxBytes) == 0..maxBytes
\* ENV-PUBLICATION-RETRY: a caller can change roots under the same operation
\* identity and receive a conflict. That error cannot discard prior protection.
EnvConflictingRoots(packs, original) == (SUBSET packs) \ {original}
=============================================================================
