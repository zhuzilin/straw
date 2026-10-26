------------------------- MODULE CatalogReadBuffer -------------------------
EXTENDS Naturals, EnvironmentRead, EnvironmentProgress
CONSTANTS BufferBytes, FrameBytes, CompleteFrames, TailBytes, Mutation
ASSUME /\ BufferBytes > 0 /\ FrameBytes > 0 /\ CompleteFrames > 0
       /\ TailBytes \in 0..(FrameBytes - 1)
       /\ Mutation \in {"none", "unbounded_buffer", "keep_buffer_on_retry",
                        "use_prefetch_boundary", "skip_append_seek"}
\* Byte-position model of a locked refresh of fixed-size authenticated frames
\* plus an incomplete suffix. Authentication, durable-floor/cache recovery and
\* peer concurrency are checked separately; this is not a composed replay proof.
Full == FrameBytes * CompleteFrames
InitialLength == Full + TailBytes
VARIABLES phase, length, cursor, consumed, valid, buffered, retryAvailable, appendAt
vars == <<phase, length, cursor, consumed, valid, buffered, retryAvailable, appendAt>>
Init == /\ phase = "read" /\ length = InitialLength /\ cursor = 0
        /\ consumed = 0 /\ valid = 0 /\ buffered = 0
        /\ retryAvailable = TRUE /\ appendAt = 0
Refill == /\ phase = "read" /\ buffered = 0 /\ cursor < length
          /\ \E n \in EnvReadChunkSizes(length - cursor,
                        IF Mutation = "unbounded_buffer" THEN length ELSE BufferBytes):
                /\ cursor' = cursor + n /\ buffered' = n
          /\ UNCHANGED <<phase, length, consumed, valid, retryAvailable, appendAt>>
Consume == /\ phase = "read" /\ buffered > 0 /\ consumed < length
           /\ consumed' = consumed + 1 /\ buffered' = buffered - 1
           /\ valid' = IF consumed' % FrameBytes = 0 THEN consumed' ELSE valid
           /\ UNCHANGED <<phase, length, cursor, retryAvailable, appendAt>>
\* Detecting the incomplete suffix can stop with unread read-ahead bytes.
Finish == /\ phase = "read" /\ valid = Full
          /\ valid' = IF Mutation = "use_prefetch_boundary" THEN cursor ELSE valid
          /\ phase' = "truncate" /\ buffered' = 0
          /\ UNCHANGED <<length, cursor, consumed, retryAvailable, appendAt>>
TruncateAndSync == /\ phase = "truncate" /\ length' = valid /\ phase' = "seek"
                   /\ UNCHANGED <<cursor, consumed, valid, buffered, retryAvailable, appendAt>>
SeekForAppend == /\ phase = "seek" /\ phase' = "append"
                 /\ cursor' = IF Mutation = "skip_append_seek" THEN cursor ELSE valid
                 /\ UNCHANGED <<length, consumed, valid, buffered, retryAvailable, appendAt>>
Append == /\ phase = "append" /\ appendAt' = cursor
          /\ cursor' = cursor + FrameBytes /\ length' = cursor' /\ phase' = "done"
          /\ UNCHANGED <<consumed, valid, buffered, retryAvailable>>
\* One injected read failure; retry opens/reseeks from zero and discards the
\* old reader. Actual short/error calls and unbounded failures are outside this
\* finite buffer-lifecycle check; CatalogReplay checks failed sync/cache retry.
Fail == /\ phase = "read" /\ retryAvailable /\ phase' = "retry"
        /\ retryAvailable' = FALSE
        /\ UNCHANGED <<length, cursor, consumed, valid, buffered, appendAt>>
Retry == /\ phase = "retry" /\ phase' = "read"
         /\ cursor' = 0 /\ consumed' = 0 /\ valid' = 0
         /\ buffered' = IF Mutation = "keep_buffer_on_retry" THEN buffered ELSE 0
         /\ UNCHANGED <<length, retryAvailable, appendAt>>
Next == Refill \/ Consume \/ Finish \/ TruncateAndSync \/ SeekForAppend \/ Append \/ Fail \/ Retry
Spec == Init /\ [][Next]_vars
        /\ EnvWeakFairness(vars, Refill) /\ EnvWeakFairness(vars, Consume)
        /\ EnvWeakFairness(vars, Finish) /\ EnvWeakFairness(vars, TruncateAndSync)
        /\ EnvWeakFairness(vars, SeekForAppend) /\ EnvWeakFairness(vars, Append)
        /\ EnvWeakFairness(vars, Retry)
TypeOK == /\ phase \in {"read", "truncate", "seek", "append", "done", "retry"}
          /\ length \in 0..(InitialLength + FrameBytes)
          /\ cursor \in 0..(InitialLength + FrameBytes)
          /\ consumed \in 0..InitialLength /\ valid \in 0..InitialLength
          /\ buffered \in 0..InitialLength /\ appendAt \in 0..InitialLength
          /\ retryAvailable \in BOOLEAN
BoundedReadAhead == buffered <= BufferBytes
BufferedMatchesCursor == phase = "read" => consumed + buffered = cursor
ValidatedBoundary == valid <= Full /\ valid % FrameBytes = 0
AppendAtBoundary == phase = "done" => appendAt = Full
EventuallyAppended == <> (phase = "done")
=============================================================================
