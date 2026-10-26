------------------------ MODULE EnvironmentWriter ------------------------
\* ENV-WRITER-LIFETIME: process state is distinct from storage availability.
\* Process execution and completion of already-authorized storage work are
\* independent. Kernel/server work may complete while the process is paused
\* or after its permanent death. Only issuing NEW work requires a live process.
\* Death is permanent for this process identity, not a remotely observable
\* certificate. A timeout/suspicion cannot distinguish these states.
EnvWriterStates == {"running", "paused", "dead"}
EnvWriterSuccessors(process) ==
    CASE process = "running" -> {"paused", "dead"}
      [] process = "paused" -> {"running", "dead"}
      [] OTHER -> {}
EnvWriterRuns(process) == process = "running"
EnvWriterCompletes(process, inFlight) == inFlight
EnvWriterSuspicions(process) == {FALSE, TRUE}
=============================================================================
