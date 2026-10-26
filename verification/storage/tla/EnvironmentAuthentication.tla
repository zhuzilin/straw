----------------------- MODULE EnvironmentAuthentication -----------------------
\* ENV-AUTH: collision-free authentication of exactly the supplied sequences.
\* Binding the expected sequence to an immutable reference is an adapter duty.
\* This is an idealized checksum contract, not a proof of SHA-256.
EnvChunkAuthenticated(observed, expected) == observed = expected
=============================================================================
