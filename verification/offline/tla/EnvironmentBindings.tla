----------------------- MODULE EnvironmentBindings -----------------------
\* ENV-OFFLINE-BINDINGS: truthful complete POSIX resolution trace, supplied by
\* an adapter. No external rebinding; algorithm unlink remains allowed and is
\* checked by the consumer. A path before '..' still contributes dependencies.
EnvReferenceBindings(bindings, reference) == bindings[reference]
=============================================================================
