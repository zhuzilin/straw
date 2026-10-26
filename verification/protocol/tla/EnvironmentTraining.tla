------------------------ MODULE EnvironmentTraining ------------------------
EXTENDS Naturals
\* ENV-TRAINING-CHECKPOINT: an application commits a model/optimizer image and
\* its matching data position as one selected immutable checkpoint. Queue
\* processing acknowledgements may advance independently of that checkpoint.
EnvMatchingTrainingCheckpoint(model, cursor) == model = cursor
\* Data accepted after the checkpoint requires an independent replay owner
\* before processing acknowledgements make it eligible for collection.
EnvReplayProtected(held, checkpoint, cursor) ==
    \A p \in checkpoint..(cursor - 1) : p \in held
=============================================================================
