----------------------------- MODULE WalRecovery -----------------------------
EXTENDS Naturals, Sequences, EnvironmentWal, EnvironmentProgress
CONSTANTS MaxFrames, Corrupt, Mutation
ASSUME /\ MaxFrames \in 1..3 /\ Corrupt \in BOOLEAN
       /\ Mutation \in {"none", "early_ack", "skip_payload", "ignore_error",
                        "keep_tail", "ignore_corrupt", "truncate_corrupt",
                        "skip_recovery_sync", "wrong_seek", "stall_recovery"}
VARIABLES phase, written, durable, acknowledged, valid, position, base, part,
          unstable, corruptDiscarded
vars == <<phase, written, durable, acknowledged, valid, position, base, part,
          unstable, corruptDiscarded>>
ReadyPhase == IF valid = 3 * MaxFrames THEN "done" ELSE "ready"
Init == /\ phase = "recover"
        /\ written = IF Corrupt THEN <<1, 2, 0>> ELSE <<>>
        /\ durable = <<>> /\ acknowledged = {} /\ valid = 0
        /\ position = 0 /\ base = 0 /\ part = 0
        /\ unstable = TRUE /\ corruptDiscarded = FALSE
Begin == /\ phase = "ready" /\ phase' = "write"
         /\ base' = valid /\ part' = 0
         /\ UNCHANGED <<written, durable, acknowledged, valid, position, unstable, corruptDiscarded>>
WriteAt(log, offset, value) ==
    IF offset < Len(log) THEN [log EXCEPT ![offset + 1] = value]
    ELSE Append(log, value)
Write == /\ phase = "write" /\ part < 3
         /\ LET next == IF Mutation = "skip_payload" /\ part = 1 THEN 3 ELSE part + 1
            IN /\ written' = WriteAt(written, position, base + next)
               /\ part' = next /\ position' = position + 1
               /\ phase' = IF next = 3 THEN "sync" ELSE "write"
               /\ acknowledged' = IF Mutation = "early_ack" /\ next = 3
                                   THEN acknowledged \cup {base \div 3 + 1} ELSE acknowledged
         /\ UNCHANGED <<durable, valid, base, unstable, corruptDiscarded>>
Sync == /\ phase = "sync" /\ durable' = EnvWalSynced(written) /\ phase' = "reply"
        /\ UNCHANGED <<written, acknowledged, valid, position, base, part, unstable, corruptDiscarded>>
Reply == /\ phase = "reply" /\ acknowledged' = acknowledged \cup {base \div 3 + 1}
         /\ valid' = base + 3
         /\ phase' = IF base + 3 = 3 * MaxFrames THEN "done" ELSE "ready"
         /\ UNCHANGED <<written, durable, position, base, part, unstable, corruptDiscarded>>
\* Error responses poison the append. Effects may exist, including a successful
\* sync followed by a lost/error response. Recovery is required before reuse.
Fail == /\ unstable /\ phase \in {"write", "sync", "reply"}
        /\ durable' \in (IF phase = "sync" THEN {durable, written} ELSE {durable})
        /\ phase' = IF Mutation = "ignore_error" THEN "ready" ELSE "recover"
        /\ valid' = IF Mutation = "ignore_error" THEN valid ELSE 0
        /\ UNCHANGED <<written, acknowledged, position, base, part, unstable, corruptDiscarded>>
Crash == /\ unstable /\ phase # "rejected"
         /\ written' \in EnvWalRecovered(durable, written)
         /\ phase' = "recover" /\ valid' = 0 /\ position' = 0
         /\ UNCHANGED <<durable, acknowledged, base, part, unstable, corruptDiscarded>>
Scan == /\ phase = "recover" /\ Mutation # "stall_recovery"
        /\ IF Len(written) - valid < 3
           THEN /\ phase' = "truncate" /\ valid' = valid
                /\ corruptDiscarded' = corruptDiscarded
           ELSE IF EnvWalFrameValid(written, valid) \/ Mutation = "ignore_corrupt"
                THEN /\ valid' = valid + 3 /\ phase' = "recover"
                     /\ corruptDiscarded' = corruptDiscarded
                ELSE /\ phase' = IF Mutation = "truncate_corrupt" THEN "truncate" ELSE "rejected"
                     /\ valid' = valid
                     /\ corruptDiscarded' = (corruptDiscarded \/ Mutation = "truncate_corrupt")
        /\ UNCHANGED <<written, durable, acknowledged, position, base, part, unstable>>
Truncate == /\ phase = "truncate" /\ phase' = "recover_sync"
            /\ written' = IF Mutation = "keep_tail" THEN written ELSE SubSeq(written, 1, valid)
            /\ UNCHANGED <<durable, acknowledged, valid, position, base, part, unstable, corruptDiscarded>>
RecoverSync == /\ phase = "recover_sync" /\ phase' = ReadyPhase
               /\ durable' = IF Mutation = "skip_recovery_sync" THEN durable ELSE EnvWalSynced(written)
               /\ position' = IF Mutation = "wrong_seek" THEN 0 ELSE valid
               /\ UNCHANGED <<written, acknowledged, valid, base, part, unstable, corruptDiscarded>>
Stop == /\ unstable /\ unstable' = FALSE
        /\ UNCHANGED <<phase, written, durable, acknowledged, valid, position, base, part, corruptDiscarded>>
Work == Begin \/ Write \/ Sync \/ Reply \/ Scan \/ Truncate \/ RecoverSync
Next == Work \/ Fail \/ Crash \/ Stop
Spec == Init /\ [][Next]_vars /\ EnvWeakFairness(vars, Work)
        /\ EnvFaultsEventuallyStop(vars, Stop)
TypeOK == /\ phase \in {"recover", "truncate", "recover_sync", "ready", "write", "sync", "reply", "done", "rejected"}
          /\ written \in Seq(0..(3 * MaxFrames)) /\ durable \in Seq(0..(3 * MaxFrames))
          /\ acknowledged \subseteq 1..MaxFrames
          /\ valid \in 0..(3 * MaxFrames) /\ position \in 0..(3 * MaxFrames)
          /\ base \in 0..(3 * MaxFrames) /\ part \in 0..3
          /\ unstable \in BOOLEAN /\ corruptDiscarded \in BOOLEAN
AcknowledgedDurable == \A n \in acknowledged : 3 * n <= Len(durable)
AcknowledgedSurvives == \A n \in acknowledged : 3 * n <= Len(written)
ReadyBoundary == phase \in {"ready", "done"} =>
                 (valid = Len(written) /\ valid % 3 = 0 /\ position = valid)
ReadyDurable == phase \in {"ready", "done"} => written = durable
AuthenticatedReplay == phase \in {"ready", "done"} =>
                       (\A i \in 1..Len(written) : written[i] = i)
NoCorruptTruncation == ~corruptDiscarded
AppendBoundary == phase = "write" /\ part = 0 => (position = base /\ Len(written) = base)
EventuallyResolved == <> (phase \in {"done", "rejected"})
=============================================================================
