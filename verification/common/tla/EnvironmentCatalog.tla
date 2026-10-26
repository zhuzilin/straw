----------------------- MODULE EnvironmentCatalog -----------------------
\* ENV-CATALOG-SERIAL: other compliant actors cannot mutate the catalog while
\* the collector holds the scan-to-tombstone lock. Disabling this contract
\* covers either a faulty distributed lock or a caller bypassing the lock.
\* Catalog actions are already atomic/durable in Ownership; this operator is
\* not a proof of locks, refresh, byte framing or their composition.
\* ReactivationRace also uses "scan" as a held-lock marker for its final
\* source/tombstone recheck through owner append; the payload scan is unlocked.
EnvCatalogAccess(gcPhase, serialized) == ~serialized \/ gcPhase # "scan"
\* ENV-GC-DISCOVERY: presence stat can be stale or fail. Initial auto-discovery
\* instead trusts fresh open: NotFound means legacy only before any requirement
\* is known. A known pool may later appear absent; never downgrade from it.
\* Managed/legacy mode is fixed before clients start; hot migration is excluded.
EnvDiscoveryStat == {"present", "absent", "error"}
EnvDiscoveryOpen(managed, remembered) ==
    IF remembered THEN {"present", "absent", "error"}
    ELSE IF managed THEN {"present", "error"} ELSE {"absent", "error"}
=============================================================================
