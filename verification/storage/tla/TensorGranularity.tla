-------------------------- MODULE TensorGranularity --------------------------
EXTENDS Naturals
CONSTANTS MaxBytes, BaseChunk, LegacyBudget, Mutation
ASSUME /\ MaxBytes > 0 /\ BaseChunk > 0 /\ LegacyBudget > 0
       /\ Mutation \in {"none", "grow_new", "reinterpret_old", "skip_left",
                        "inclusive_end", "read_empty", "cap_metadata"}
\* Arithmetic policy only: metadata is already bound to the immutable record.
\* No filesystem, hashing, allocator, clock or Python-to-Rust refinement here.
VARIABLES length, start, stop, legacy, phase, stored, digests, first, end
vars == <<length, start, stop, legacy, phase, stored, digests, first, end>>
Min(a, b) == IF a < b THEN a ELSE b
Count(n, c) == (n + c - 1) \div c
RECURSIVE OldChunk(_, _)
OldChunk(n, c) == IF Count(n, c) <= LegacyBudget THEN c ELSE OldChunk(n, 2 * c)
ExpectedFirst(c) == IF start = stop THEN start ELSE (start \div c) * c
ExpectedEnd(c) == IF start = stop THEN stop ELSE Min(length, (((stop - 1) \div c) + 1) * c)
Init == /\ length \in 0..MaxBytes /\ start \in 0..length /\ stop \in start..length
        /\ legacy \in BOOLEAN /\ phase = "encode"
        /\ stored = BaseChunk /\ digests = 0 /\ first = 0 /\ end = 0
Encode == /\ phase = "encode"
          /\ stored' = IF legacy \/ Mutation = "grow_new" THEN OldChunk(length, BaseChunk) ELSE BaseChunk
          /\ digests' = IF Mutation = "cap_metadata" THEN Min(Count(length, stored'), LegacyBudget)
                        ELSE Count(length, stored')
          /\ phase' = "read"
          /\ UNCHANGED <<length, start, stop, legacy, first, end>>
Read == /\ phase = "read"
        /\ LET c == IF Mutation = "reinterpret_old" THEN BaseChunk ELSE stored
           IN /\ first' = IF Mutation = "read_empty" /\ start = stop THEN 0
                          ELSE IF Mutation = "skip_left" THEN start ELSE ExpectedFirst(c)
              /\ end' = IF Mutation = "read_empty" /\ start = stop THEN length
                        ELSE IF Mutation = "inclusive_end" /\ start < stop
                             THEN Min(length, ((stop \div c) + 1) * c) ELSE ExpectedEnd(c)
        /\ phase' = "done"
        /\ UNCHANGED <<length, start, stop, legacy, stored, digests>>
Next == Encode \/ Read
Spec == Init /\ [][Next]_vars
TypeOK == /\ length \in 0..MaxBytes /\ start \in 0..length /\ stop \in start..length
          /\ legacy \in BOOLEAN /\ phase \in {"encode", "read", "done"}
          /\ stored \in 1..(BaseChunk + 2 * MaxBytes) /\ digests \in 0..MaxBytes
          /\ first \in 0..length /\ end \in first..length
FixedNewMetadata == phase # "encode" /\ ~legacy => stored = BaseChunk
CompleteMetadata == phase # "encode" => digests = Count(length, stored)
CanonicalSpan == phase = "done" => /\ first = ExpectedFirst(stored) /\ end = ExpectedEnd(stored)
EmptyReadsNoPayload == phase = "done" /\ start = stop => end = first
BoundedNewAuth == phase = "done" /\ ~legacy =>
                 /\ end - first <= stop - start + 2 * (BaseChunk - 1)
                 /\ (stop = start + 1 => end - first <= BaseChunk)
=============================================================================
