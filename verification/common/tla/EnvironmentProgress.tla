----------------------- MODULE EnvironmentProgress -----------------------
\* ENV-FAIR: a continuously enabled modeled step eventually executes. This
\* does not prove per-task nonstarvation or eventual successful external I/O.
EnvWeakFairness(stateVars, step) == WF_stateVars(step)
\* ENV-STABILIZE: used only when stop is continuously enabled until it fires,
\* and no subsequent action can reintroduce faults. No numerical failure bound.
EnvFaultsEventuallyStop(stateVars, stop) == WF_stateVars(stop)
=============================================================================
