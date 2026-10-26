------------------------- MODULE ReactivationScan -------------------------
EXTENDS Naturals, FiniteSets, EnvironmentRead, EnvironmentProgress
CONSTANTS Records, MaxBytes, BufferBytes, Mutation
ASSUME /\ Records # {} /\ MaxBytes > 0 /\ BufferBytes > 0
       /\ Mutation \in {"none", "whole_buffer", "skip_tail", "ignore_corruption", "accept_read_error"}
VARIABLES lengths, corrupt, remaining, current, position, checked, dirty,
          peak, phase, authorized, failed
vars == <<lengths, corrupt, remaining, current, position, checked, dirty,
          peak, phase, authorized, failed>>
Positions == UNION {{<<r, p>> : p \in 1..lengths[r]} : r \in Records}
Init == /\ lengths \in [Records -> 0..MaxBytes]
        /\ corrupt \in EnvCorruptBytes(Positions)
        /\ remaining = Records /\ current \in Records
        /\ position = 0 /\ checked = {} /\ dirty = FALSE /\ peak = 0
        /\ phase = "scan" /\ authorized = FALSE /\ failed = FALSE
Minimum(a, b) == IF a < b THEN a ELSE b
Read == /\ phase = "scan" /\ position < lengths[current]
        /\ \E outcome \in EnvExactReadOutcomes:
           LET count == IF Mutation = "whole_buffer" THEN lengths[current] - position
                        ELSE Minimum(lengths[current] - position, BufferBytes)
               bytes == {<<current, p>> : p \in (position + 1)..(position + count)}
               complete == outcome = "complete"
           IN /\ checked' = IF complete THEN checked \cup bytes ELSE checked
              /\ dirty' = (dirty \/ (complete /\ bytes \cap corrupt # {}))
              /\ position' = IF ~complete THEN position
                             ELSE IF Mutation = "skip_tail" THEN lengths[current]
                             ELSE position + count
              /\ peak' = IF count > peak THEN count ELSE peak
              /\ phase' = IF complete THEN "scan"
                          ELSE IF Mutation = "accept_read_error" THEN "verified" ELSE "error"
              /\ failed' = (failed \/ ~complete)
        /\ UNCHANGED <<lengths, corrupt, remaining, current, authorized>>
Valid == ~dirty \/ Mutation = "ignore_corruption"
Reject == /\ phase = "scan" /\ position = lengths[current] /\ ~Valid
          /\ phase' = "error" /\ failed' = TRUE
          /\ UNCHANGED <<lengths, corrupt, remaining, current, position, checked,
                          dirty, peak, authorized>>
NextRecord == /\ phase = "scan" /\ position = lengths[current] /\ Valid
              /\ remaining \ {current} # {}
              /\ remaining' = remaining \ {current} /\ current' \in remaining'
              /\ position' = 0 /\ dirty' = FALSE
              /\ UNCHANGED <<lengths, corrupt, checked, peak, phase, authorized, failed>>
Verified == /\ phase = "scan" /\ position = lengths[current] /\ Valid
            /\ remaining = {current} /\ remaining' = {} /\ phase' = "verified"
            /\ UNCHANGED <<lengths, corrupt, current, position, checked, dirty, peak,
                            authorized, failed>>
Authorize == /\ phase = "verified" /\ phase' = "done" /\ authorized' = TRUE
             /\ UNCHANGED <<lengths, corrupt, remaining, current, position, checked,
                             dirty, peak, failed>>
Next == Read \/ Reject \/ NextRecord \/ Verified \/ Authorize
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ lengths \in [Records -> 0..MaxBytes] /\ corrupt \subseteq Positions
          /\ remaining \subseteq Records /\ current \in Records
          /\ position \in 0..lengths[current] /\ checked \subseteq Positions
          /\ peak \in 0..MaxBytes /\ phase \in {"scan", "verified", "done", "error"}
          /\ dirty \in BOOLEAN /\ authorized \in BOOLEAN /\ failed \in BOOLEAN
BoundedScratch == peak <= BufferBytes
NoAuthorizationAfterFailure == failed => ~authorized
CompleteCoverage == authorized => checked = Positions
Authentication == authorized => corrupt = {}
Terminates == <>(phase \in {"done", "error"})
\* One attempt after metadata validation, while a source owner/lock protects
\* the complete closure. This is not a graph/GC/replay composition proof.
=============================================================================
