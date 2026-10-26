----------------------------- MODULE TrainingResume -----------------------------
EXTENDS Naturals, FiniteSets, TLC, EnvironmentTraining
CONSTANTS Mutation, Count
ASSUME /\ Mutation \in {"none", "latest_queue_cursor", "no_replay_owner", "mixed_checkpoint"}
       /\ Count \in Nat /\ Count >= 2
VARIABLES s
vars == <<s>>
\* One model step is already checkpointed. Later results do not yet exist and
\* therefore cannot be dependencies of that earlier immutable publication.
Init == s = [phase |-> "work", produced |-> 1, model |-> 1, consumer |-> 1,
             checkpoint |-> 1, checkpointCursor |-> 1, cursor |-> 1,
             data |-> {0}, held |-> {}]
Produce == /\ s.phase = "work" /\ s.produced < Count
           /\ s' = [s EXCEPT !.data = @ \cup {s.produced}, !.produced = @ + 1]
HoldReplay == /\ s.phase = "work" /\ Mutation # "no_replay_owner"
              /\ s' = [s EXCEPT !.held = @ \cup (s.checkpoint..(s.produced - 1))]
Train == /\ s.phase = "work" /\ s.model < s.produced /\ s.model \in s.data
         /\ s' = [s EXCEPT !.model = @ + 1]
Acknowledge == /\ s.phase = "work"
               /\ (Mutation = "no_replay_owner" \/ EnvReplayProtected(s.held, s.checkpoint, s.model))
               /\ s' = [s EXCEPT !.consumer = s.model]
Checkpoint == /\ s.phase = "work"
              /\ (Mutation = "mixed_checkpoint" \/ EnvMatchingTrainingCheckpoint(s.model, s.consumer))
              /\ s' = [s EXCEPT !.checkpoint = s.model, !.checkpointCursor = s.consumer]
ReleaseOld == /\ s.phase = "work"
              /\ s' = [s EXCEPT !.held = {p \in s.held : p >= s.checkpoint}]
Collect == /\ s.phase \in {"work", "recover", "replay"}
           /\ \E p \in ((0..(s.consumer - 1)) \cap s.data) \ s.held:
                 s' = [s EXCEPT !.data = @ \ {p}]
Crash == /\ s.phase = "work" /\ s' = [s EXCEPT !.phase = "recover"]
Restore == /\ s.phase = "recover"
           /\ s' = [s EXCEPT !.model = s.checkpoint,
                            !.cursor = IF Mutation = "latest_queue_cursor" THEN s.consumer ELSE s.checkpointCursor,
                            !.phase = "replay"]
Replay == /\ s.phase = "replay" /\ s.cursor < s.produced /\ s.cursor \in s.data
          /\ s' = [s EXCEPT !.cursor = @ + 1, !.model = @ + 1]
Next == Produce \/ HoldReplay \/ Train \/ Acknowledge \/ Checkpoint \/ ReleaseOld \/ Collect \/ Crash \/ Restore \/ Replay
Spec == Init /\ [][Next]_vars
TypeOK == /\ s.phase \in {"work", "recover", "replay"}
          /\ s.produced \in 1..Count /\ s.model \in 1..(2 * Count)
          /\ s.consumer \in 1..Count /\ s.checkpoint \in 1..Count
          /\ s.checkpointCursor \in 1..Count /\ s.cursor \in 1..Count
          /\ s.data \subseteq 0..(Count - 1) /\ s.held \subseteq 0..(Count - 1)
CheckpointMatches == s.checkpoint = s.checkpointCursor
ReplayDataAvailable == s.phase \in {"recover", "replay"} => s.checkpoint..(s.produced - 1) \subseteq s.data
ResumeMatchesModel == s.phase = "replay" => s.cursor = s.model
=============================================================================
