----------------------------- MODULE RangeRead -----------------------------
EXTENDS Naturals, Sequences, EnvironmentRead, EnvironmentAuthentication, EnvironmentProgress
CONSTANTS Length, ChunkBytes, BufferBytes, ReadErrors, Corruption, Mutation
ASSUME /\ Length > 0 /\ ChunkBytes > 0 /\ BufferBytes > 0
       /\ ReadErrors \in BOOLEAN /\ Corruption \in BOOLEAN
       /\ Mutation \in {"none", "whole_buffer", "copy_whole_buffer", "local_copy_position",
                        "skip_auth", "skip_last_chunk", "early_return", "accept_error", "stall", "reject_valid"}
\* One attempt after reference/envelope validation. Position labels make wrong
\* offsets observable; they are not actual byte values or a TB throughput model.
VARIABLES start, stop, corrupt, phase, left, position, observed, output,
          checked, authenticated, peak, failed
vars == <<start, stop, corrupt, phase, left, position, observed, output,
          checked, authenticated, peak, failed>>
Min(a, b) == IF a < b THEN a ELSE b
Max(a, b) == IF a > b THEN a ELSE b
Clip(edge, at, count) == IF edge <= at THEN 0 ELSE Min(edge - at, count)
Expected(at, count) == [i \in 1..count |-> at + i]
Contents(at, count) == [i \in 1..count |-> IF at + i - 1 \in corrupt THEN 0 ELSE at + i]
ChunkEnd == Min(Length, left + ChunkBytes)
Touched == IF start = stop THEN {} ELSE ((start \div ChunkBytes) * ChunkBytes)..
             (Min(Length, (((stop - 1) \div ChunkBytes) + 1) * ChunkBytes) - 1)
Init == /\ start \in 0..Length /\ stop \in start..Length
        /\ corrupt \in IF Corruption THEN EnvCorruptBytes(0..(Length - 1)) ELSE {{}}
        /\ phase = IF start = stop THEN "done" ELSE "read"
        /\ left = (start \div ChunkBytes) * ChunkBytes /\ position = left
        /\ observed = <<>> /\ output = <<>> /\ checked = {} /\ authenticated = {}
        /\ peak = 0 /\ failed = FALSE
Read == /\ phase = "read" /\ position < ChunkEnd
        /\ \E outcome \in IF ReadErrors THEN EnvExactReadOutcomes ELSE {"complete"}:
           LET count == IF Mutation = "whole_buffer" THEN ChunkEnd - position
                        ELSE Min(BufferBytes, ChunkEnd - position)
               data == Contents(position, count)
               at == IF Mutation = "local_copy_position" THEN position - left ELSE position
               from == Clip(start, at, count)
               to == Clip(stop, at, count)
               part == IF Mutation = "copy_whole_buffer" THEN data ELSE SubSeq(data, from + 1, to)
               complete == outcome = "complete"
           IN /\ position' = IF complete THEN position + count ELSE position
              /\ observed' = IF complete THEN observed \o data ELSE observed
              /\ output' = IF complete THEN output \o part ELSE output
              /\ checked' = IF complete THEN checked \cup position..(position + count - 1) ELSE checked
              /\ phase' = IF ~complete THEN IF Mutation = "accept_error" THEN "done" ELSE "error"
                          ELSE IF Mutation = "early_return" THEN "done"
                          ELSE IF position' = ChunkEnd THEN "verify" ELSE "read"
              /\ failed' = (failed \/ ~complete)
              /\ peak' = Max(peak, count)
        /\ UNCHANGED <<start, stop, corrupt, left, authenticated>>
Verify == /\ phase = "verify" /\ Mutation # "stall"
          /\ LET valid == EnvChunkAuthenticated(observed, Expected(left, ChunkEnd - left))
                 accepted == (valid \/ Mutation = "skip_auth") /\ Mutation # "reject_valid"
                 last == ChunkEnd >= stop \/ Mutation = "skip_last_chunk"
             IN /\ phase' = IF ~accepted THEN "error" ELSE IF last THEN "done" ELSE "read"
                /\ authenticated' = IF accepted THEN authenticated \cup left..(ChunkEnd - 1) ELSE authenticated
                /\ left' = IF accepted /\ ~last THEN ChunkEnd ELSE left
                /\ position' = IF accepted /\ ~last THEN ChunkEnd ELSE position
                /\ observed' = IF accepted /\ ~last THEN <<>> ELSE observed
                /\ failed' = (failed \/ ~accepted)
          /\ UNCHANGED <<start, stop, corrupt, output, checked, peak>>
Next == Read \/ Verify
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ start \in 0..Length /\ stop \in start..Length
          /\ corrupt \subseteq 0..(Length - 1)
          /\ phase \in {"read", "verify", "done", "error"}
          /\ left \in 0..Length /\ position \in left..Length
          /\ observed \in Seq(0..Length) /\ output \in Seq(0..Length)
          /\ checked \subseteq 0..(Length - 1) /\ authenticated \subseteq checked
          /\ peak \in 0..Length /\ failed \in BOOLEAN
ExactOutput == phase = "done" => output = Expected(start, stop - start)
AuthenticatedCoverage == phase = "done" => authenticated = Touched
CheckedAccounting == phase = "done" => checked = Touched
NoCorruptReturn == phase = "done" => corrupt \cap Touched = {}
NoReturnOnFailure == failed => phase # "done"
NoSpuriousError == ~ReadErrors /\ corrupt \cap Touched = {} => ~failed
BoundedScratch == peak <= BufferBytes
Terminates == <>(phase \in {"done", "error"})
=============================================================================
