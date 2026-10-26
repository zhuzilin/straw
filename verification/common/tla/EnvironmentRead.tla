-------------------------- MODULE EnvironmentRead --------------------------
EXTENDS Naturals
\* ENV-READ: a modeled read_exact returns all requested bytes, a short read,
\* or another I/O error. An error may have consumed a prefix; that attempt is
\* discarded before authorization. Real syscall termination is not proved.
\* A successful non-EOF refill may return any positive prefix within both the
\* requested capacity and the fixed snapshot remainder. Errors/EOF are separate.
EnvReadChunkSizes(remaining, capacity) == 1..(IF remaining < capacity THEN remaining ELSE capacity)
EnvExactReadOutcomes == {"complete", "short", "error"}
\* Persistent corrupt bytes within this attempt. Authentication is abstracted
\* to detecting every corrupted byte (ENV-AUTH), not a SHA-256 proof.
EnvCorruptBytes(positions) == SUBSET positions
=============================================================================
