"""Run TLC specifications and require specific negative-control failures.

Java 17+ is required. Logs, generated configurations and counterexample traces
are kept in --output. A failed TLC launch/parse/timeout is never a passing mutant.
"""

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path

from environment_manifest import MANIFEST, ROOT, environment_metadata, load_environment, tla_sources

TLC_URL = "https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar"
SPECS = tla_sources()


ENVIRONMENT_MUTANTS = {
    "composition_forgets_live_names": (
        "EnvironmentCrossComposition.tla",
        "{restored \\in SUBSET durable : current \\subseteq restored}",
        "SUBSET durable",
    ),
    "catalog_transaction_sync_lies": (
        "EnvironmentPersistence.tla",
        "EnvSyncedState(current) == current",
        "EnvSyncedState(current) == 0",
    ),
    "cross_holder_never_finishes": (
        "EnvironmentCrossLog.tla",
        'EnvCrossMayFinish(state, actor, permanent) == state = "active" /\\ actor \\notin permanent',
        "EnvCrossMayFinish(state, actor, permanent) == FALSE",
    ),
    "cross_catalog_loses_confirmed": (
        "EnvironmentCrossLog.tla",
        "EnvCrossCatalogRecovered(durable, current, initial) == EnvCrossRecovered(durable, current)",
        "EnvCrossCatalogRecovered(durable, current, initial) == {durable, current, initial}",
    ),
    "cross_queue_loses_confirmed": (
        "EnvironmentCrossLog.tla",
        "EnvCrossQueueRecovered(durable, current) == EnvCrossRecovered(durable, current)",
        "EnvCrossQueueRecovered(durable, current) == {durable, current, 0}",
    ),
    "cross_directory_sync_lies": (
        "EnvironmentCrossLog.tla",
        "EnvCrossDirectorySynced(durableNames, pack) == durableNames \\ {pack}",
        "EnvCrossDirectorySynced(durableNames, pack) == durableNames",
    ),
    "cross_no_work_fairness": (
        "EnvironmentProgress.tla",
        "EnvWeakFairness(stateVars, step) == WF_stateVars(step)",
        "EnvWeakFairness(stateVars, step) == TRUE",
    ),
    "cross_faults_never_stop": (
        "EnvironmentProgress.tla",
        "EnvFaultsEventuallyStop(stateVars, stop) == WF_stateVars(stop)",
        "EnvFaultsEventuallyStop(stateVars, stop) == TRUE",
    ),
    "wal_prefix_loses_confirmed": (
        "EnvironmentWal.tla",
        "n \\in Len(durable)..Len(written)",
        "n \\in 0..Len(written)",
    ),
    "wal_prefix_sync_lies": (
        "EnvironmentWal.tla",
        "EnvWalSynced(written) == written",
        "EnvWalSynced(written) == <<>>",
    ),
    "wal_prefix_authentication_lies": (
        "EnvironmentWal.tla",
        "\\A i \\in 1..3 : written[offset + i] = offset + i",
        "TRUE",
    ),
    "offline_roots_bindings_incomplete": (
        "EnvironmentBindings.tla",
        "EnvReferenceBindings(bindings, reference) == bindings[reference]",
        "EnvReferenceBindings(bindings, reference) == {}",
    ),
    "offline_bindings_incomplete": (
        "EnvironmentBindings.tla",
        "EnvReferenceBindings(bindings, reference) == bindings[reference]",
        'EnvReferenceBindings(bindings, reference) == {"pack", "writer"}',
    ),
    "typed_decode_lies": (
        "EnvironmentTyped.tla",
        "EnvObservedLayout(actual, requested) == actual",
        "EnvObservedLayout(actual, requested) == requested",
    ),
    "range_authentication_lies": (
        "EnvironmentAuthentication.tla",
        "EnvChunkAuthenticated(observed, expected) == observed = expected",
        "EnvChunkAuthenticated(observed, expected) == TRUE",
    ),
    "reader_ack_too_early": (
        "EnvironmentReaders.tla",
        r"EnvReaderMayAcknowledge(finished, reader) == reader \in finished",
        "EnvReaderMayAcknowledge(finished, reader) == TRUE",
    ),
    "handoff_receipt_lies": (
        "EnvironmentCommitCleanup.tla",
        "EnvHandoffRecorded(durableReceipt) == durableReceipt",
        "EnvHandoffRecorded(durableReceipt) == TRUE",
    ),
    "local_mutex_grants_busy": (
        "EnvironmentLocalAccess.tla",
        'IF held THEN {"busy"}',
        'IF held THEN {"acquired"}',
    ),
    "consumption_edges_lie": (
        "EnvironmentConsumption.tla",
        "EnvConsumptionEdges(graph, node) == graph[node]",
        "EnvConsumptionEdges(graph, node) == {}",
    ),
    "handoff_stage_lies": ("EnvironmentHandoff.tla", "EnvStagedRoot == TRUE", "EnvStagedRoot == FALSE"),
    "preparation_unlink_lies": (
        "EnvironmentPreparation.tla",
        "EnvRemovedPrivate == FALSE",
        "EnvRemovedPrivate == TRUE",
    ),
    "truncate_loses_prefix": (
        "EnvironmentIncomplete.tla",
        "EnvTruncatedLength(prefix) == prefix",
        "EnvTruncatedLength(prefix) == 0",
    ),
    "namespace_identity_lies": (
        "EnvironmentNamespace.tla",
        "EnvResolvedIdentity(targets, name) == targets[name]",
        "EnvResolvedIdentity(targets, name) == name",
    ),
    "file_sync_lies": (
        "EnvironmentPersistence.tla",
        "EnvSyncedState(current) == current",
        "EnvSyncedState(current) == FALSE",
    ),
    "directory_barrier_lies": (
        "EnvironmentPersistence.tla",
        "EnvDirectoryEntrySynced(durable, child) == durable \\cup {child}",
        "EnvDirectoryEntrySynced(durable, child) == durable",
    ),
    "discovery_open_lies": (
        "EnvironmentCatalog.tla",
        'ELSE IF managed THEN {"present", "error"}',
        'ELSE IF managed THEN {"present", "absent", "error"}',
    ),
    "sync_lies": (
        "EnvironmentPersistence.tla",
        "EnvSyncedState(current) == current",
        "EnvSyncedState(current) == TRUE",
    ),
    "wal_forgets": (
        "EnvironmentPersistence.tla",
        "THEN {durable, written} ELSE {durable}",
        "THEN {durable, written, empty} ELSE {durable, empty}",
    ),
    "lock_not_serial": ("EnvironmentCatalog.tla", r'~serialized \/ gcPhase # "scan"', "TRUE"),
    "offline_open_lies": (
        "EnvironmentOffline.tla",
        'THEN {"present", "error"} ELSE {"absent", "error"}',
        'THEN {"present", "absent", "error"} ELSE {"absent", "error"}',
    ),
    "lookup_bound_broken": (
        "EnvironmentLookup.tla",
        'THEN {"present"} ELSE {"present", "missing"}',
        'THEN {"present", "missing"} ELSE {"present", "missing"}',
    ),
    "visibility_bound_broken": (
        "EnvironmentVisibility.tla",
        "THEN {{}} ELSE SUBSET allowedGaps",
        "THEN {allowedGaps} ELSE SUBSET allowedGaps",
    ),
}


