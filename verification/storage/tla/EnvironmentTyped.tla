--------------------------- MODULE EnvironmentTyped ---------------------------
\* The adapter must preserve authenticated envelope and caller layout fields.
\* This is an explicit unproved correspondence, not a JSON/FFI proof.
EnvObservedLayout(actual, requested) == actual
\* Payload allocation may fail. Metadata allocation/decoding and process aborts
\* are outside this model, as is a numeric allocator/memory-capacity guarantee.
EnvPayloadAllocationOutcomes == {"ok", "error"}
=============================================================================
