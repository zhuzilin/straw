# Integrating an application

[中文版](APPLICATIONS_zh.md)

straw's core owns bytes, references, durable tasks, acceptance and storage
lifetimes. Your application owns the meaning of a sample, batching, rewards,
model execution, worker placement and checkpoints. Consumers can be
preprocessing pipelines, inference services or training systems. Keep their
framework adapters in the consuming application.

## Integration boundary

| straw | Application |
|---|---|
| `Record` and versioned codec names | Encode/decode custom sample/session objects |
| `TensorRef`, dependencies, checked row reads | Tensor shapes, masks, routing and score semantics |
| Leases, receipts and durable continuation refs | Worker scheduling, RPC and retry policy |
| Consumer state and ready-batch refs | Batch planning, sharding and conversion |
| Retained owners and sealed-pack GC | Actual reader completion and checkpoint retirement |
| WAL recovery after an externally fenced restart | Model/optimizer/RNG recovery and committed checkpoint selection |

Exchange references through your existing control transport. The JSON reference
types are independent of Ray and of any application's Python classes. Publish
custom metadata with explicit tagged schemas; reject unknown versions rather
than silently importing or pickling arbitrary classes.

Keep a long-lived writer per process. Publish large tensor fields separately
and use dependencies when several result queues share them. Decode tensor
descriptors in batches with `from_record_set_many`. Persist intermediate work
incrementally instead of repeatedly copying an entire growing buffer.

In a rollout/training integration, acknowledge raw outputs after every required
consumer has finished, and ready batches only after every training rank has
finished reading. Incomplete routing/score captures must not become a durable
continuation merely because token generation finished. Retry from the last
internally consistent input. These are application validation rules.

Latch background GC failures and stop admitting work; propagate the failure to
the application supervisor. Keep uncertain storage retained for inspection.
Remote execution already in flight still requires explicit supervision/fencing.

For joint checkpoints, retain all pending/buffered/prefetched dependencies,
make model/optimizer/RNG and queue-view components durable, then publish a final
committed manifest identifying their exact versions. Keep the old committed
checkpoint intact until the new one is durable. A model-only latest marker or
queue acknowledgment cannot replace this protocol. Replaying an uncommitted
optimizer step after rollback is distinct from duplicate logical acceptance.

Application integration tests should cover their real schemas, abort/resume,
shared tensor lifetime, multi-reader completion and whole-job recovery. They
complement the [core protocol checks](VERIFICATION.md); they do not replace them.
