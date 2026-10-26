----------------------- MODULE EnvironmentCommitCleanup -----------------------
\* ENV-COMMIT-CLEANUP: the lower catalog exposes truthful durable receipts.
\* Its serialized frame may take effect even when the response is an error.
\* Atomic coupling of receipt and release is an adapter obligation, not a
\* promise that the filesystem performs the algorithm's missing operations.
EnvHandoffRecorded(durableReceipt) == durableReceipt
EnvHandoffResponses(unstable) ==
    IF unstable THEN {"unchanged_error", "committed_error", "ok"} ELSE {"ok"}
=============================================================================
