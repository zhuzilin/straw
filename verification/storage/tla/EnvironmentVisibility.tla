---------------------- MODULE EnvironmentVisibility ----------------------
EXTENDS Naturals
\* ENV-VIS: per-fresh-open defensive observation model, not a JuiceFS claim.
\* VisibleBy counts attempts. No wall-clock or p99 bound is implied.
EnvExtentRegions == {"length", "headerZero", "headerPartial", "footer", "index",
                     "envelope", "payload", "chunk"}
EnvDataRegions == {"footer", "index", "envelope", "payload", "chunk"}
EnvMissingViews(attempt, visibleBy, allowedGaps) ==
    IF attempt >= visibleBy THEN {{}} ELSE SUBSET allowedGaps
=============================================================================
