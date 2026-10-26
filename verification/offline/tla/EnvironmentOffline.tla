------------------------- MODULE EnvironmentOffline -------------------------
EXTENDS Naturals
\* ENV-OFFLINE: advisory stat may lie. Fresh open absence is trusted; errors
\* do not certify absence. Shared ownership has a catalog/queue marker.
EnvOfflineStat(present) == {"present", "absent", "error"}
EnvOfflineOpen(present) == IF present THEN {"present", "error"} ELSE {"absent", "error"}
\* A failed deletion may already have removed the file. No rollback promise.
EnvOfflineDeletion == {<<TRUE, TRUE>>, <<TRUE, FALSE>>, <<FALSE, FALSE>>}
=============================================================================
