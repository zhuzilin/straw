------------------------ MODULE CatalogTransaction ------------------------
EXTENDS Naturals, EnvironmentPersistence, EnvironmentProgress
CONSTANTS Mutation, FailAt, ValidEvent
ASSUME /\ FailAt \in 0..5
       /\ ValidEvent \in BOOLEAN
       /\ Mutation \in {"none", "late_prepare", "cached_bypass", "missing_exit_guard",
                         "missing_active_guard", "nested_entry", "skip_recovery_sync", "skip_validation", "invalid_as_success",
                         "skip_lock", "wrong_handle", "stale_cache", "skip_rebuild"}
\* One caller catches an uncertain append error and tries cached and writing
\* APIs before returning. Only a new locked recovery may make them available.
\* FailAt 0..4 covers no write, header/body/trailer and synced-but-not-applied;
\* 5 finishes normally. ValidEvent abstracts the result of wire/semantic
\* admission and ordered index preflight. Its concrete binding is a caller duty.
VARIABLE s
vars == <<s>>
Init == s = [phase |-> "outside", active |-> FALSE, dirty |-> FALSE,
             outcome |-> "pending",
             uncertain |-> FALSE, effect |-> 0, written |-> 0, durable |-> 0,
             badOutside |-> FALSE, badWrite |-> FALSE, badCached |-> FALSE,
             badReturn |-> FALSE, badNested |-> FALSE,
             locked |-> FALSE, identity |-> 1, cache |-> 0, badContext |-> FALSE]
Available == (s.active \/ Mutation = "missing_active_guard") /\ ~s.dirty
OutsideCall == /\ s.phase = "outside"
               /\ s' = [s EXCEPT !.badOutside = Available, !.phase = "enter"]
Enter == /\ s.phase = "enter" /\ ~s.active
         /\ s' = [s EXCEPT !.active = TRUE, !.phase = "nested",
                          !.locked = (Mutation # "skip_lock"),
                          !.identity = IF Mutation = "wrong_handle" THEN 2 ELSE 1]
NestedCall == /\ s.phase = "nested"
              /\ s' = [s EXCEPT !.badNested = (Mutation = "nested_entry"), !.phase = "begin"]
RejectInvalid == /\ s.phase = "begin" /\ Available /\ ~ValidEvent /\ Mutation # "skip_validation"
                 /\ s' = [s EXCEPT !.phase = "rejected",
                                  !.outcome = IF Mutation = "invalid_as_success" THEN "ok" ELSE "invalid"]
ReturnRejected == /\ s.phase = "rejected" /\ s' = [s EXCEPT !.phase = "return"]
BeginAppend == /\ s.phase = "begin" /\ Available /\ (ValidEvent \/ Mutation = "skip_validation")
               /\ s' = [s EXCEPT !.dirty = (Mutation # "late_prepare"),
                                !.uncertain = TRUE, !.phase = "append",
                                !.badContext = (~s.locked \/ s.identity # 1)]
AppendPart == /\ s.phase = "append" /\ s.effect < 5 /\ s.effect # FailAt
              /\ s' = [s EXCEPT !.effect = @ + 1,
                       !.written = IF s.effect < 3 THEN @ + 1 ELSE @,
                       !.durable = IF s.effect = 3 THEN EnvSyncedState(s.written) ELSE @,
                       !.dirty = IF s.effect = 4 THEN FALSE ELSE @,
                       !.cache = IF s.effect = 4 /\ Mutation # "stale_cache" THEN s.written ELSE @,
                       !.uncertain = IF s.effect = 4 THEN FALSE ELSE @,
                       !.outcome = IF s.effect = 4 THEN "ok" ELSE @,
                       !.phase = IF s.effect = 4 THEN "return" ELSE @]
FailedAppend == /\ s.phase = "append" /\ s.effect = FailAt
                /\ s' = [s EXCEPT !.phase = "caught", !.outcome = "io_error"]
CaughtAndContinued == /\ s.phase = "caught"
                      /\ s' = [s EXCEPT !.badWrite = Available,
                               !.badCached = (Available \/ Mutation = "cached_bypass"),
                               !.phase = "return"]
Return == /\ s.phase = "return"
          /\ s' = [s EXCEPT !.badReturn = (s.uncertain /\ (Available \/ Mutation = "missing_exit_guard")),
                           !.active = FALSE, !.locked = FALSE, !.phase = "closed"]
\* A new attempt obtains the same stable lock and rebuilds derived state;
\* neither a previous cache nor a successful lower fsync authorizes reuse.
Restart == /\ s.phase = "closed"
           /\ s' = [s EXCEPT !.locked = s.uncertain, !.identity = 1,
                          !.phase = IF s.uncertain THEN "truncate" ELSE "done"]
Truncate == /\ s.phase = "truncate"
            /\ s' = [s EXCEPT !.written = IF s.written = 3 THEN 3 ELSE 0, !.phase = "sync"]
Recover == /\ s.phase = "sync"
           /\ s' = [s EXCEPT !.durable = IF Mutation = "skip_recovery_sync" THEN @ ELSE EnvSyncedState(s.written),
                            !.cache = IF Mutation = "skip_rebuild" THEN s.cache ELSE s.written,
                            !.dirty = FALSE, !.uncertain = FALSE, !.active = TRUE, !.phase = "retry"]
Retry == /\ s.phase = "retry" /\ Available
         /\ s' = [s EXCEPT !.active = FALSE, !.locked = FALSE, !.phase = "done", !.outcome = "ok"]
Next == OutsideCall \/ Enter \/ NestedCall \/ RejectInvalid \/ ReturnRejected \/ BeginAppend \/ AppendPart \/ FailedAppend
        \/ CaughtAndContinued \/ Return \/ Restart \/ Truncate \/ Recover \/ Retry
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Next)
TypeOK == /\ s.phase \in {"outside", "enter", "nested", "begin", "rejected", "append", "caught", "return", "closed", "truncate", "sync", "retry", "done"}
          /\ s.effect \in 0..5 /\ s.written \in 0..3 /\ s.durable \in 0..3
          /\ s.outcome \in {"pending", "ok", "invalid", "io_error"}
          /\ s.active \in BOOLEAN /\ s.dirty \in BOOLEAN /\ s.uncertain \in BOOLEAN
          /\ s.badOutside \in BOOLEAN /\ s.badWrite \in BOOLEAN /\ s.badCached \in BOOLEAN
          /\ s.badReturn \in BOOLEAN /\ s.badNested \in BOOLEAN
          /\ s.locked \in BOOLEAN /\ s.identity \in {1,2}
          /\ s.cache \in 0..3 /\ s.badContext \in BOOLEAN
NoOutsideCall == ~s.badOutside
NoWriteAfterError == ~s.badWrite
NoCachedAfterError == ~s.badCached
NoSuccessAfterError == ~s.badReturn
NoNestedEntry == ~s.badNested
InvalidHasNoEffects == ~ValidEvent => (s.written = 0 /\ s.durable = 0 /\ ~s.dirty)
RejectedReusable == s.phase = "rejected" => Available
InvalidReturnsError == (~ValidEvent /\ s.phase = "done") => s.outcome = "invalid"
ReadyDurable == (s.active /\ ~s.dirty) => s.written = s.durable
BoundCaller == ~s.badContext
CacheMatchesPrefix == (s.active /\ ~s.dirty) => s.cache = s.written
EventuallyDone == <> (s.phase = "done" /\ (ValidEvent \/ s.outcome = "invalid"))
=============================================================================
