//! Verify the production arithmetic/controllers and their model theorems.
#![feature(proc_macro_hygiene, allocator_api)]
#[path = "../../../rust/access.rs"]
pub mod access;

#[path = "../../../rust/bounds.rs"]
pub mod bounds;

#[path = "../../../rust/catalog_append.rs"]
pub mod catalog_append;

#[path = "../../../rust/consumption.rs"]
pub mod consumption;

#[path = "../../../rust/protocol.rs"]
pub mod protocol;

#[path = "../../protocol/verus/control_plane.rs"]
pub mod control_plane;

#[path = "../../../rust/selection.rs"]
pub mod selection;

pub mod environment;

#[path = "../../gc/verus/gc_liveness.rs"]
pub mod gc_liveness;

#[path = "../../../rust/reclamation.rs"]
pub mod reclamation;

#[path = "../../../rust/stream.rs"]
pub mod stream;

#[path = "../../../rust/traversal.rs"]
pub mod traversal;

#[path = "../../../rust/replay.rs"]
pub mod replay;

#[path = "../../../rust/discovery.rs"]
pub mod discovery;

#[path = "../../../rust/directory.rs"]
pub mod directory;

#[path = "../../../rust/holding.rs"]
pub mod holding;

#[path = "../../../rust/finalization.rs"]
pub mod finalization;

#[path = "../../../rust/pending.rs"]
pub mod pending;

#[path = "../../../rust/identities.rs"]
pub mod identities;

#[path = "../../../rust/incomplete.rs"]
pub mod incomplete;

#[path = "../../../rust/typed.rs"]
pub mod typed;

#[path = "../../../rust/wal.rs"]
pub mod wal;

#[path = "../../../rust/framing.rs"]
pub mod framing;

#[path = "../../../rust/wal_recovery.rs"]
pub mod wal_recovery;

#[path = "../../../rust/wal_device.rs"]
pub mod wal_device;

#[path = "../../../rust/wal_source.rs"]
pub mod wal_source;

#[path = "../../../rust/wal_sink.rs"]
pub mod wal_sink;

#[path = "../../protocol/verus/retry_liveness.rs"]
pub mod retry_liveness;

#[path = "../../../rust/catalog_state.rs"]
pub mod catalog_state;

#[path = "serde_json.rs"]
pub mod json_specs;

#[path = "../../../rust/catalog_json.rs"]
pub mod catalog_json;

#[path = "../../../rust/catalog_log.rs"]
pub mod catalog_log;

#[path = "../../../rust/catalog_namespace.rs"]
pub mod catalog_namespace;

#[path = "../../../rust/wal_append.rs"]
pub mod wal_append;

#[path = "../../../rust/catalog_target.rs"]
pub mod catalog_target;

#[path = "../../../rust/catalog_transaction.rs"]
pub mod catalog_transaction;

#[path = "../../../rust/catalog_frame.rs"]
pub mod catalog_frame;

#[path = "../../../rust/catalog_event.rs"]
pub mod catalog_event;

#[path = "../../../rust/catalog_decode.rs"]
pub mod catalog_decode;

#[path = "collections.rs"]
pub mod collection_specs;

#[path = "../../../rust/gc_batch.rs"]
pub mod gc_batch;

#[path = "../../../rust/gc_collection.rs"]
pub mod gc_collection;

#[path = "../../../rust/gc_mark.rs"]
pub mod gc_mark;
#[path = "../../../rust/gc_receipt.rs"]
pub mod gc_receipt;

#[path = "../../../rust/closure.rs"]
pub mod closure;

#[path = "../../../rust/closure_node.rs"]
pub mod closure_node;

#[path = "../../../rust/journal_recovery.rs"]
pub mod journal_recovery;

#[path = "../../../rust/journal_state.rs"]
pub mod journal_state;

#[path = "../../../rust/journal_append.rs"]
pub mod journal_append;

#[path = "../../wal/verus/journal_equivalence.rs"]
pub mod journal_equivalence;

#[path = "../../../rust/json_update.rs"]
pub mod json_update;

#[path = "../../../rust/queue_metadata.rs"]
pub mod queue_metadata;

