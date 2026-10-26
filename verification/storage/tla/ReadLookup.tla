----------------------------- MODULE ReadLookup -----------------------------
EXTENDS Naturals, Sequences, EnvironmentLookup, EnvironmentVisibility, EnvironmentProgress
CONSTANTS MaxRetries, LookupRetries, NameBy, VisibleBy, OtherErrors, Corrupt, Mutation
ASSUME /\ MaxRetries \in 0..5 /\ LookupRetries \in 0..4
       /\ NameBy \in Nat /\ VisibleBy \in Nat
       /\ OtherErrors \in BOOLEAN /\ Corrupt \in BOOLEAN
       /\ Mutation \in {"none", "early_missing", "retry_error", "extra_lookup",
                         "reset_extent", "missing_success", "stall"}
\* One protected immutable reference. Namespace resolution precedes this
\* model; each successful open's data validation is abstracted as one step.
\* It complements ReadAttempt, rather than proving the complete I/O adapter.
VARIABLES phase, attempt, lookup, calls, lookupWait, extentWait,
          opened, authenticated, lastOpen, failure
vars == <<phase, attempt, lookup, calls, lookupWait, extentWait,
          opened, authenticated, lastOpen, failure>>
Delays == <<100, 250, 500, 1000, 2000>>
RECURSIVE PrefixSum(_, _)
PrefixSum(values, n) == IF n = 0 THEN 0 ELSE values[n] + PrefixSum(values, n - 1)
LookupWaitBound == 10 * (2 ^ LookupRetries - 1)
TotalWaitBound == (MaxRetries + 1) * LookupWaitBound + PrefixSum(Delays, MaxRetries)
Init == /\ phase = "open" /\ attempt = 0 /\ lookup = 0 /\ calls = 0
        /\ lookupWait = 0 /\ extentWait = 0 /\ opened = FALSE
        /\ authenticated = FALSE /\ lastOpen = "none" /\ failure = "none"
Open == /\ phase = "open"
        /\ \E response \in EnvOpenResponses(lookup, NameBy, OtherErrors):
            /\ lastOpen' = response
            /\ opened' = (response = "present")
            /\ LET again == lookup < LookupRetries \/
                            (Mutation = "extra_lookup" /\ lookup = LookupRetries)
               IN /\ phase' = CASE response = "present" -> "data"
                               [] response = "error" -> IF Mutation = "retry_error" THEN "lookup_wait" ELSE "error"
                               [] Mutation = "missing_success" -> "done"
                               [] again /\ Mutation # "early_missing" -> "lookup_wait"
                               [] OTHER -> "error"
                  /\ failure' = CASE response = "error" -> "storage"
                                  [] phase' = "error" -> "missing"
                                  [] OTHER -> "none"
        /\ calls' = calls + 1
        /\ UNCHANGED <<attempt, lookup, lookupWait, extentWait, authenticated>>
LookupRetry == /\ phase = "lookup_wait"
               /\ lookupWait' = lookupWait + 10 * 2 ^ lookup
               /\ lookup' = lookup + 1 /\ phase' = "open"
               /\ UNCHANGED <<attempt, calls, extentWait, opened, authenticated, lastOpen, failure>>
Data == /\ phase = "data"
        /\ \E view \in EnvMissingViews(attempt, VisibleBy, {"payload"}):
            LET valid == view = {} /\ ~Corrupt
            IN /\ authenticated' = valid
               /\ phase' = IF valid THEN "done" ELSE IF attempt = MaxRetries THEN "error" ELSE "extent_wait"
               /\ failure' = IF valid THEN "none" ELSE "data"
        /\ opened' = FALSE
        /\ UNCHANGED <<attempt, lookup, calls, lookupWait, extentWait, lastOpen>>
ExtentRetry == /\ phase = "extent_wait" /\ Mutation # "stall"
               /\ extentWait' = extentWait + Delays[attempt + 1]
               /\ attempt' = IF Mutation = "reset_extent" THEN 0 ELSE attempt + 1
               /\ lookup' = 0 /\ phase' = "open"
               /\ UNCHANGED <<calls, lookupWait, opened, authenticated, lastOpen, failure>>
Next == Open \/ LookupRetry \/ Data \/ ExtentRetry
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ phase \in {"open", "data", "lookup_wait", "extent_wait", "done", "error"}
          /\ attempt \in Nat /\ lookup \in Nat /\ calls \in Nat
          /\ lookupWait \in Nat /\ extentWait \in Nat
          /\ opened \in BOOLEAN /\ authenticated \in BOOLEAN
          /\ lastOpen \in {"none", "present", "missing", "error"}
          /\ failure \in {"none", "missing", "storage", "data"}
WithinBudget == /\ attempt <= MaxRetries /\ lookup <= LookupRetries
                /\ calls <= (MaxRetries + 1) * (LookupRetries + 1)
                /\ lookupWait + extentWait <= TotalWaitBound
ReleasedBetweenAttempts == phase # "data" => ~opened
NoUnverifiedReturn == phase = "done" => authenticated /\ lastOpen = "present" /\ ~Corrupt
HardErrorsStop == lastOpen = "error" => phase = "error" /\ failure = "storage"
NoFailureWithinContract == (~OtherErrors /\ ~Corrupt /\ NameBy <= LookupRetries /\ VisibleBy <= MaxRetries) => phase # "error"
FailureExplained == phase = "error" =>
    (failure = "storage" /\ lastOpen = "error") \/
    (failure = "missing" /\ lastOpen = "missing" /\ lookup = LookupRetries) \/
    (failure = "data" /\ lastOpen = "present" /\ attempt = MaxRetries)
Terminates == <>(phase \in {"done", "error"})
=============================================================================
