------------------------ MODULE ExtentVisibility ------------------------
EXTENDS Naturals, FiniteSets, EnvironmentVisibility, EnvironmentPersistence, EnvironmentProgress

\* Two readers of one published immutable extent. A view is per-open: seeing
\* the length/header says nothing about the visibility of any other region.
\* Byte/checksum computations are abstracted as authentication (no collisions).
CONSTANTS Readers, MaxRetries, VisibleBy, Mode, Corrupt, AllowedGaps
ASSUME /\ Readers # {} /\ MaxRetries \in Nat /\ VisibleBy \in Nat
       /\ Mode \in {"fixed", "v011", "v012", "v013", "skip_validation"}
       /\ Corrupt \in BOOLEAN
Regions == EnvExtentRegions
ASSUME /\ AllowedGaps \subseteq Regions /\ AllowedGaps # {}
DataRegions == EnvDataRegions
VARIABLES durable, published, pc, attempt, missing, badView, authenticated
vars == <<durable, published, pc, attempt, missing, badView, authenticated>>

Init == /\ durable = FALSE /\ published = FALSE
        /\ pc = [r \in Readers |-> "wait"]
        /\ attempt = [r \in Readers |-> 0]
        /\ missing = [r \in Readers |-> {}]
        /\ badView = [r \in Readers |-> FALSE]
        /\ authenticated = [r \in Readers |-> FALSE]

Sync == /\ ~durable /\ durable' = EnvSyncedState(TRUE)
        /\ UNCHANGED <<published, pc, attempt, missing, badView, authenticated>>
Publish == /\ durable /\ ~published /\ published' = TRUE
           /\ UNCHANGED <<durable, pc, attempt, missing, badView, authenticated>>

\* Every subset represents independent partial/noncontiguous visibility. Once
\* the visibility bound is reached a fresh open is good, unless data is corrupt.
Open(r) == /\ published /\ pc[r] \in {"wait", "reopen"}
           /\ \E gaps \in EnvMissingViews(attempt[r], VisibleBy, AllowedGaps):
                 missing' = [missing EXCEPT ![r] = gaps]
           /\ badView' = [badView EXCEPT ![r] = Corrupt]
           /\ pc' = [pc EXCEPT ![r] = "check"]
           /\ UNCHANGED <<durable, published, attempt, authenticated>>

Retryable(r) == CASE Mode = "v011" -> FALSE
                 [] Mode = "v012" -> missing[r] \cap {"length", "headerZero"} # {}
                 [] Mode = "v013" -> missing[r] \cap {"length", "headerZero", "headerPartial"} # {}
                 [] OTHER -> TRUE

Check(r) == /\ pc[r] = "check"
            /\ IF missing[r] = {} /\ ~badView[r]
                  THEN /\ pc' = [pc EXCEPT ![r] = "ok"]
                       /\ authenticated' = [authenticated EXCEPT ![r] = TRUE]
                       /\ UNCHANGED attempt
                  ELSE IF Mode = "skip_validation" /\ missing[r] \subseteq DataRegions
                          THEN /\ pc' = [pc EXCEPT ![r] = "ok"]
                               /\ UNCHANGED <<attempt, authenticated>>
                          ELSE IF attempt[r] < MaxRetries /\ Retryable(r)
                                  THEN /\ pc' = [pc EXCEPT ![r] = "reopen"]
                                       /\ attempt' = [attempt EXCEPT ![r] = @ + 1]
                                       /\ UNCHANGED authenticated
                                  ELSE /\ pc' = [pc EXCEPT ![r] = "error"]
                                       /\ UNCHANGED <<attempt, authenticated>>
            /\ UNCHANGED <<durable, published, missing, badView>>

Next == Sync \/ Publish \/ (\E r \in Readers: Open(r) \/ Check(r))
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Sync) /\ EnvWeakFairness(vars, Publish)
        /\ \A r \in Readers: EnvWeakFairness(vars, Open(r)) /\ EnvWeakFairness(vars, Check(r))

TypeOK == /\ durable \in BOOLEAN /\ published \in BOOLEAN
          /\ pc \in [Readers -> {"wait", "reopen", "check", "ok", "error"}]
          /\ attempt \in [Readers -> 0..MaxRetries]
          /\ missing \in [Readers -> SUBSET Regions]
          /\ badView \in [Readers -> BOOLEAN]
          /\ authenticated \in [Readers -> BOOLEAN]
NoUnverifiedReturn == \A r \in Readers: pc[r] = "ok" =>
                         durable /\ published /\ authenticated[r]
\* Exhaustion is legitimate: eventual visibility alone has no time bound.
NoPrematureFailure == \A r \in Readers:
                        pc[r] = "error" => attempt[r] = MaxRetries
NoFailureWithinVisibilityContract == (~Corrupt /\ VisibleBy <= MaxRetries) =>
                                       \A r \in Readers: pc[r] # "error"
EventuallyTerminates == <> (\A r \in Readers: pc[r] \in {"ok", "error"})
VisibleInBudgetSucceeds == (~Corrupt /\ VisibleBy <= MaxRetries) =>
                            <> (\A r \in Readers: pc[r] = "ok")
=============================================================================