#[path = "../../../rust/request.rs"]
pub mod request;

#[path = "../../protocol/verus/request_history.rs"]
pub mod request_history;

#[path = "../../wal/verus/prefix_projection.rs"]
pub mod prefix_projection;

#[path = "../../wal/verus/catalog_equivalence.rs"]
pub mod catalog_equivalence;

#[path = "../../wal/verus/catalog_json_equivalence.rs"]
pub mod catalog_json_equivalence;

#[path = "../../protocol/verus/cross_log.rs"]
pub mod cross_log;

#[path = "../../../rust/gc_io.rs"]
pub mod gc_io;

#[path = "../../../rust/directory_io.rs"]
pub mod directory_io;

#[path = "../../../rust/queue_event.rs"]
pub mod queue_event;
#[path = "../../../rust/queue_admin.rs"]
pub mod queue_admin;

#[path = "../../protocol/verus/queue_event_equivalence.rs"]
pub mod queue_event_equivalence;

#[path = "../../../rust/queue_task.rs"]
pub mod queue_task;

#[path = "../../../rust/queue_reader.rs"]
pub mod queue_reader;

#[path = "../../../rust/queue_receipt.rs"]
pub mod queue_receipt;

#[path = "../../../rust/queue_state.rs"]
pub mod queue_state;
#[path = "../../../rust/queue_transition.rs"]
pub mod queue_transition;
#[path = "../../protocol/verus/queue_transition_roots.rs"]
pub mod queue_transition_roots;

#[path = "../../../rust/queue_replay.rs"]
pub mod queue_replay;

#[path = "../../../rust/queue_application.rs"]
mod queue_application;
#[path = "../../../rust/namespace_identity.rs"]
mod namespace_identity;
#[path = "../../../rust/namespace_step.rs"]
mod namespace_step;
#[path = "../../../rust/namespace_target.rs"]
mod namespace_target;
#[path = "../../../rust/queue_roots.rs"]
mod queue_roots;
#[path = "../../../rust/queue_gc.rs"]
mod queue_gc;
#[path = "../../../rust/writer_retirement.rs"]
mod writer_retirement;
#[path = "../../../rust/writer_registry.rs"]
mod writer_registry;
#[path = "../../../rust/queue_history.rs"]
pub mod queue_history;

#[path = "../../../rust/file_primitives.rs"]
pub mod file_primitives;
#[path = "../../../rust/file_adapter.rs"]
pub mod file_adapter;

#[path = "../../../rust/queue_retirement.rs"]
pub mod queue_retirement;

#[path = "../../../rust/writer_gate_flow.rs"]
pub mod writer_gate_flow;
#[path = "../../../rust/writer_ownership.rs"]
pub mod writer_ownership;
#[path = "../../../rust/journal_io.rs"]
pub mod journal_io;
#[path = "../../../rust/queue_gc_prepare.rs"]
pub mod queue_gc_prepare;
#[path = "../../../rust/catalog_primitives.rs"]
pub mod catalog_primitives;
#[path = "../../../rust/catalog_runtime.rs"]
pub mod catalog_runtime;
#[path = "../../../rust/queue_progress.rs"]
pub mod queue_progress;
#[path = "../../../rust/journal_owner.rs"]
pub mod journal_owner;
#[path = "../../../rust/journal_primitives.rs"]
pub mod journal_primitives;
#[path = "../../../rust/reference_validation.rs"]
pub mod reference_validation;

#[path = "../../../rust/staging_consumption.rs"]
pub mod staging_consumption;

#[path = "../../../rust/queue_commit.rs"]
pub mod queue_commit;

#[path = "../../../rust/catalog_gc.rs"]
pub mod catalog_gc;

#[path = "../../../rust/reference_node.rs"]
pub mod reference_node;

#[path = "../../../rust/record_manifest.rs"]
pub mod record_manifest;

#[path = "../../../rust/record_index.rs"]
pub mod record_index;

#[path = "../../../rust/extent_retry.rs"]
pub mod extent_retry;

#[path = "../../../rust/queue_handoff.rs"]
pub mod queue_handoff;
