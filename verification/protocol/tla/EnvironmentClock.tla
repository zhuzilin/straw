------------------------- MODULE EnvironmentClock -------------------------
\* ENV-CLOCK-ABSTRACT: expiry is a nondeterministic Boolean event while the
\* coordinator is idle. No clock drift, NaN, RPC queuing or elapsed-time model.
EnvLeaseExpires(leased, expired, nextExpired) == leased /\ ~expired /\ nextExpired = TRUE
=============================================================================
