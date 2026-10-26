------------------- MODULE EnvironmentWriterRetirement -------------------
EXTENDS Naturals
\* Independent filesystem/process assumptions. Process death does not itself
\* cancel previously submitted I/O. The gate may pass to a retiree only after
\* such operations finish/cancel; this is a deployment contract, not a fact
\* derivable from flock availability or a model check.
EnvWriterStates == {"running", "paused", "dead"}
EnvWriterNext(p) == IF p = "dead" THEN {"dead"} ELSE {"running", "paused", "dead"}
EnvGateMayPass(process, inFlight) == process = "dead" /\ ~inFlight
EnvMayComplete(inFlight) == inFlight
=============================================================================
