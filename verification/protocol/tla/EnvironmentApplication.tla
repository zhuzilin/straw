------------------------ MODULE EnvironmentApplication ------------------------
EXTENDS Sequences
\* ENV-COMMIT-APPLICATION: a hook can return an error or unwind to a Rust caller
\* that catches it. An unwind bypasses the Result error handler; completed WAL
\* and memory effects are not rolled back. No eventual-success premise is used.
EnvApplicationCuts == {"ok", "error_before", "error_after", "unwind_before", "unwind_after"}
EnvApplicationOutcome(cut, at) ==
    IF cut = "error_" \o at THEN "error"
    ELSE IF cut = "unwind_" \o at THEN "unwind" ELSE "ok"
=============================================================================