def cases():
    yield "cross_log_restore_without_retain", "CrossLogComposition", {
        "Mutation": '"restore_without_retain"',
        "@invariants": ["TypeOK", "QueueCoverage"],
    }, "QueueCoverage"

    for policy in ("recover", "abandon"):
        yield "writer_retirement_corrupt_" + policy, "WriterRetirement", {
            "Policy": json.dumps(policy),
            "CorruptResults": "TRUE",
        }, None
    yield "writer_retirement_skip_authentication", "WriterRetirement", {
        "CorruptResults": "TRUE",
        "Mutation": '"skip_authentication"',
        "@invariants": ["TypeOK", "AuthenticatedRecovery"],
    }, "AuthenticatedRecovery"

    for context in ("same", "pool", "run", "namespace", "root"):
        yield "reactivation_context_" + context, "ReactivationRace", {"CacheContext": json.dumps(context)}, None
    yield "reactivation_owner_mismatch", "ReactivationRace", {"OwnerRootMatches": "FALSE"}, None
    for mutation, invariant, extra in (
        ("wrong_cache", "AuthenticatedRoot", {"CacheContext": '"pool"'}),
        ("wrong_owner_root", "CompleteAuthorizedClosure", {"OwnerRootMatches": "FALSE"}),
        ("drop_dependency", "CompleteAuthorizedClosure", {}),
    ):
        yield "reactivation_" + mutation, "ReactivationRace", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant

    for mutation in ("ignore_epoch", "ignore_attempt"):
        yield "composition_" + mutation, "CrossLogComposition", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", "CurrentLeaseOnly"],
        }, "CurrentLeaseOnly"

    for binding in ("index", "frame", "envelope", "cache"):
        for warm in (False, True):
            yield f"read_attempt_binding_{binding}_{warm}", "ReadAttempt", {
                "Session": "TRUE",
                "InitialCache": str(warm).upper(),
                "BadBinding": json.dumps(binding),
            }, None
    for mutation, binding in (("skip_index", "index"), ("skip_frame", "frame"), ("wrong_cache", "cache")):
        yield "read_attempt_" + mutation, "ReadAttempt", {
            "Session": "TRUE",
            "InitialCache": str(mutation == "wrong_cache").upper(),
            "BadBinding": json.dumps(binding),
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", "NoUnverifiedReturn"],
        }, "NoUnverifiedReturn"

    yield "catalog_transaction_invalid_event", "CatalogTransaction", {"ValidEvent": "FALSE"}, None
    yield "catalog_transaction_skips_validation", "CatalogTransaction", {
        "ValidEvent": "FALSE",
        "Mutation": '"skip_validation"',
        "@invariants": ["TypeOK", "InvalidHasNoEffects"],
    }, "InvalidHasNoEffects"
    yield "catalog_transaction_invalid_returns_success", "CatalogTransaction", {
        "ValidEvent": "FALSE",
        "Mutation": '"invalid_as_success"',
        "@invariants": ["TypeOK", "InvalidReturnsError"],
    }, "InvalidReturnsError"
    for fail_at in range(6):
        yield f"catalog_transaction_failure_{fail_at}", "CatalogTransaction", {"FailAt": str(fail_at)}, None
    for mutation, invariant in (
        ("late_prepare", "NoWriteAfterError"),
        ("cached_bypass", "NoCachedAfterError"),
        ("missing_exit_guard", "NoSuccessAfterError"),
        ("missing_active_guard", "NoOutsideCall"),
        ("nested_entry", "NoNestedEntry"),
        ("skip_lock", "BoundCaller"),
        ("wrong_handle", "BoundCaller"),
        ("skip_recovery_sync", "ReadyDurable"),
    ):
        yield "catalog_transaction_" + mutation, "CatalogTransaction", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "catalog_transaction_stale_cache", "CatalogTransaction", {
        "Mutation": '"stale_cache"',
        "FailAt": "5",
        "@invariants": ["TypeOK", "CacheMatchesPrefix"],
    }, "CacheMatchesPrefix"
    yield "catalog_transaction_skip_rebuild", "CatalogTransaction", {
        "Mutation": '"skip_rebuild"',
        "FailAt": "4",
        "@invariants": ["TypeOK", "CacheMatchesPrefix"],
    }, "CacheMatchesPrefix"
    yield "environment_catalog_transaction_sync_lies", "CatalogTransaction", {
        "@environment_mutation": "catalog_transaction_sync_lies",
        "@invariants": ["TypeOK", "ReadyDurable"],
    }, "ReadyDurable"
    for packs in (1, 2):
        yield f"durable_handoff_packs_{packs}", "DurableHandoff", {"PackCount": str(packs)}, None
    for actor in ("reader", "checkpoint", "peer"):
        yield "durable_handoff_" + actor, "DurableHandoff", {
            "PackCount": "2",
            "Actors": "{" + json.dumps(actor) + "}",
        }, None
    yield "durable_handoff_all_actors", "DurableHandoff", {
        "PackCount": "2",
        "Actors": '{"reader", "checkpoint", "peer"}',
    }, None
    yield "durable_handoff_permanent_reader", "DurableHandoff", {
        "PackCount": "2",
        "Actors": '{"reader"}',
        "PermanentActors": '{"reader"}',
    }, None
    for mutation, invariant, extra in (
        ("queue_before_retain", "AcceptedProtected", {}),
        ("drop_dependency", "AcceptedProtected", {"PackCount": "2"}),
        ("catalog_reply_before_sync", "AcceptedProtected", {}),
        ("catalog_skip_recovery_sync", "AcceptedProtected", {}),
        ("queue_reply_before_sync", "AcknowledgedSurvives", {}),
        ("queue_skip_recovery_sync", "StageReleaseOrdered", {}),
        ("consume_before_accept_sync", "StageReleaseOrdered", {}),
        ("prune_before_retire_sync", "AcceptedProtected", {}),
        ("unlink_before_mark_sync", "DeletedAfterMark", {}),
        ("receipt_before_dirsync", "ReceiptDurable", {}),
        ("skip_barrier_when_missing", "ReceiptDurable", {}),
        ("forget_tombstone", "ReceiptDurable", {}),
        ("ignore_fence", "RetainFenced", {"Actors": '{"reader"}'}),
    ):
        yield "durable_handoff_" + mutation, "DurableHandoff", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "durable_handoff_stall_retry", "DurableHandoff", {
        "Mutation": json.dumps("stall_retry"),
        "@temporal_property": "EventuallyReclaimed",
    }, "EventuallyReclaimed"
    for mutation, invariant in (
        ("cross_catalog_loses_confirmed", "AcceptedProtected"),
        ("cross_queue_loses_confirmed", "AcknowledgedSurvives"),
        ("cross_directory_sync_lies", "ReceiptDurable"),
    ):
        yield "environment_" + mutation, "DurableHandoff", {
            "@environment_mutation": mutation,
            "@invariants": ["TypeOK", invariant],
        }, invariant
    for mutation in ("cross_no_work_fairness", "cross_faults_never_stop"):
        yield "environment_" + mutation, "DurableHandoff", {
            "@environment_mutation": mutation,
            "@temporal_property": "EventuallyReclaimed",
        }, "EventuallyReclaimed"
    yield "environment_cross_holder_never_finishes", "DurableHandoff", {
        "Actors": '{"reader"}',
        "@environment_mutation": "cross_holder_never_finishes",
        "@temporal_property": "EventuallyReclaimed",
    }, "EventuallyReclaimed"
    for count in (1, 2, 3):
        yield f"wal_recovery_{count}", "WalRecovery", {"MaxFrames": str(count)}, None
    yield "wal_recovery_corrupt", "WalRecovery", {"Corrupt": "TRUE"}, None
    for mutation, invariant, extra in (
        ("early_ack", "AcknowledgedDurable", {}),
        ("skip_payload", "AcknowledgedDurable", {}),
        ("ignore_error", "ReadyBoundary", {}),
        ("keep_tail", "ReadyBoundary", {}),
        ("ignore_corrupt", "AuthenticatedReplay", {"Corrupt": "TRUE"}),
        ("truncate_corrupt", "NoCorruptTruncation", {"Corrupt": "TRUE"}),
        ("skip_recovery_sync", "ReadyDurable", {}),
        ("wrong_seek", "ReadyBoundary", {}),
    ):
        yield "wal_recovery_" + mutation, "WalRecovery", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "wal_recovery_stall", "WalRecovery", {
        "Mutation": json.dumps("stall_recovery"),
        "@temporal_property": "EventuallyResolved",
    }, "EventuallyResolved"
    for mutation, invariant, extra in (
        ("wal_prefix_loses_confirmed", "AcknowledgedSurvives", {}),
        ("wal_prefix_sync_lies", "AcknowledgedDurable", {}),
        ("wal_prefix_authentication_lies", "AuthenticatedReplay", {"Corrupt": "TRUE"}),
    ):
        yield "environment_" + mutation, "WalRecovery", {
            "@environment_mutation": mutation,
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    for depth in (1, 4, 6):
        yield f"offline_roots_depth_{depth}", "OfflineRoots", {"Depth": str(depth)}, None
    for name, overrides in (
        ("cycle", {"Cycle": "TRUE"}),
        ("unknown", {"ScanOK": "FALSE"}),
        ("invalid", {"ValidationOK": "FALSE"}),
        ("delete_error", {"DeleteOK": "FALSE"}),
    ):
        yield "offline_roots_" + name, "OfflineRoots", overrides, None
    for mutation, invariant, extra in (
        ("omit_identity", "RecoveryReadable", {}),
        ("omit_journal", "RecoveryReadable", {}),
        ("drop_child", "DataReadable", {}),
        ("forget_binding", "RecoveryReadable", {}),
        ("ignore_scan_error", "ValidatedBeforeDeletion", {"ScanOK": "FALSE"}),
        ("ignore_validation", "ValidatedBeforeDeletion", {"ValidationOK": "FALSE"}),
        ("skip_validation", "ValidatedBeforeDeletion", {}),
        ("delete_during_scan", "ValidatedBeforeDeletion", {}),
        ("recursive_limit", "NoArtificialDepthLimit", {}),
    ):
        yield "offline_roots_" + mutation, "OfflineRoots", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "offline_roots_stall", "OfflineRoots", {
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_offline_roots_incomplete", "OfflineRoots", {
        "@environment_mutation": "offline_roots_bindings_incomplete",
        "@invariants": ["TypeOK", "RecoveryReadable"],
    }, "RecoveryReadable"
    for mode in ("file", "chain", "directory", "backtrack"):
        for canonical in ("FALSE", "TRUE"):
            yield f"offline_alias_{mode}_{canonical}", "OfflineAliases", {
                "Mode": json.dumps(mode),
                "CanonicalRoot": canonical,
            }, None
    yield "offline_alias_unknown", "OfflineAliases", {"ResolutionOK": "FALSE"}, None
    for mutation, invariant, extra in (
        ("raw_names", "AcceptedReadable", {}),
        ("final_only", "AcceptedReadable", {"Mode": json.dumps("directory")}),
        ("ancestors_only", "AcceptedReadable", {"Mode": json.dumps("backtrack")}),
        ("partial_unconditional", "AcceptedReadable", {}),
        ("unlink_target", "NoTargetSubstitution", {}),
        ("ignore_error", "NoUnknownDeletion", {"ResolutionOK": "FALSE"}),
    ):
        yield "offline_alias_" + mutation, "OfflineAliases", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "offline_alias_stall", "OfflineAliases", {
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_offline_bindings_incomplete", "OfflineAliases", {
        "Mode": json.dumps("backtrack"),
        "@environment_mutation": "offline_bindings_incomplete",
        "@invariants": ["TypeOK", "AcceptedReadable"],
    }, "AcceptedReadable"
    yield "offline_catalog", "OfflineCleanup", {}, None
    yield "offline_queues", "OfflineCleanup", {"Catalog": "FALSE", "Queues": "TRUE"}, None
    yield "offline_orphan", "OfflineCleanup", {"Catalog": "FALSE", "SharedRef": "FALSE"}, None
    yield "offline_retained", "OfflineCleanup", {"Catalog": "FALSE", "SharedRef": "FALSE", "RootRef": "TRUE"}, None
    yield "offline_active", "OfflineCleanup", {"Catalog": "FALSE", "SharedRef": "FALSE", "Stopped": "FALSE"}, None
    yield "offline_scan_error", "OfflineCleanup", {"Catalog": "FALSE", "SharedRef": "FALSE", "ScanOK": "FALSE"}, None
    yield "offline_marker_after_report", "OfflineCleanup", {
        "Catalog": "FALSE",
        "SharedRef": "FALSE",
        "AddShared": "TRUE",
    }, None
    for mutation, invariant, extra in (
        ("stat_guard", "NoOwnedDeletion", {}),
        ("ignore_error", "NoOwnedDeletion", {}),
        ("skip_recheck", "NoOwnedDeletion", {"Catalog": "FALSE", "SharedRef": "FALSE", "AddShared": "TRUE"}),
        ("ignore_queues", "NoOwnedDeletion", {"Catalog": "FALSE", "Queues": "TRUE"}),
        ("ignore_stopped", "NoLiveDeletion", {"Catalog": "FALSE", "SharedRef": "FALSE", "Stopped": "FALSE"}),
        ("delete_retained", "NoOwnedDeletion", {"Catalog": "FALSE", "SharedRef": "FALSE", "RootRef": "TRUE"}),
        (
            "ignore_scan_error",
            "NoOwnedDeletion",
            {"Catalog": "FALSE", "SharedRef": "FALSE", "RootRef": "TRUE", "ScanOK": "FALSE"},
        ),
    ):
        yield "offline_" + mutation, "OfflineCleanup", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "offline_stall", "OfflineCleanup", {
        "Catalog": "FALSE",
        "SharedRef": "FALSE",
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_offline_open_lies", "OfflineCleanup", {
        "@environment_mutation": "offline_open_lies",
        "@invariants": ["TypeOK", "NoOwnedDeletion"],
    }, "NoOwnedDeletion"
    yield "read_lookup", "ReadLookup", {}, None
    yield "read_lookup_actual_budgets", "ReadLookup", {
        "MaxRetries": "5",
        "LookupRetries": "4",
        "NameBy": "4",
        "VisibleBy": "5",
    }, None
    yield "read_lookup_no_retries", "ReadLookup", {
        "MaxRetries": "0",
        "LookupRetries": "0",
        "NameBy": "0",
        "VisibleBy": "0",
    }, None
    yield "read_lookup_missing", "ReadLookup", {"NameBy": "3"}, None
    yield "read_lookup_invisible", "ReadLookup", {"VisibleBy": "3"}, None
    yield "read_lookup_corrupt", "ReadLookup", {"Corrupt": "TRUE"}, None
    yield "read_lookup_other_errors", "ReadLookup", {"OtherErrors": "TRUE"}, None
    for mutation, invariant, extra in (
        ("early_missing", "NoFailureWithinContract", {}),
        ("retry_error", "HardErrorsStop", {"OtherErrors": "TRUE"}),
        ("extra_lookup", "WithinBudget", {"NameBy": "4"}),
        ("reset_extent", "WithinBudget", {"Corrupt": "TRUE"}),
        ("missing_success", "NoUnverifiedReturn", {}),
    ):
        yield "read_lookup_" + mutation, "ReadLookup", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
            **extra,
        }, invariant
    yield "read_lookup_stall", "ReadLookup", {
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_lookup_bound_lies", "ReadLookup", {
        "@environment_mutation": "lookup_bound_broken",
        "@invariants": ["TypeOK", "NoFailureWithinContract"],
    }, "NoFailureWithinContract"
    yield "read_attempt", "ReadAttempt", {}, None
    yield "read_attempt_session", "ReadAttempt", {"Session": "TRUE"}, None
    yield "read_attempt_cached", "ReadAttempt", {"Session": "TRUE", "InitialCache": "TRUE"}, None
    yield "read_attempt_corrupt", "ReadAttempt", {"Corrupt": "TRUE"}, None
    yield "read_attempt_exhausted", "ReadAttempt", {"VisibleBy": "3"}, None
    for mutation, invariant in (
        ("double_open", "SingleOpen"),
        ("hold_on_retry", "ReleasedBetweenAttempts"),
        ("skip_envelope", "NoUnverifiedReturn"),
        ("skip_payload", "NoUnverifiedReturn"),
    ):
        yield "read_attempt_" + mutation, "ReadAttempt", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "read_attempt_stall", "ReadAttempt", {
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_read_attempt_visibility_lies", "ReadAttempt", {
        "@environment_mutation": "visibility_bound_broken",
        "@invariants": ["TypeOK", "NoFailureWithinContract"],
    }, "NoFailureWithinContract"
    yield "typed_read", "TypedRead", {}, None
    yield "typed_read_empty", "TypedRead", {"Length": "0"}, None
    yield "typed_read_allocation_failure", "TypedRead", {"AllocationFailure": "TRUE"}, None
    for mutation, invariant in (
        ("late_check", "MismatchNoPayloadWork"),
        ("empty_bypass", "ReturnedMatches"),
        ("skip_compare", "ReturnedMatches"),
        ("reject_valid", "NoSpuriousTypeError"),
    ):
        yield "typed_read_" + mutation, "TypedRead", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "typed_read_retry_mismatch", "TypedRead", {
        "Mutation": json.dumps("retry_mismatch"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_typed_decode_lies", "TypedRead", {
        "@environment_mutation": "typed_decode_lies",
        "@invariants": ["TypeOK", "ReturnedMatches"],
    }, "ReturnedMatches"
    for length, chunk, buffer in ((1, 4, 3), (7, 3, 2), (8, 4, 1), (9, 4, 3), (7, 9, 2)):
        yield f"range_read_{length}_{chunk}_{buffer}", "RangeRead", {
            "Length": str(length),
            "ChunkBytes": str(chunk),
            "BufferBytes": str(buffer),
        }, None
    yield "range_read_faults", "RangeRead", {"ReadErrors": "TRUE", "Corruption": "TRUE"}, None
    yield "range_read_corruption", "RangeRead", {"Corruption": "TRUE"}, None
    yield "range_read_reject_valid", "RangeRead", {
        "Mutation": json.dumps("reject_valid"),
        "@invariants": ["TypeOK", "NoSpuriousError"],
    }, "NoSpuriousError"
    for mutation, invariant in (
        ("whole_buffer", "BoundedScratch"),
        ("copy_whole_buffer", "ExactOutput"),
        ("local_copy_position", "ExactOutput"),
        ("skip_auth", "NoCorruptReturn"),
        ("skip_last_chunk", "ExactOutput"),
        ("early_return", "AuthenticatedCoverage"),
        ("accept_error", "NoReturnOnFailure"),
    ):
        yield "range_read_" + mutation, "RangeRead", {
            "Mutation": json.dumps(mutation),
            "ReadErrors": "TRUE",
            "Corruption": "TRUE",
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "range_read_stall", "RangeRead", {
        "Mutation": json.dumps("stall"),
        "@temporal_property": "Terminates",
    }, "Terminates"
    yield "environment_range_authentication_lies", "RangeRead", {
        "@environment_mutation": "range_authentication_lies",
        "Corruption": "TRUE",
        "@invariants": ["TypeOK", "NoCorruptReturn"],
    }, "NoCorruptReturn"
    for readers in (1, 2, 4):
        yield f"queue_record_readers_{readers}", "QueueRecordWorkflow", {"ReaderCount": str(readers)}, None
    for history in (0, 128, 1024):
        yield f"queue_record_history_{history}", "QueueRecordWorkflow", {"History": str(history)}, None
    yield "queue_record_reader_failure", "QueueRecordWorkflow", {"ReaderFailures": "TRUE"}, None
    for mutation in ("lost_intent", "forget_last_ack", "forget_receipts"):
        overrides = {"Mutation": json.dumps(mutation), "@temporal_property": "EventuallyOutcome"}
        if mutation == "lost_intent":
            overrides["@invariants"] = ["TypeOK", "NoLostReaders", "FailedProtected", "ReceiptsRecorded", "Done"]
        if mutation == "forget_receipts":
            overrides["@invariants"] = ["TypeOK", "NoLostReaders", "FailedProtected", "NoUnconsumedStaging", "Done"]
        yield "queue_record_" + mutation, "QueueRecordWorkflow", overrides, "EventuallyOutcome"
    for mutation in ("cancel_drops_reads", "recover_drops_reads", "early_prune"):
        yield "queue_record_" + mutation, "QueueRecordWorkflow", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", "NoLostReaders"],
        }, "NoLostReaders"
    yield "queue_record_cleanup_failed_reader", "QueueRecordWorkflow", {
        "Mutation": json.dumps("cleanup_failed_reader"),
        "ReaderFailures": "TRUE",
        "@invariants": ["TypeOK", "FailedProtected"],
    }, "FailedProtected"
    yield "environment_queue_record_early_ack", "QueueRecordWorkflow", {
        "@environment_mutation": "reader_ack_too_early",
        "@invariants": ["TypeOK", "NoLostReaders"],
    }, "NoLostReaders"
    yield "environment_queue_record_receipt_lies", "QueueRecordWorkflow", {
        "@environment_mutation": "handoff_receipt_lies",
        "@invariants": ["TypeOK", "NoUnconsumedStaging"],
    }, "NoUnconsumedStaging"
    for cut in ("before_release", "after_release", "success"):
        for recover in ("FALSE", "TRUE"):
            yield f"commit_cleanup_{cut}_{recover}", "CommitCleanup", {
                "InitialCut": json.dumps(cut),
                "Recover": recover,
            }, None
    yield "commit_cleanup_restage", "CommitCleanup", {
        "InitialCut": json.dumps("after_release"),
        "Restage": "TRUE",
    }, None
    yield "commit_cleanup_reader", "CommitCleanup", {"IndependentReader": "TRUE"}, None
    for mutation in ("legacy", "forget_replay"):
        yield "commit_cleanup_" + mutation, "CommitCleanup", {
            "Mutation": json.dumps(mutation),
            "@temporal_property": "EventuallyResolved",
            "@invariants": ["TypeOK", "LateStagingPreserved", "ReaderPreserved", "PendingProtected"],
        }, "EventuallyResolved"
    for mutation, invariant, restage in (
        ("forget_error", "CompletedConsumption", False),
        ("no_receipt", "LateStagingPreserved", True),
        ("reconsume", "LateStagingPreserved", True),
        ("receipt_only", "CompletedConsumption", False),
        ("early_prune", "PendingProtected", False),
    ):
        yield "commit_cleanup_" + mutation, "CommitCleanup", {
            "Mutation": json.dumps(mutation),
            "Recover": "FALSE",
            "Restage": str(restage).upper(),
            "InitialCut": json.dumps("after_release" if restage or mutation == "receipt_only" else "before_release"),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_handoff_receipt_lies", "CommitCleanup", {
        "@environment_mutation": "handoff_receipt_lies",
        "@invariants": ["TypeOK", "CompletedConsumption"],
    }, "CompletedConsumption"
    for cut in ("ok", "error_before", "error_after", "unwind_before", "unwind_after"):
        for retire in ("FALSE", "TRUE"):
            yield f"commit_application_{cut}_{retire}", "CommitApplication", {
                "Cut": json.dumps(cut),
                "Retire": retire,
            }, None
    for mutation, cut in (
        ("late_poison", "unwind_before"),
        ("clear_error", "error_before"),
        ("skip_entry_guard", "unwind_before"),
        ("skip_rebuild", "unwind_before"),
        ("early_prune", "ok"),
        ("late_retirement", "ok"),
    ):
        yield "commit_application_" + mutation, "CommitApplication", {
            "Mutation": json.dumps(mutation),
            "Cut": json.dumps(cut),
            "@invariants": ["TypeOK", "AcceptedDataPresent"],
        }, "AcceptedDataPresent"
    for count in (2, 3, 4):
        yield f"training_resume_{count}", "TrainingResume", {"Count": str(count)}, None
    for mutation, invariant in (
        ("latest_queue_cursor", "ResumeMatchesModel"),
        ("no_replay_owner", "ReplayDataAvailable"),
        ("mixed_checkpoint", "CheckpointMatches"),
    ):
        yield "training_resume_" + mutation, "TrainingResume", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "writer_access", "WriterAccess", {}, None
    yield "writer_access_poisoned", "WriterAccess", {"PoisonedOnRetry": "TRUE"}, None
    yield "writer_access_blocking", "WriterAccess", {
        "Mutation": json.dumps("blocking"),
        "@temporal_property": "EventuallyFinished",
        "@invariants": ["TypeOK", "ExclusiveAccess", "BusyPreservesWriter", "PoisonDenied", "PublishedBeforeClose"],
    }, "EventuallyFinished"
    yield "writer_access_mutate_busy", "WriterAccess", {
        "Mutation": json.dumps("mutate_busy"),
        "@invariants": ["TypeOK", "BusyPreservesWriter"],
    }, "BusyPreservesWriter"
    yield "writer_access_ignore_poison", "WriterAccess", {
        "Mutation": json.dumps("ignore_poison"),
        "PoisonedOnRetry": "TRUE",
        "@invariants": ["TypeOK", "PoisonDenied"],
    }, "PoisonDenied"
    yield "environment_local_mutex_grants_busy", "WriterAccess", {
        "@environment_mutation": "local_mutex_grants_busy",
        "@invariants": ["TypeOK", "ExclusiveAccess"],
    }, "ExclusiveAccess"
    for readers in (1, 2, 4):
        yield f"shared_workflow_readers_{readers}", "SharedRecordWorkflow", {"ReaderCount": str(readers)}, None
    yield "shared_workflow_reader_failure", "SharedRecordWorkflow", {"ReaderFailures": "TRUE"}, None
    yield "shared_workflow_cleanup_failed_reader", "SharedRecordWorkflow", {
        "ReaderFailures": "TRUE",
        "Mutation": json.dumps("cleanup_failed_reader"),
        "@invariants": ["TypeOK", "FailedProtected"],
    }, "FailedProtected"
    for mutation, invariant in (
        ("eager_cleanup", "NoLostRetryInput"),
        ("early_continue", "NoLostReaders"),
        ("missing_result", "NoLostResult"),
        ("close_leak", "NoAbandonedOwner"),
    ):
        yield "shared_workflow_" + mutation, "SharedRecordWorkflow", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "shared_workflow_no_recipe", "SharedRecordWorkflow", {
        "Mutation": json.dumps("no_recipe"),
        "@temporal_property": "EventuallyOutcome",
        "@invariants": ["TypeOK", "NoLostReaders", "NoLostRetryInput", "NoLostResult", "NoAbandonedOwner", "Finished"],
    }, "EventuallyOutcome"
    yield "environment_workflow_edges_lie", "SharedRecordWorkflow", {
        "@environment_mutation": "consumption_edges_lie",
        "@invariants": ["TypeOK", "ReleasedHasRecipe"],
    }, "ReleasedHasRecipe"
    for failure in ("hold", "build", "complete"):
        yield "provisional_" + failure, "ProvisionalHold", {"Failure": json.dumps(failure)}, None
    for mutation, failure, invariant in (
        ("legacy", "build", "NoAbandonedHold"),
        ("late_guard", "hold", "NoAbandonedHold"),
        ("forget_error", "build", "NoAbandonedHold"),
        ("clean_complete", "complete", "NoLostComplete"),
        ("rotation", "complete", "PreparationProtected"),
        ("eager_error", "build", "PreparationProtected"),
        ("overwrite_pending", "build", "NoAbandonedHold"),
    ):
        yield "provisional_" + mutation, "ProvisionalHold", {
            "Failure": json.dumps(failure),
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    for depth in (1, 3, 12):
        yield f"consumption_depth_{depth}", "StagingConsumption", {"Depth": str(depth)}, None
    for mutation in ("legacy", "forget_replay"):
        overrides = {
            "Mutation": json.dumps(mutation),
            "@temporal_property": "EventuallyReturned",
        }
        if mutation == "legacy":
            # Legacy has no recipe at all. Keep every other invariant while
            # isolating its inability to finish after a lost response and GC.
            overrides["@invariants"] = ["TypeOK", "GraphClosed", "GraphExact", "ReaderPreserved", "ConsumeComplete"]
        yield "consumption_" + mutation, "StagingConsumption", overrides, "EventuallyReturned"
    for mutation, invariant in (("missing_node", "GraphClosed"), ("skip_cached_children", "ConsumeComplete")):
        yield "consumption_" + mutation, "StagingConsumption", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_consumption_edges_lie", "StagingConsumption", {
        "@environment_mutation": "consumption_edges_lie",
        "@invariants": ["TypeOK", "ConsumeComplete"],
    }, "ConsumeComplete"
    for api in ("records", "record_set", "publish"):
        for standalone in ("FALSE", "TRUE"):
            yield f"handoff_{api}_{standalone}", "PublicationHandoff", {
                "Api": json.dumps(api),
                "Standalone": standalone,
            }, None
    for mutation in ("legacy", "early_ack", "forget_on_error", "wrong_api", "skip_release"):
        invariant = "CompletedHandoff" if mutation == "skip_release" else "NoForgottenUnreturnedFile"
        yield "handoff_" + mutation, "PublicationHandoff", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_handoff_stage_lies", "PublicationHandoff", {
        "@environment_mutation": "handoff_stage_lies",
        "@invariants": ["TypeOK", "CompletedHandoff"],
    }, "CompletedHandoff"
    for unstable in ("FALSE", "TRUE"):
        yield "preparation_" + unstable, "PublicationPreparation", {"InitiallyUnstable": unstable}, None
    for mutation, invariant in (
        ("legacy", "NoLostReservation"),
        ("early_seal", "NoEarlySeal"),
        ("forget_after_error", "NoLostReservation"),
    ):
        yield "preparation_" + mutation, "PublicationPreparation", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_preparation_unlink_lies", "PublicationPreparation", {
        "@environment_mutation": "preparation_unlink_lies",
        "@invariants": ["TypeOK", "NoAbandonedPrivateFile"],
    }, "NoAbandonedPrivateFile"
    for policy in ("recover", "abandon"):
        yield "writer_retirement_" + policy, "WriterRetirement", {"Policy": json.dumps(policy)}, None
    for mutation, invariant in (
        ("skip_fence_check", "FenceStopsNewWrites"),
        ("death_unlock", "NoLiveWriterRetired"),
        ("skip_registry", "RegisteredBeforeOwner"),
        ("release_early", "ReconciledBeforeRelease"),
        ("receipt_early", "ReceiptSound"),
    ):
        yield "writer_retirement_" + mutation, "WriterRetirement", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "writer_lifetime", "WriterLifetime", {}, None
    yield "writer_clean_close", "WriterLifetime", {
        "Scenario": '"clean"',
        "RequireRetirement": "TRUE",
    }, None
    yield "writer_timeout_seal", "WriterLifetime", {
        "Mutation": '"timeout_seal"',
        "@invariants": ["TypeOK", "NoWriteAfterRetirement"],
    }, "NoWriteAfterRetirement"
    yield "writer_crash_safe_leak", "WriterLifetime", {"Scenario": '"crashed"'}, None
    yield "writer_dead_has_inflight", "WriterLifetime", {
        "Scenario": '"crashed"',
        "Mutation": '"death_seal"',
        "@invariants": ["TypeOK", "NoLiveWriterRetired"],
    }, "NoLiveWriterRetired"
    yield "writer_dead_late_write", "WriterLifetime", {
        "Scenario": '"crashed"',
        "Mutation": '"death_seal"',
        "@invariants": ["TypeOK", "NoWriteAfterRetirement"],
    }, "NoWriteAfterRetirement"
    # This is a known progress boundary of the current policy, not an injected
    # storage error or an implemented recovery algorithm.
    yield "writer_crash_no_retirement", "WriterLifetime", {
        "Scenario": '"crashed"',
        "RequireRetirement": "TRUE",
        "@temporal_property": "RequiredProgress",
    }, "RequiredProgress"
    for present in ("FALSE", "TRUE"):
        for unstable in ("FALSE", "TRUE"):
            yield f"gc_parent_{present}_{unstable}", "GCParentRepair", {
                "InitiallyPresent": present,
                "InitiallyUnstable": unstable,
            }, None
    for unstable in ("FALSE", "TRUE"):
        yield "gc_parent_legacy_" + unstable, "GCParentRepair", {
            "Mutation": '"legacy"',
            "InitiallyUnstable": unstable,
            "@temporal_property": "EventuallyRetired",
        }, "EventuallyRetired"
    for mutation in ("missing_is_done", "skip_repair_sync"):
        yield "gc_parent_" + mutation, "GCParentRepair", {
            "InitiallyPresent": "TRUE",
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", "ReceiptIsSafe"],
        }, "ReceiptIsSafe"
    yield "environment_gc_parent_sync_lies", "GCParentRepair", {
        "InitiallyPresent": "TRUE",
        "@environment_mutation": "sync_lies",
        "@invariants": ["TypeOK", "ReceiptIsSafe"],
    }, "ReceiptIsSafe"
    for mode in ("packed", "standalone"):
        yield "incomplete_" + mode, "IncompletePublication", {"Mode": json.dumps(mode)}, None
    yield "incomplete_larger_prefix", "IncompletePublication", {"Prefix": "4", "Tail": "1"}, None
    for mutation, mode, invariant in (
        ("wrong_floor", "packed", "PrefixPreserved"),
        ("skip_sync", "packed", "ReusableIsClean"),
        ("seal_early", "standalone", "NoEarlySeal"),
        ("forget_guard", "packed", "WriterFenced"),
        ("reject_tombstone", "standalone", "NoFalseRetirementError"),
    ):
        yield "incomplete_" + mutation, "IncompletePublication", {
            "Mode": json.dumps(mode),
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_truncate_loses_prefix", "IncompletePublication", {
        "@environment_mutation": "truncate_loses_prefix",
        "@invariants": ["TypeOK", "PrefixPreserved"],
    }, "PrefixPreserved"
    yield "namespace_alias", "NamespaceIdentity", {}, None
    yield "namespace_canonical", "NamespaceIdentity", {"ReaderName": '"file"'}, None
    yield "namespace_writer_alias", "NamespaceIdentity", {"WriterName": '"writer"'}, None
    yield "namespace_replay_owner", "NamespaceIdentity", {"ReplayOwner": "TRUE"}, None
    yield "namespace_replay_tombstone", "NamespaceIdentity", {
        "WriterName": '"writer"',
        "ReplayTombstone": "TRUE",
    }, None
    for mutation, overrides, invariant in (
        ("raw_owner", {}, "NoLostOwnedData"),
        ("raw_pack", {"WriterName": '"writer"'}, "NoLostOwnedData"),
        ("raw_replay", {"ReplayOwner": "TRUE"}, "NoLostOwnedData"),
        ("raw_fence", {}, "TombstoneFencesAliases"),
        ("raw_tombstone", {"WriterName": '"writer"', "ReplayTombstone": "TRUE"}, "TombstoneFencesAliases"),
        ("raw_receipt", {"WriterName": '"writer"'}, "ReceiptRetiresIdentity"),
    ):
        yield "namespace_" + mutation, "NamespaceIdentity", {
            **overrides,
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_namespace_identity_lies", "NamespaceIdentity", {
        "@environment_mutation": "namespace_identity_lies",
        "@invariants": ["TypeOK", "NoLostOwnedData"],
    }, "NoLostOwnedData"
    for mode in ("standalone", "packed"):
        for failure in ("before", "during", "after"):
            yield "pending_" + mode + "_" + failure, "PendingPublication", {
                "Mode": json.dumps(mode),
                "FailureAt": json.dumps(failure),
            }, None
        yield "pending_" + mode + "_corrupt", "PendingPublication", {
            "Mode": json.dumps(mode),
            "AllowCorruption": "TRUE",
        }, None
    yield "pending_confirmed_corrupt", "PendingPublication", {"FailureAt": '"after"', "AllowCorruption": "TRUE"}, None
    for mutation, invariant in (
        ("remember_late", "OneCompletedFile"),
        ("trust_failure", "DurableVerifiedReturn"),
        ("skip_retry_sync", "DurableVerifiedReturn"),
        ("skip_validation", "DurableVerifiedReturn"),
    ):
        yield "pending_" + mutation, "PendingPublication", {
            "Mutation": json.dumps(mutation),
            "AllowCorruption": "TRUE" if mutation == "skip_validation" else "FALSE",
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "environment_file_sync_lies", "PendingPublication", {
        "@environment_mutation": "file_sync_lies",
        "@invariants": ["TypeOK", "DurableVerifiedReturn"],
    }, "DurableVerifiedReturn"
    yield "publication_finalize", "PublicationFinalization", {}, None
    yield "publication_finalize_stable", "PublicationFinalization", {"InitiallyUnstable": "FALSE"}, None
    yield "publication_finalize_packed", "PublicationFinalization", {"Standalone": "FALSE"}, None
    yield "publication_finalize_packed_stable", "PublicationFinalization", {
        "Standalone": "FALSE",
        "InitiallyUnstable": "FALSE",
    }, None
    for mutation, invariant in (
        ("skip_seal", "ReleasedReclaimed"),
        ("seal_early", "NoEarlyLoss"),
        ("ignore_seal_error", "ReleasedReclaimed"),
        ("seal_packed", "PackedWritableBeforeReturn"),
    ):
        yield "publication_finalize_" + mutation, "PublicationFinalization", {
            "Mutation": json.dumps(mutation),
            "Standalone": "FALSE" if mutation == "seal_packed" else "TRUE",
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "publication_retry", "PublicationRetry", {}, None
    yield "publication_retry_empty", "PublicationRetry", {"EmptyRetry": "TRUE"}, None
    yield "publication_retry_transitive", "PublicationRetry", {"Original": "{a, b}"}, None
    for mutation in ("replace_hold", "release_on_error", "release_before_stage"):
        yield (
            "publication_retry_" + mutation,
            "PublicationRetry",
            {"Mutation": json.dumps(mutation), "@invariants": ["TypeOK", "NoLostDependency"]},
            "NoLostDependency",
        )
    yield "directory_durability", "DirectoryDurability", {}, None
    yield "directory_existing", "DirectoryDurability", {"Exists": "TRUE"}, None
    yield "directory_durable", "DirectoryDurability", {"Exists": "TRUE", "Durable": "TRUE"}, None
    yield "directory_one_level", "DirectoryDurability", {"Depth": "1"}, None
    # Four-host probe resolves 13 directories, including the stable root.
    # This exercises that observed depth, not a supported maximum path length.
    yield "directory_measured_depth", "DirectoryDurability", {"Depth": "12"}, None
    # Four-host GC parent repair completed 12 directory barriers including root:
    # 11 child-name edges. A deployment fixture, not a supported-depth limit.
    yield "directory_gc_repair_measured_depth", "DirectoryDurability", {"Depth": "11"}, None
    yield "directory_partial_existing", "DirectoryDurability", {"InitialPrefix": "1"}, None
    yield (
        "directory_partial_existing_is_durable",
        "DirectoryDurability",
        {
            "InitialPrefix": "1",
            "InitiallyUnstable": "FALSE",
            "Mutation": json.dumps("existence_is_durable"),
            "@invariants": ["TypeOK", "NoLostAcknowledgedData"],
        },
        "NoLostAcknowledgedData",
    )
    for mutation in ("existence_is_durable", "skip_root", "ignore_sync_error"):
        yield (
            "directory_" + mutation,
            "DirectoryDurability",
            {"Mutation": json.dumps(mutation), "@invariants": ["TypeOK", "NoLostAcknowledgedData"]},
            "NoLostAcknowledgedData",
        )
    yield (
        "environment_directory_barrier_lies",
        "DirectoryDurability",
        {"@environment_mutation": "directory_barrier_lies", "@invariants": ["TypeOK", "NoLostAcknowledgedData"]},
        "NoLostAcknowledgedData",
    )
    yield "gc_discovery", "GCDiscovery", {}, None
    yield "gc_discovery_remembered", "GCDiscovery", {"Remembered": "TRUE"}, None
    yield "gc_discovery_explicit", "GCDiscovery", {"Explicit": "TRUE"}, None
    yield "gc_discovery_legacy", "GCDiscovery", {"Managed": "FALSE"}, None
    for mutation in ("stat_discovery", "error_is_absence", "forget_required"):
        overrides = {"Mutation": json.dumps(mutation), "@invariants": ["TypeOK", "NoLostAcknowledgedData"]}
        if mutation == "forget_required":
            overrides["Remembered"] = "TRUE"
        yield "gc_discovery_" + mutation, "GCDiscovery", overrides, "NoLostAcknowledgedData"
    yield (
        "environment_discovery_open_lies",
        "GCDiscovery",
        {"@environment_mutation": "discovery_open_lies", "@invariants": ["TypeOK", "NoLostAcknowledgedData"]},
        "NoLostAcknowledgedData",
    )
    yield "catalog_buffer", "CatalogReadBuffer", {}, None
    yield "catalog_buffer_complete", "CatalogReadBuffer", {"TailBytes": "0"}, None
    yield "catalog_buffer_one_byte", "CatalogReadBuffer", {"BufferBytes": "1"}, None
    for mutation, invariant in (
        ("unbounded_buffer", "BoundedReadAhead"),
        ("keep_buffer_on_retry", "BufferedMatchesCursor"),
        ("use_prefetch_boundary", "ValidatedBoundary"),
        ("skip_append_seek", "AppendAtBoundary"),
    ):
        yield "catalog_buffer_" + mutation, "CatalogReadBuffer", {"Mutation": json.dumps(mutation)}, invariant
    yield "catalog_replay", "CatalogReplay", {}, None
    yield "catalog_replay_clean", "CatalogReplay", {"Tail": "FALSE"}, None
    yield (
        "catalog_replay_unconditional_invalidation",
        "CatalogReplay",
        {"Tail": "FALSE", "Mutation": json.dumps("unconditional_invalidation")},
        "NoUnneededRebuild",
    )
    for mutation, property_name in (
        ("keep_cache", "NoFalseCorruption"),
        ("keep_owners", "CoherentReady"),
        ("confirm_before_sync", "ConfirmedPrefix"),
        ("late_prepare", "CoherentReady"),
    ):
        yield "catalog_replay_" + mutation, "CatalogReplay", {"Mutation": json.dumps(mutation)}, property_name
    yield (
        "environment_reactivation_lock_not_serial",
        "ReactivationRace",
        {"@environment_mutation": "lock_not_serial", "@invariants": ["TypeOK", "NoDanglingRestore"]},
        "NoDanglingRestore",
    )
    yield "reactivation_race", "ReactivationRace", {}, None
    for mutation, invariant in (
        ("stale_snapshot", "NoDanglingRestore"),
        ("unlocked_commit", "NoDanglingRestore"),
        ("missing_source", "AuthorizedFromSource"),
        ("unbound_proof", "AuthenticatedRoot"),
    ):
        overrides = {"Mutation": json.dumps(mutation)}
        if mutation in ("stale_snapshot", "unlocked_commit"):
            # Explore the actual dangling-reference consequence, beyond the
            # earlier violation of the source-owner precondition. Positive
            # cases always check every invariant; preserve this scope in cfg.
            overrides["@invariants"] = ["TypeOK", "NoDanglingRestore"]
        yield "reactivation_race_" + mutation, "ReactivationRace", overrides, invariant
    yield "reactivation_scan", "ReactivationScan", {}, None
    for mutation, invariant in (
        ("whole_buffer", "BoundedScratch"),
        ("skip_tail", "CompleteCoverage"),
        ("ignore_corruption", "Authentication"),
        ("accept_read_error", "NoAuthorizationAfterFailure"),
    ):
        yield "reactivation_" + mutation, "ReactivationScan", {"Mutation": json.dumps(mutation)}, invariant
    yield "dependency_traversal", "DependencyTraversal", {}, None
    for mutation, invariant in (
        ("drop_successor", "CompleteClosure"),
        ("repeat_node", "NoDuplicateVisit"),
        ("incomplete_cache", "CompleteClosure"),
    ):
        yield "dependency_" + mutation, "DependencyTraversal", {"Mutation": json.dumps(mutation)}, invariant
    for name, module, invariant in (
        ("sync_lies", "GCDeletion", "DurableCompletion"),
        ("wal_forgets", "Completion", "AcknowledgedDurable"),
        ("lock_not_serial", "Ownership", "NoDanglingReferences"),
        ("visibility_bound_broken", "ExtentVisibility", "NoFailureWithinVisibilityContract"),
    ):
        yield "environment_" + name, module, {"@environment_mutation": name}, invariant
    yield "environment_composition_forgets_live_names", "CrossLogComposition", {
        "@environment_mutation": "composition_forgets_live_names",
        "@invariants": ["TypeOK", "ProtectedPresent"],
    }, "ProtectedPresent"
    yield "cross_log_shared", "CrossLogComposition", {}, None
    yield "cross_log_multiple_requests", "CrossLogComposition", {
        "RequestCount": "3",
        "PackCount": "1",
        "GenerationMax": "1",
    }, None
    yield "cross_log_holder", "CrossLogComposition", {
        "QueueCount": "1",
        "RequestCount": "1",
        "HolderCount": "1",
        "GenerationMax": "1",
    }, None
    for mutation, invariant in (
        ("accept_without_retain", "QueueCoverage"),
        ("drop_dependency", "RetainedClosure"),
        ("duplicate_accept", "AtMostOnce"),
        ("prune_before_retire", "QueueCoverage"),
        ("prune_peer", "QueueCoverage"),
        ("consume_restage", "OldHandoffNoEffect"),
        ("mark_owned", "ProtectedPresent"),
        ("receipt_before_sync", "ReclaimedDurable"),
        ("forget_tombstone", "NoLateRetain"),
        ("ignore_conflict", "CorrectRetry"),
        ("forget_recovery", "CachedConsistent"),
        ("prune_offline", "QueueCoverage"),
    ):
        yield "cross_log_" + mutation, "CrossLogComposition", {
            "Mutation": json.dumps(mutation),
            "@invariants": ["TypeOK", invariant],
        }, invariant
    yield "request_replay", "RequestReplay", {}, None
    for mutation, invariant in (
        ("validate_first", "CachedReply"),
        ("ignore_receipt", "OneAcceptance"),
        ("ignore_conflict", "CorrectReplay"),
        ("forget_recovery", "RecoveredReceipts"),
    ):
        yield "request_replay_" + mutation, "RequestReplay", {"Mutation": json.dumps(mutation)}, invariant
    yield "completion", "Completion", {}, None
    for mutation, invariant in (
        ("lease_before_receipt", "ReplayAfterRecovery"),
        ("ignore_lease", "OnlyCurrentAdmission"),
        ("ack_before_sync", "AcknowledgedDurable"),
        ("lose_recovery", "AcknowledgedDurable"),
        ("ignore_conflict", "NoConflictingReplay"),
    ):
        yield "completion_" + mutation, "Completion", {"Mutation": json.dumps(mutation)}, invariant
    yield "gc_deletion", "GCDeletion", {}, None
    yield "gc_receipt", "GCReceipt", {}, None
    for mutation, invariant in (
        ("record_before_sync", "ReceiptIsSafe"),
        ("absence_is_receipt", "SkipRequiresReceipt"),
        ("forget_tombstone", "ReceiptKeepsFence"),
        ("skip_unlink_after_stat_missing", "ReceiptIsSafe"),
    ):
        yield "gc_receipt_" + mutation, "GCReceipt", {"Mutation": json.dumps(mutation)}, invariant
    yield "gc_batch", "GCBatchDeletion", {}, None
    for mutation in ("early_sync", "missing_parent", "drop_parent"):
        yield "gc_batch_" + mutation, "GCBatchDeletion", {"Mutation": json.dumps(mutation)}, "DurableCompletion"
    yield "gc_missing_sync", "GCDeletion", {"Mutation": '"skip_missing_sync"'}, "DurableCompletion"
    yield "gc_unlink_before_mark", "GCDeletion", {"Mutation": '"unlink_before_mark"'}, "DeleteAfterMark"
    yield "ownership", "Ownership", {}, None
    for mutation, invariant in (
        ("wal_before_retain", "QueueProtection"),
        ("drop_dependency", "QueueProtection"),
        ("gc_without_lock", "NoDanglingReferences"),
        ("unlink_before_tombstone", "DeleteAfterTombstone"),
        ("reader_timeout", "ReaderProtection"),
        ("rewind_without_retain", "QueueProtection"),
    ):
        yield mutation, "Ownership", {"Mutation": json.dumps(mutation)}, invariant
    # Documented fresh-open contract: no post-commit stale view. Historical
    # policies pass this stronger environment assumption too. The delayed-view
    # cases below test defensive tolerance beyond this contract.
    for version in ("fixed", "v011", "v012", "v013"):
        yield (
            "documented_" + version,
            "ExtentVisibility",
            {"Mode": json.dumps(version), "VisibleBy": "0", "Readers": "{r1, r2}", "MaxRetries": "5"},
            None,
        )
    yield "visibility", "ExtentVisibility", {"MaxRetries": "5", "VisibleBy": "5"}, None
    yield "two_readers", "ExtentVisibility", {"Readers": "{r1, r2}", "MaxRetries": "1", "VisibleBy": "1"}, None
    yield "visibility_exhausted", "ExtentVisibility", {"VisibleBy": "3"}, None
    yield "persistent_corruption", "ExtentVisibility", {"Corrupt": "TRUE"}, None
    # Isolate the actual historical gaps, not merely any failure in old code.
    for version, gap in (("v011", "headerZero"), ("v012", "headerPartial"), ("v013", "footer")):
        yield (
            version,
            "ExtentVisibility",
            {"Mode": json.dumps(version), "AllowedGaps": "{" + json.dumps(gap) + "}"},
            "NoPrematureFailure",
        )
    yield "skip_validation", "ExtentVisibility", {"Mode": '"skip_validation"'}, "NoUnverifiedReturn"


def run_case(java, jar, output, name, module, overrides, expected, timeout, registry):
    directory = output / name
    directory.mkdir(parents=True, exist_ok=True)
    source = SPECS[module]
    shutil.copy2(source, directory / source.name)
    environment = environment_metadata(module, registry)
    for path in environment["environment_sha256"]:
        shutil.copy2(ROOT / path, directory / Path(path).name)
    overrides = dict(overrides)
    invariant_filter = overrides.pop("@invariants", None)
    temporal_property = overrides.pop("@temporal_property", None)
    environment_mutation = overrides.pop("@environment_mutation", None)
    if environment_mutation:
        filename, old, new = ENVIRONMENT_MUTANTS[environment_mutation]
        paths = [path for path in environment["environment_sha256"] if Path(path).name == filename]
        if len(paths) != 1:
            raise ValueError(f"Environment mutation is not a dependency of {module}")
        path = paths[0]
        target = directory / filename
        contents = target.read_text()
        if contents.count(old) != 1:
            raise ValueError(f"Environment mutation must change exactly one site: {environment_mutation}")
        target.write_text(contents.replace(old, new))
        environment["environment_sha256"][path] = hashlib.sha256(target.read_bytes()).hexdigest()
    config = source.with_suffix(".cfg").read_text()
    if temporal_property:
        if (
            temporal_property != expected
            or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", temporal_property)
            or re.findall(r"(?m)^\s*PROPERT(?:Y|IES)\s+(.+)$", config) != [temporal_property]
        ):
            raise ValueError("Temporal negatives require exactly one named configured property")
        if invariant_filter:
            configured = re.findall(r"(?m)^INVARIANTS (.+)$", config)
            if (
                len(configured) != 1
                or "TypeOK" not in invariant_filter
                or not set(invariant_filter) <= set(configured[0].split())
                or len(set(invariant_filter)) != len(invariant_filter)
            ):
                raise ValueError("Temporal invariant filters must retain TypeOK and select configured invariants")
    if invariant_filter:
        if (
            not expected
            or (not temporal_property and expected not in invariant_filter)
            or not all(re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", x) for x in invariant_filter)
        ):
            raise ValueError("Invariant filters require an expected negative-control violation")
        config, count = re.subn(r"(?m)^INVARIANTS .*$", "INVARIANTS " + " ".join(invariant_filter), config)
        if count != 1:
            raise ValueError("Expected one invariant configuration line")
    for key, value in overrides.items():
        config, count = re.subn(rf"(?m)^(\s*{key}\s*=).*$", lambda m: f"{m[1]} {value}", config)
        if count != 1:
            raise ValueError(f"Expected one configuration entry for {key}")
    (directory / f"{module}.cfg").write_text(config)
    command = [
        java,
        "-XX:+UseParallelGC",
        "-Xmx2g",
        "-cp",
        str(jar),
        "tlc2.TLC",
        "-workers",
        "1",
        "-seed",
        "1",
        "-fp",
        "0",
        "-config",
        f"{module}.cfg",
        module,
    ]
    started = time.monotonic()
    log_path = directory / "tlc.log"
    timed_out = False
    # TLC's disk queues and temporal graph need fast local temporary storage.
    # Keep the reproducible specs/logs in output, not gigabytes of scratch state.
    with tempfile.TemporaryDirectory(prefix="straw-tlc-") as scratch, log_path.open("w") as log:
        command[-1:-1] = ["-metadir", scratch]
        try:
            process = subprocess.run(command, cwd=directory, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
            code = process.returncode
        except subprocess.TimeoutExpired:
            timed_out = True
            code = None
    log = log_path.read_text()
    if expected is None:
        passed = code == 0 and "Model checking completed. No error has been found." in log
    elif temporal_property:
        passed = (
            code == 13
            and f"Temporal property {temporal_property} was violated." in log
            and "The following behavior constitutes a counter-example:" in log
            and "State 1:" in log
            and ("Back to state" in log or "Stuttering" in log)
        )
    else:
        passed = (
            code == 12
            and f"Invariant {expected} is violated." in log
            and "The behavior up to this point is:" in log
            and "State 1:" in log
        )
    counts = re.findall(r"([\d,]+) states generated, ([\d,]+) distinct states found", log)
    result = {
        **environment,
        "case": name,
        "module": module,
        "environment_mutation": environment_mutation,
        "case_kind": (
            "environment_negative" if environment_mutation else "algorithm_negative" if expected else "positive"
        ),
        "expected_violation": expected,
        "expected_violation_kind": "temporal" if temporal_property else "invariant" if expected else None,
        "invariant_filter": invariant_filter,
        "passed": passed,
        "exit_code": code,
        "timed_out": timed_out,
        "seconds": round(time.monotonic() - started, 3),
        "spec_sha256": hashlib.sha256((directory / source.name).read_bytes()).hexdigest(),
        "config_sha256": hashlib.sha256(config.encode()).hexdigest(),
        "log": str(log_path),
    }
    if counts:
        result.update(zip(("generated_states", "distinct_states"), (int(n.replace(",", "")) for n in counts[-1])))
    print(
        f"{'PASS' if passed else 'FAIL'} {name}: {result.get('distinct_states', '?')} states ({result['seconds']}s)",
        flush=True,
    )
    if not passed:
        print(log[-6000:], flush=True)
    return result


def shard_cases(configured, index=0, count=1):
    """Disjoint, exhaustive partitions of positive models and negative controls."""
    if count <= 0 or not 0 <= index < count:
        raise ValueError("Require 0 <= shard-index < shard-count and shard-count > 0")
    return [case for position, case in enumerate(configured) if position % count == index]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, help="Use an existing TLC jar instead of downloading it")
    parser.add_argument("--output", type=Path, default=Path("target/tla"))
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--case", action="append", choices=[c[0] for c in cases()], help="Run selected cases only")
    selection.add_argument(
        "--area",
        action="append",
        choices=sorted({SPECS[c[1]].parent.parent.name for c in cases()}),
        help="Run all positive and negative cases for selected feature areas",
    )
    parser.add_argument("--timeout", type=int, default=300, help="Per-case timeout in seconds")
    parser.add_argument("--shard-index", type=int, default=0)
    parser.add_argument("--shard-count", type=int, default=1)
    args = parser.parse_args()
    if args.shard_count <= 0 or not 0 <= args.shard_index < args.shard_count:
        parser.error("Require 0 <= shard-index < shard-count and shard-count > 0")
    if (args.case or args.area) and args.shard_count != 1:
        parser.error("Case/area filters cannot be combined with sharding")
    java = shutil.which("java")
    if not java:
        parser.error("Java 17+ is required (e.g. install openjdk-17-jre-headless)")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    jar = args.jar.resolve() if args.jar else output / "tla2tools.jar"
    if not jar.exists() and args.jar is None:
        with urllib.request.urlopen(TLC_URL, timeout=60) as response:
            data = response.read()
        jar.write_bytes(data)
    if not jar.is_file():
        parser.error("TLC jar does not exist")
    registry = load_environment()
    configured = list(cases())
    positive_models = {case[1] for case in configured if case[3] is None}
    expected_models = set(registry["models"])
    if positive_models != expected_models:
        raise ValueError(
            f"Every declared model needs a configured positive case: "
            f"missing={sorted(expected_models - positive_models)}, extra={sorted(positive_models - expected_models)}"
        )
    shutil.copy2(MANIFEST, output / MANIFEST.name)
    selected = [
        case
        for case in shard_cases(configured, args.shard_index, args.shard_count)
        if (args.case is None or case[0] in args.case)
        and (args.area is None or SPECS[case[1]].parent.parent.name in args.area)
    ]
    results = [run_case(java, jar, output, *case, args.timeout, registry) for case in selected]
    report = {
        "environment_registry_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
        "tlc_url": TLC_URL,
        "tlc_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
        "selected_areas": args.area,
        "shard": {"index": args.shard_index, "count": args.shard_count},
        "configured_cases": [case[0] for case in configured],
        "selected_cases": [case[0] for case in selected],
        "cases": results,
        "passed": all(r["passed"] for r in results),
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
