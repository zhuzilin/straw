----------------------------- MODULE ReadAttempt -----------------------------
EXTENDS Naturals, EnvironmentVisibility, EnvironmentProgress
CONSTANTS MaxRetries, VisibleBy, Session, InitialCache, Corrupt, Mutation
ASSUME /\ MaxRetries \in Nat /\ VisibleBy \in Nat
       /\ Session \in BOOLEAN /\ InitialCache \in BOOLEAN /\ Corrupt \in BOOLEAN
       /\ (InitialCache => Session)
       /\ Mutation \in {"none", "double_open", "hold_on_retry", "skip_envelope", "skip_payload", "stall"}
\* One API call to an immutable protected extent. An initial cached index is
\* already authenticated and bound to the exact reference (ENV-AUTH).
\* Views belong to opened descriptors; the model does not measure syscall time.
VARIABLES phase, attempt, opened, openCount, gaps, cached,
          indexChecked, envelopeChecked, payloadChecked
vars == <<phase, attempt, opened, openCount, gaps, cached,
          indexChecked, envelopeChecked, payloadChecked>>
IndexRegions == {"length", "headerZero", "headerPartial", "footer", "index"}
Init == /\ phase = "open" /\ attempt = 0 /\ opened = FALSE /\ openCount = 0
        /\ gaps = {} /\ cached = InitialCache /\ indexChecked = InitialCache
        /\ envelopeChecked = FALSE /\ payloadChecked = FALSE
Open == /\ phase \in {"open", "again"}
        /\ \E view \in EnvMissingViews(attempt, VisibleBy, EnvExtentRegions): gaps' = view
        /\ opened' = TRUE /\ openCount' = openCount + 1
        /\ phase' = IF phase = "again" \/ cached THEN "envelope" ELSE "index"
        /\ UNCHANGED <<attempt, cached, indexChecked, envelopeChecked, payloadChecked>>
Index == /\ phase = "index"
         /\ LET valid == gaps \cap IndexRegions = {}
            IN /\ indexChecked' = valid
               /\ cached' = (Session /\ valid)
               /\ phase' = IF ~valid THEN "failed" ELSE
                            IF Mutation = "double_open" THEN "again" ELSE "envelope"
         /\ UNCHANGED <<attempt, opened, openCount, gaps, envelopeChecked, payloadChecked>>
Envelope == /\ phase = "envelope"
            /\ envelopeChecked' = ("envelope" \notin gaps /\ Mutation # "skip_envelope")
            /\ phase' = IF "envelope" \notin gaps \/ Mutation = "skip_envelope" THEN "payload" ELSE "failed"
            /\ UNCHANGED <<attempt, opened, openCount, gaps, cached, indexChecked, payloadChecked>>
Payload == /\ phase = "payload"
           /\ LET valid == gaps \cap {"payload", "chunk"} = {} /\ ~Corrupt
                  returned == valid \/ Mutation = "skip_payload"
              IN /\ payloadChecked' = (valid /\ Mutation # "skip_payload")
                 /\ phase' = IF returned THEN "done" ELSE "failed"
                 /\ opened' = ~returned
           /\ UNCHANGED <<attempt, openCount, gaps, cached, indexChecked, envelopeChecked>>
CloseFailed == /\ phase = "failed"
               /\ phase' = IF attempt = MaxRetries THEN "error" ELSE "wait"
               /\ opened' = (Mutation = "hold_on_retry" /\ attempt < MaxRetries)
               /\ UNCHANGED <<attempt, openCount, gaps, cached, indexChecked, envelopeChecked, payloadChecked>>
Retry == /\ phase = "wait" /\ Mutation # "stall"
         /\ attempt' = attempt + 1 /\ openCount' = 0
         /\ phase' = IF opened THEN IF cached THEN "envelope" ELSE "index" ELSE "open"
         /\ indexChecked' = cached /\ envelopeChecked' = FALSE /\ payloadChecked' = FALSE
         /\ UNCHANGED <<opened, gaps, cached>>
Next == Open \/ Index \/ Envelope \/ Payload \/ CloseFailed \/ Retry
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"open", "again", "index", "envelope", "payload", "failed", "wait", "done", "error"}
          /\ attempt \in 0..MaxRetries /\ openCount \in 0..2
          /\ gaps \subseteq EnvExtentRegions
          /\ opened \in BOOLEAN /\ cached \in BOOLEAN
          /\ indexChecked \in BOOLEAN /\ envelopeChecked \in BOOLEAN /\ payloadChecked \in BOOLEAN
SingleOpen == openCount <= 1
ReleasedBetweenAttempts == phase \in {"open", "wait", "done", "error"} => ~opened
NoUnverifiedReturn == phase = "done" => indexChecked /\ envelopeChecked /\ payloadChecked /\ ~Corrupt
NoFailureWithinContract == (~Corrupt /\ VisibleBy <= MaxRetries) => phase # "error"
Terminates == <>(phase \in {"done", "error"})
=============================================================================
