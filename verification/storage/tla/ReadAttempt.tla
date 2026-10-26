----------------------------- MODULE ReadAttempt -----------------------------
EXTENDS Naturals, EnvironmentVisibility, EnvironmentProgress
CONSTANTS MaxRetries, VisibleBy, Session, InitialCache, Corrupt, BadBinding, Mutation
ASSUME /\ MaxRetries \in Nat /\ VisibleBy \in Nat
       /\ Session \in BOOLEAN /\ InitialCache \in BOOLEAN /\ Corrupt \in BOOLEAN
       /\ (InitialCache => Session)
       /\ BadBinding \in {"none", "index", "frame", "envelope", "cache"}
       /\ Mutation \in {"none", "double_open", "hold_on_retry", "skip_envelope", "skip_payload", "stall", "skip_index", "skip_frame", "wrong_cache"}
\* One API call to an immutable protected extent. An initial cached index is
\* authenticated for its original reference; a different key is a cache miss.
\* BadBinding separates persistent index/frame/envelope corruption from lag.
\* Views belong to opened descriptors; the model does not measure syscall time.
VARIABLES phase, attempt, opened, openCount, gaps, cached,
          indexChecked, frameChecked, envelopeChecked, payloadChecked
vars == <<phase, attempt, opened, openCount, gaps, cached,
          indexChecked, frameChecked, envelopeChecked, payloadChecked>>
IndexRegions == {"length", "headerZero", "headerPartial", "footer", "index"}
Init == /\ phase = "open" /\ attempt = 0 /\ opened = FALSE /\ openCount = 0
        /\ gaps = {} /\ cached = (InitialCache /\ (BadBinding # "cache" \/ Mutation = "wrong_cache")) /\ indexChecked = (InitialCache /\ BadBinding # "cache")
        /\ frameChecked = FALSE /\ envelopeChecked = FALSE /\ payloadChecked = FALSE
Open == /\ phase \in {"open", "again"}
        /\ \E view \in EnvMissingViews(attempt, VisibleBy, EnvExtentRegions): gaps' = view
        /\ opened' = TRUE /\ openCount' = openCount + 1
        /\ phase' = IF phase = "again" \/ cached THEN "frame" ELSE "index"
        /\ UNCHANGED <<attempt, cached, indexChecked, frameChecked, envelopeChecked, payloadChecked>>
Index == /\ phase = "index"
         /\ LET valid == gaps \cap IndexRegions = {} /\ BadBinding # "index"
            IN /\ indexChecked' = valid
               /\ cached' = (Session /\ valid)
               /\ phase' = IF ~valid /\ Mutation # "skip_index" THEN "failed" ELSE
                            IF Mutation = "double_open" THEN "again" ELSE "frame"
         /\ UNCHANGED <<attempt, opened, openCount, gaps, frameChecked, envelopeChecked, payloadChecked>>
\* The selected frame must fit the authenticated extent and match its index
\* entry before its envelope can authorize a payload. Cached indexes do not
\* cache envelope or payload authentication.
Frame == /\ phase = "frame"
         /\ frameChecked' = (BadBinding # "frame" /\ Mutation # "skip_frame")
         /\ phase' = IF BadBinding # "frame" \/ Mutation = "skip_frame" THEN "envelope" ELSE "failed"
         /\ UNCHANGED <<attempt, opened, openCount, gaps, cached, indexChecked, envelopeChecked, payloadChecked>>
Envelope == /\ phase = "envelope"
            /\ envelopeChecked' = ("envelope" \notin gaps /\ BadBinding # "envelope" /\ Mutation # "skip_envelope")
            /\ phase' = IF ("envelope" \notin gaps /\ BadBinding # "envelope") \/ Mutation = "skip_envelope" THEN "payload" ELSE "failed"
            /\ UNCHANGED <<attempt, opened, openCount, gaps, cached, indexChecked, frameChecked, payloadChecked>>
Payload == /\ phase = "payload"
           /\ LET valid == gaps \cap {"payload", "chunk"} = {} /\ ~Corrupt
                  returned == valid \/ Mutation = "skip_payload"
              IN /\ payloadChecked' = (valid /\ Mutation # "skip_payload")
                 /\ phase' = IF returned THEN "done" ELSE "failed"
                 /\ opened' = ~returned
           /\ UNCHANGED <<attempt, openCount, gaps, cached, indexChecked, frameChecked, envelopeChecked>>
CloseFailed == /\ phase = "failed"
               /\ phase' = IF attempt = MaxRetries THEN "error" ELSE "wait"
               /\ opened' = (Mutation = "hold_on_retry" /\ attempt < MaxRetries)
               /\ UNCHANGED <<attempt, openCount, gaps, cached, indexChecked, frameChecked, envelopeChecked, payloadChecked>>
Retry == /\ phase = "wait" /\ Mutation # "stall"
         /\ attempt' = attempt + 1 /\ openCount' = 0
         /\ phase' = IF opened THEN IF cached THEN "frame" ELSE "index" ELSE "open"
         /\ indexChecked' = (cached /\ indexChecked) /\ frameChecked' = FALSE /\ envelopeChecked' = FALSE /\ payloadChecked' = FALSE
         /\ UNCHANGED <<opened, gaps, cached>>
Next == Open \/ Index \/ Frame \/ Envelope \/ Payload \/ CloseFailed \/ Retry
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"open", "again", "index", "frame", "envelope", "payload", "failed", "wait", "done", "error"}
          /\ attempt \in 0..MaxRetries /\ openCount \in 0..2
          /\ gaps \subseteq EnvExtentRegions
          /\ opened \in BOOLEAN /\ cached \in BOOLEAN
          /\ indexChecked \in BOOLEAN /\ frameChecked \in BOOLEAN /\ envelopeChecked \in BOOLEAN /\ payloadChecked \in BOOLEAN
SingleOpen == openCount <= 1
ReleasedBetweenAttempts == phase \in {"open", "wait", "done", "error"} => ~opened
NoUnverifiedReturn == phase = "done" => indexChecked /\ frameChecked /\ envelopeChecked /\ payloadChecked /\ ~Corrupt
NoFailureWithinContract == (~Corrupt /\ BadBinding \in {"none", "cache"} /\ VisibleBy <= MaxRetries) => phase # "error"
Terminates == <>(phase \in {"done", "error"})
=============================================================================
