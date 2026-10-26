-------------------------- MODULE EnvironmentWal --------------------------
EXTENDS Naturals, Sequences
\* ENV-WAL-PREFIX: an append is serialized. After a crash any prefix between
\* the last successful barrier and the current append may remain. A failed
\* barrier may have persisted its effects. No earlier confirmed unit is lost.
\* One unit abstracts a header, payload or trailer; this is not a byte decoder.
EnvWalRecovered(durable, written) ==
    {SubSeq(written, 1, n) : n \in Len(durable)..Len(written)}
EnvWalSynced(written) == written
\* Project a single uncertain append after its durable prefix. Incomplete
\* header/body prefixes preserve the old business state; all three units
\* may expose the pending state even when the original reply was lost.
EnvWalProjectedStates(before, pending, part) ==
    {IF Len(prefix) = 3 THEN pending ELSE before :
        prefix \in EnvWalRecovered(<<>>, SubSeq(<<1, 2, 3>>, 1, part))}
\* Authentication/sequence validation is truthful. Distinct tags stand for
\* complete frame contents; tag 0 represents a complete authentication failure.
EnvWalFrameValid(written, offset) ==
    \A i \in 1..3 : written[offset + i] = offset + i
=============================================================================
