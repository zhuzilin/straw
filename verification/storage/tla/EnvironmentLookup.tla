------------------------- MODULE EnvironmentLookup -------------------------
EXTENDS Naturals
\* ENV-LOOKUP: a returning read-only open yields a descriptor, NotFound, or
\* another error. NotFound need not mean permanent absence. NameBy bounds
\* local lookup attempts within each outer data attempt, not milliseconds.
EnvOpenResponses(lookup, nameBy, otherErrors) ==
    (IF lookup >= nameBy THEN {"present"} ELSE {"present", "missing"})
    \cup (IF otherErrors THEN {"error"} ELSE {})
=============================================================================
