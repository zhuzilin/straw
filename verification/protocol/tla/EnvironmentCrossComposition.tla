--------------------- MODULE EnvironmentCrossComposition ---------------------
EXTENDS Naturals, FiniteSets
\* ENV-CROSS-COMPOSITION: transitions expose independently durable effects.
\* An uncertain call may stutter or expose its effect, including a queue
\* acceptance that did not reach in-memory apply/reply. Native linearization,
\* namespace separation, complete immutable closures and fencing remain duties
\* of lower straw components; this is not a filesystem-provided transaction.
\* Initial payloads/names are durable. Only deletion is modeled afterward, so
\* a storage crash can restore unsynced deletions, never erase a live durable name.
EnvCompositionRestored(current, durable) ==
    {restored \in SUBSET durable : current \subseteq restored}
EnvCompositionSynced(durable, pack) == durable \ {pack}
EnvCompositionReceipt(durable, key) == durable[key]
=============================================================================
