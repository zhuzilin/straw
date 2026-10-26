------------------------ MODULE EnvironmentInputs ------------------------
EXTENDS Naturals
\* ENV-INPUT: finite identity sets are exploration bounds, not API quotas.
EnvCompletionRequests(submissions, digests, epochs, attempts) ==
    [submission : submissions, digest : digests, epoch : epochs, attempt : attempts]
\* ENV-PUBLICATION-RETRY: a caller can change roots under the same operation
\* identity and receive a conflict. That error cannot discard prior protection.
EnvConflictingRoots(packs, original) == (SUBSET packs) \ {original}
=============================================================================
