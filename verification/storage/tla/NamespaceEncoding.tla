------------------------ MODULE NamespaceEncoding ------------------------
EXTENDS Naturals, Sequences, EnvironmentNamespace, EnvironmentProgress
CONSTANTS Mode, Mutation
Names == {"file", "alias"}
Targets == [name \in Names |-> "file"]
Fields == 1..4
Original == <<"alias", "alias", "alias", "alias">>
ASSUME /\ Mode \in {"new", "legacy", "unknown"}
       /\ Mutation \in {"none", "skip_field", "trust_legacy", "accept_unknown", "resolve_new"}
VARIABLES phase, keys, cursor, marker, decoded, lookups, rejected
vars == <<phase, keys, cursor, marker, decoded, lookups, rejected>>
Init == /\ phase = IF Mode = "new" THEN "normalize" ELSE "decode"
        /\ keys = Original /\ cursor = 1
        /\ marker = IF Mode = "unknown" THEN 2 ELSE 0
        /\ decoded = <<>> /\ lookups = 0 /\ rejected = FALSE
Normalize == /\ phase = "normalize" /\ cursor <= 4
             /\ keys' = [keys EXCEPT ![cursor] = IF Mutation = "skip_field" /\ cursor = 4 THEN @ ELSE EnvResolvedIdentity(Targets, @)]
             /\ cursor' = cursor + 1
             /\ UNCHANGED <<phase, marker, decoded, lookups, rejected>>
Certify == /\ phase = "normalize" /\ cursor = 5
           /\ marker' = 1 /\ cursor' = 1 /\ phase' = "decode"
           /\ UNCHANGED <<keys, decoded, lookups, rejected>>
Decode == /\ phase = "decode" /\ cursor <= 4
          /\ LET trust == (marker = 1 /\ Mutation # "resolve_new") \/ (marker = 0 /\ Mutation = "trust_legacy")
                 reject == marker > 1 /\ Mutation # "accept_unknown"
             IN /\ rejected' = reject
                /\ decoded' = IF reject THEN decoded ELSE Append(decoded, IF trust THEN keys[cursor] ELSE EnvResolvedIdentity(Targets, keys[cursor]))
                /\ lookups' = lookups + IF reject \/ trust THEN 0 ELSE 1
                /\ phase' = IF reject THEN "done" ELSE phase
          /\ cursor' = cursor + 1
          /\ UNCHANGED <<keys, marker>>
Finish == /\ phase = "decode" /\ cursor = 5 /\ phase' = "done"
          /\ UNCHANGED <<keys, cursor, marker, decoded, lookups, rejected>>
Next == Normalize \/ Certify \/ Decode \/ Finish
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"normalize", "decode", "done"}
          /\ keys \in [Fields -> Names] /\ cursor \in 1..5 /\ marker \in 0..2
          /\ decoded \in Seq(Names) /\ Len(decoded) <= 4 /\ lookups \in 0..4 /\ rejected \in BOOLEAN
CertifiedKeys == marker = 1 => \A field \in Fields: keys[field] = Targets[Original[field]]
DecodedIdentities == \A field \in 1..Len(decoded): decoded[field] = Targets[Original[field]]
UnknownRejected == marker > 1 => decoded = <<>>
NoResolvedLookups == marker = 1 => lookups = 0
EventuallyDecoded == <> (phase = "done" /\ (rejected \/ Len(decoded) = 4))
=============================================================================
