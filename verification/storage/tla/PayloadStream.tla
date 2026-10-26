--------------------------- MODULE PayloadStream ---------------------------
EXTENDS Naturals, EnvironmentInputs, EnvironmentProgress
CONSTANTS MaxBytes, MaxChunks, Mutation
ASSUME Mutation \in {"none", "ignore_length", "ignore_callback_error", "reread_length"}
\* Two traversals: envelope hashing then writing. Length is captured once.
\* Hash equality is abstracted to equal byte counts; this is only a necessary
\* condition, so this model overapproximates content equality.
VARIABLES expected, phase, seen, hashed, written, frameLength, failed, hadError, calls
vars == <<expected, phase, seen, hashed, written, frameLength, failed, hadError, calls>>
Init == /\ expected \in EnvStreamLengths(MaxBytes)
        /\ phase = "hash" /\ seen = 0 /\ hashed = 0 /\ written = 0
        /\ frameLength = expected /\ failed = FALSE /\ hadError = FALSE /\ calls = 0
Chunk == /\ phase \in {"hash", "write"} /\ calls < MaxChunks
         /\ \E n \in EnvStreamLengths(MaxBytes), callbackError \in BOOLEAN:
             LET accept == ~failed /\ (Mutation = "ignore_length" \/ seen + n <= expected)
                 error == accept /\ callbackError /\ phase = "write"
             IN /\ seen' = IF accept THEN seen + n ELSE seen
                /\ written' = IF accept /\ phase = "write" THEN written + n ELSE written
                /\ hadError' = (hadError \/ error)
                /\ failed' = (failed \/ ~accept \/ (error /\ Mutation # "ignore_callback_error"))
         /\ calls' = calls + 1
         /\ UNCHANGED <<expected, phase, hashed, frameLength>>
End == /\ phase \in {"hash", "write"}
       /\ \E advertised \in EnvStreamLengths(MaxBytes):
          LET valid == ~failed /\ (Mutation = "ignore_length" \/ seen = expected)
              enterWrite == phase = "hash" /\ valid
          IN /\ phase' = IF ~valid THEN "error" ELSE IF enterWrite THEN "write"
                          ELSE IF written = hashed THEN "done" ELSE "error"
             /\ hashed' = IF enterWrite THEN seen ELSE hashed
             /\ frameLength' = IF enterWrite /\ Mutation = "reread_length" THEN advertised ELSE frameLength
             /\ seen' = IF enterWrite THEN 0 ELSE seen
       /\ calls' = 0
       /\ UNCHANGED <<expected, written, failed, hadError>>
SourceError == /\ phase \in {"hash", "write"} /\ phase' = "error"
               /\ UNCHANGED <<expected, seen, hashed, written, frameLength, failed, hadError, calls>>
Next == Chunk \/ End \/ SourceError
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ expected \in 0..MaxBytes /\ frameLength \in 0..MaxBytes
          /\ phase \in {"hash", "write", "done", "error"}
          /\ seen \in 0..(MaxBytes * MaxChunks) /\ hashed \in 0..(MaxBytes * MaxChunks)
          /\ written \in 0..(MaxBytes * MaxChunks) /\ calls \in 0..MaxChunks
          /\ failed \in BOOLEAN /\ hadError \in BOOLEAN
ExactPublication == phase = "done" => written = expected /\ frameLength = expected
NoIgnoredFailure == phase = "done" => ~hadError
GuardBounds == Mutation # "none" \/ (seen <= expected /\ written <= expected)
Terminates == <>(phase \in {"done", "error"})
=============================================================================
