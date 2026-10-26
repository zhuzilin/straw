------------------------ MODULE EnvironmentHandoff ------------------------
\* ENV-HANDOFF: callers can offer another ID, another API with the same ID,
\* or conflicting content while a prior call has not returned successfully.
EnvHandoffRequests == {"same", "other_id", "other_api", "conflict"}
\* A successful abstract catalog stage establishes the requested owner.
\* Real identity matching, closure computation and serialization are adapters.
EnvStagedRoot == TRUE
=============================================================================
