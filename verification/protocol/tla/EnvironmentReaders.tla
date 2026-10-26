-------------------------- MODULE EnvironmentReaders --------------------------
\* ENV-PEERS: a caller may acknowledge a reader only after the actual read ends.
\* Cancellation/expiry/recovery of its execution lease is not such evidence.
EnvReaderMayAcknowledge(finished, reader) == reader \in finished
=============================================================================
