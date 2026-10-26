----------------------- MODULE EnvironmentNamespace -----------------------
\* ENV-NAMESPACE: stable aliases and truthful, pool-relative namespace resolution.
\* This mapping is supplied, not derived from realpath/readlink or Rust replay.
EnvResolvedIdentity(targets, name) == targets[name]
=============================================================================
