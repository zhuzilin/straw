--------------------- MODULE EnvironmentParentRepair ---------------------
\* ENV-GC-PARENT: a reserved pack's private parent may never have been created.
\* Before stabilization, opening an existing directory can also report missing.
\* A missing response is not durable-absence evidence. Other errors propagate.
EnvParentOpen(present, unstable) ==
    IF unstable THEN (IF present THEN {"opened", "missing", "error"} ELSE {"missing", "error"})
    ELSE (IF present THEN {"opened"} ELSE {"missing"})
\* Missing prefixes are private unused names; mkdir does not replace an
\* existing directory, modify other contents, or resurrect a prior identity.
EnvParentCreated(present) == TRUE
EnvParentAfterFailedCreate(present) == {present, TRUE}
=============================================================================
