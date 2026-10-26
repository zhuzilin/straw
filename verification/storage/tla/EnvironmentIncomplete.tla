---------------------- MODULE EnvironmentIncomplete ----------------------
EXTENDS Naturals
\* ENV-INCOMPLETE: one live owning writer; an accurate saved pre-write length.
\* A successful truncation preserves the prefix and returns exactly that length.
\* Failed operations may remove some/all of the unpublished suffix.
EnvTruncatedLength(prefix) == prefix
EnvFailedTruncate(prefix, current) == prefix..current
=============================================================================
