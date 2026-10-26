"""Prove production Rust controllers/bounds and unbounded model theorems.

Install Rust 1.99.1 with rustc-dev and llvm-tools, then use --download or --verus.
Contracts are cfg-gated attributes on production function bodies; no body is
translated or replaced in the positive case. Concrete I/O adapters and complete business-state call chains are not proved.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
import urllib.request
import zipfile
from pathlib import Path

from environment_manifest import MANIFEST, environment_metadata, load_environment

VERSION = "0.2026.10.18.7d82994"
SHA256 = "e9af03732d4bdd32b93a8fcf471706739c24c1473fa5086618d96c47237845dd"
URL = f"https://github.com/verus-lang/verus/releases/download/release%2F{VERSION}/verus-{VERSION}-x86-linux.zip"
ROOT = Path(__file__).resolve().parents[2]


def download(directory):
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / f"verus-{VERSION}.zip"
    if not archive.exists():
        temporary = archive.with_suffix(".partial")
        urllib.request.urlretrieve(URL, temporary)
        if hashlib.sha256(temporary.read_bytes()).hexdigest() != SHA256:
            raise ValueError("Verus archive checksum mismatch")
        temporary.replace(archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise ValueError("Verus archive checksum mismatch")
    with zipfile.ZipFile(archive) as source:
        for item in source.infolist():
            if Path(item.filename).is_absolute() or ".." in Path(item.filename).parts:
                raise ValueError("Unsafe archive path")
        source.extractall(directory)
        for item in source.infolist():
            mode = item.external_attr >> 16
            if mode:
                (directory / item.filename).chmod(mode)
    candidates = list(directory.glob("*/verus"))
    if len(candidates) != 1:
        raise ValueError("Expected one Verus executable")
    return candidates[0].resolve()


SOURCES = (
    "rust/access.rs",
    "rust/catalog_append.rs",
    "rust/catalog_gc.rs",
    "rust/catalog_runtime.rs",
    "rust/catalog_decode.rs",
    "rust/catalog_event.rs",
    "rust/catalog_frame.rs",
    "rust/catalog_json.rs",
    "rust/catalog_log.rs",
    "rust/catalog_namespace.rs",
    "rust/catalog_target.rs",
    "rust/catalog_transaction.rs",
    "rust/catalog_state.rs",
    "rust/wal.rs",
    "rust/framing.rs",
    "rust/wal_recovery.rs",
    "rust/journal_recovery.rs",
    "rust/journal_state.rs",
    "rust/journal_append.rs",
    "rust/journal_io.rs",
    "rust/journal_owner.rs",
    "rust/json_update.rs",
    "rust/queue_metadata.rs",
    "rust/queue_event.rs",
    "rust/queue_application.rs",
    "rust/queue_commit.rs",
    "rust/queue_retirement.rs",
    "rust/queue_roots.rs",
    "rust/queue_gc.rs",
    "rust/queue_gc_prepare.rs",
    "rust/queue_progress.rs",
    "rust/reference_validation.rs",
    "rust/reference_node.rs",
    "rust/record_manifest.rs",
    "rust/record_index.rs",
    "rust/extent_retry.rs",
    "rust/queue_handoff.rs",
    "rust/namespace_identity.rs",
    "rust/namespace_step.rs",
    "rust/namespace_target.rs",
    "rust/writer_gate_flow.rs",
    "rust/writer_ownership.rs",
    "rust/writer_retirement.rs",
    "rust/writer_registry.rs",
    "rust/queue_history.rs",
    "rust/queue_admin.rs",
    "rust/queue_replay.rs",
    "rust/queue_task.rs",
    "rust/queue_reader.rs",
    "rust/queue_receipt.rs",
    "rust/queue_state.rs",
    "rust/queue_transition.rs",
    "verification/protocol/verus/queue_transition_roots.rs",
    "rust/request.rs",
    "rust/wal_append.rs",
    "rust/wal_device.rs",
    "rust/file_adapter.rs",
    "rust/wal_source.rs",
    "rust/wal_sink.rs",
    "rust/bounds.rs",
    "rust/typed.rs",
    "rust/consumption.rs",
    "rust/staging_consumption.rs",
    "rust/protocol.rs",
    "rust/selection.rs",
    "rust/reclamation.rs",
    "rust/gc_batch.rs",
    "rust/gc_collection.rs",
    "rust/gc_io.rs",
    "rust/gc_mark.rs",
    "rust/gc_receipt.rs",
    "rust/closure.rs",
    "rust/closure_node.rs",
    "rust/stream.rs",
    "rust/traversal.rs",
    "rust/replay.rs",
    "rust/discovery.rs",
    "rust/directory.rs",
    "rust/directory_io.rs",
    "rust/holding.rs",
    "rust/finalization.rs",
    "rust/pending.rs",
    "rust/identities.rs",
    "rust/incomplete.rs",
    "verification/protocol/verus/control_plane.rs",
    "verification/protocol/verus/cross_log.rs",
    "verification/protocol/verus/retry_liveness.rs",
    "verification/protocol/verus/request_history.rs",
    "verification/protocol/verus/queue_event_equivalence.rs",
    "verification/common/verus/environment.rs",
    "verification/gc/verus/gc_liveness.rs",
    "verification/wal/verus/prefix_projection.rs",
    "verification/wal/verus/catalog_equivalence.rs",
    "verification/wal/verus/journal_equivalence.rs",
    "verification/wal/verus/catalog_json_equivalence.rs",
    "verification/common/verus/verus.rs",
)

# This is a library trust boundary, kept out of the assumption-free straw code.
# Only the listed foreign types, trait declaration and library primitives are permitted.
JSON_SPECS = "verification/common/verus/serde_json.rs"
JSON_METHODS = {
    "<serde_json::Number as From<u64>>::from",
    "serde_json::Map::<String, Value>::new",
    "<Value as Clone>::clone",
    "<Value as PartialEq>::eq",
    "serde_json::Map::<String, Value>::get::<Q>",
    "serde_json::Map::<String, Value>::insert",
    "serde_json::Map::<String, Value>::get_mut::<Q>",
    "serde_json::Map::<String, Value>::remove::<Q>",
    "serde_json::Map::<String, Value>::retain::<F>",
    "serde_json::from_slice::<T>",
    "serde_json::to_vec::<T>",
    "Value::as_u64",
    "<str as serde_json::value::Index>::index_into",
    "<str as serde_json::value::Index>::index_into_mut",
    "Value::as_bool",
    "Value::is_null",
    "Value::as_str",
    "Value::as_array",
    "Value::as_object",
    "serde_json::Map::<String, Value>::iter",
    "<serde_json::map::Iter<'a> as Iterator>::next",
}


JSON_CANONICAL_SPEC_SHA256 = "add5575c36439492b667c0bbba8a5ba808f055ce6d749cef23d58bc1560b8106"
JSON_CANONICAL_LAWS = [
    "value_roundtrip_content",
    "array_content_congruence",
    "string_content_congruence",
    "unsigned_content_congruence",
]


def audit_json_specs(code):
    marker = "// Separately audited pinned-serde Value laws; not straw algorithms.\n"
    if code.count(marker) != 1:
        raise ValueError("Expected exactly the reviewed canonical Value law block")
    prefix, canonical = code.split(marker)
    canonical = marker + canonical
    if hashlib.sha256(canonical.encode()).hexdigest() != JSON_CANONICAL_SPEC_SHA256:
        raise ValueError("Canonical Value library contracts changed without review")
    if (
        re.findall(r"pub proof fn (\w+)", canonical) != JSON_CANONICAL_LAWS
        or canonical.count("verifier::external_body") != 4
    ):
        raise ValueError("Unexpected canonical Value trusted surface")
    code = prefix
    # Narrow foreign Value/Map laws, never straw functions.
    law = r"#\[verifier::external_body\]\s+pub proof fn value_encoding\(value: Value, raw: Seq<u8>\)\s+requires serializes\(&value, raw\),\s+ensures parses\(raw, value\),\s+\{\}"
    if len(re.findall(law, code)) != 1:
        raise ValueError("Only the explicit Value serialization relation law is permitted")
    code = re.sub(law, "", code)
    consistency = r"#\[verifier::external_body\]\s+pub proof fn value_parse_consistency\(raw: Seq<u8>, left: Value, right: Value, depth: nat\)\s+requires parses\(raw, left\), parses\(raw, right\),\s+ensures observe\(left, depth\) == observe\(right, depth\),\s+\{\}"
    if len(re.findall(consistency, code)) != 1:
        raise ValueError("Only the explicit Value observation consistency law is permitted")
    code = re.sub(consistency, "", code)
    unique = r"#\[verifier::external_body\]\s+pub proof fn map_entries_unique\(map: serde_json::Map<String, Value>\)\s+ensures entries\(map\)\.map\(\|i: int, pair: \(String, Value\)\| pair\.0@\)\.no_duplicates\(\),\s+\{\}"
    if len(re.findall(unique, code)) != 1:
        raise ValueError("Only the explicit serde_json Map key uniqueness law is permitted")
    code = re.sub(unique, "", code)
    unfold = r"pub proof fn map_pairs_unfold\(items: Seq<\(String, Value\)>\)\s+ensures map_pairs\(items\) == if items.len\(\) == 0 \{ Map::empty\(\) \}\s+else \{ map_pairs\(items.drop_last\(\)\).insert\(items.last\(\).0, items.last\(\).1\) \},\s+\{ reveal\(map_pairs\); \}"
    if len(re.findall(unfold, code)) != 1:
        raise ValueError("Expected the checked map_pairs definition-unfolding lemma")
    code = re.sub(unfold, "", code)
    methods = re.findall(r"pub assume_specification[^\[]*\[([^\]]+)\]", code)
    if {method.strip() for method in methods} != JSON_METHODS or len(methods) != len(JSON_METHODS):
        raise ValueError("Unexpected serde_json trusted method surface")
    wrappers = re.findall(
        r"#\[verifier::external_body\](?:\s*#\[verifier::(?:accept|reject)_recursive_types\([KV]\)\])*\s*pub struct (Ex\w+)",
        code,
    )
    if set(wrappers) != {"ExNumber", "ExMap", "ExIter", "ExError"} or code.count("verifier::external_body") != 4:
        raise ValueError("Only the four opaque serde_json types may have external bodies")
    if code.count("external_type_specification") != 5 or not re.search(
        r"#\[verifier::external_type_specification\]\s+pub struct ExValue\(Value\);", code
    ):
        raise ValueError("Value must expose its actual enum; only Number/Map/Iter/Error remain opaque")
    if re.search(r"\b(?:assume|admit|axiom|external_fn_specification)\b|(?<!spec )\bfn\b", code):
        raise ValueError("serde_json specifications must not contain executable or admitted straw functions")
    if (
        re.findall(r"pub trait (\w+)", code) != ["ExDeserialize", "ExSerialize"]
        or code.count("external_trait_specification") != 2
    ):
        raise ValueError("Only the serde Deserialize/Serialize marker declarations are permitted")


FILE_SPECS = "rust/file_primitives.rs"
FILE_SPEC_SHA256 = "6afcb590c78acd6b9cc174f68c1ebd2dca53be6a11c33ad7b961a376601af611"
FILE_PRIMITIVES = {
    "length",
    "buffer",
    "unbuffer",
    "read",
    "read_buffered",
    "truncate",
    "seek",
    "position",
    "write",
    "sync",
}


def audit_file_specs(code):
    # Pin the reviewed std-call bodies and contracts together. Any change needs
    # an explicit primitive-boundary review before updating this digest.
    if hashlib.sha256(code.encode()).hexdigest() != FILE_SPEC_SHA256:
        raise ValueError("Native I/O primitive contracts or bodies changed without review")
    if set(re.findall(r"pub fn (\w+)", code)) != FILE_PRIMITIVES:
        raise ValueError("Unexpected native I/O trusted function")
    if code.count("verifier::external_body") != 14 or code.count("verifier::external_type_specification") != 4:
        raise ValueError("Unexpected native I/O trusted surface")
    if re.search(r"\b(assume|assume_specification|admit)\b", code):
        raise ValueError("Native I/O boundary cannot add arbitrary proof axioms")


CATALOG_SPECS = "rust/catalog_primitives.rs"
CATALOG_SPEC_SHA256 = "743a29289f71bb04b2785c01b91403a69925927acc30d74394dcee3972732c12"
CATALOG_PRIMITIVES = {"open", "identity", "clone_lock", "acquire", "release"}


def audit_catalog_specs(code):
    # Pin the five OS calls, logical descriptor binding, and initial permission
    # mint together; no catalog algorithm is inside this trusted surface.
    if hashlib.sha256(code.encode()).hexdigest() != CATALOG_SPEC_SHA256:
        raise ValueError("Catalog OS primitive contracts or bodies changed without review")
    if code.count("verifier::external_body") != 5 or "external_type_specification" in code:
        raise ValueError("Unexpected catalog OS trusted surface")
    if code.count("Tracked::assume_new()") != 1 or code.count("Ghost::assume_new()") != 1:
        raise ValueError("Expected only the opened-file permission and initial lock flag")
    if re.search(r"\b(assume|assume_specification|admit)\b", code):
        raise ValueError("Catalog OS boundary cannot admit straw algorithms")


JOURNAL_SPECS = "rust/journal_primitives.rs"
JOURNAL_SPEC_SHA256 = "b6453038c4d56fa370f6e407c5202ee2f2494539af9145872b80ad7ea8760628"


def audit_journal_specs(code):
    # One OpenOptions call creates one descriptor permission. Journal recovery,
    # Run admission, barriers and error quarantine stay in checked Rust.
    if hashlib.sha256(code.encode()).hexdigest() != JOURNAL_SPEC_SHA256:
        raise ValueError("Journal open primitive changed without review")
    if code.count("verifier::external_body") != 3 or code.count("verifier::external_type_specification") != 2:
        raise ValueError("Expected only Path/PathBuf and the Journal open primitive")
    if code.count("Tracked::assume_new()") != 1 or code.count("Ghost::assume_new()") != 1:
        raise ValueError("Expected only the opened-file permission and location")
    if re.findall(r"pub\(crate\) fn (\w+)", code) != ["open"]:
        raise ValueError("Unexpected Journal trusted function")
    if re.search(r"\b(assume|assume_specification|admit)\b", code):
        raise ValueError("Journal OS boundary cannot admit straw algorithms")


COLLECTION_SPECS = "verification/common/verus/collections.rs"
COLLECTION_METHODS = {
    "std::mem::replace::<T>",
    "HashMap::<K, V, S, A>::into_iter",
    "<IntoIter<K, V, A> as Iterator>::next",
    "HashSet::<K, S, A>::into_iter",
    "<SetIntoIter<K, A> as Iterator>::next",
}


def audit_collection_specs(code):
    bulk = r"#\[verifier::external_body\]\s+pub proof fn btree_bulk<T: Ord>\(remaining: Seq<T>, value: std::collections::BTreeSet<T>\)\s+requires vstd::laws_cmp::obeys_cmp::<T>\(\),\s+vstd::std_specs::iter::FromIteratorSpec::from_iter_ensures\(remaining, value\),\s+ensures value@ == remaining.to_set\(\),\s+\{\}"
    if len(re.findall(bulk, code)) != 1:
        raise ValueError("Expected only the named BTreeSet bulk-construction library law")
    code = re.sub(bulk, "", code)
    methods = re.findall(r"pub assume_specification[^\[]*\[([^\]]+)\]", code)
    if {method.strip() for method in methods} != COLLECTION_METHODS or len(methods) != len(COLLECTION_METHODS):
        raise ValueError("Unexpected HashMap ownership iterator trusted method surface")
    wrappers = re.findall(
        r"#\[verifier::external_body\](?:\s*#\[verifier::(?:accept|reject)_recursive_types\([KVA]\)\])*\s*pub struct (Ex\w+)",
        code,
    )
    if wrappers != ["ExIntoIter", "ExSetIntoIter"] or code.count("verifier::external_body") != 2:
        raise ValueError("Only the opaque std HashMap/HashSet ownership iterators may have external bodies")
    if re.search(r"\b(?:assume|admit|axiom|external_fn_specification)\b|(?<!spec )\bfn\b", code):
        raise ValueError("HashMap specifications must not contain executable or admitted straw functions")
    implementations = re.findall(r"impl<[^>]+>\s+([^{}]+)\{", code)
    if implementations:
        raise ValueError("Collection specifications must not add trait implementations")


def json_dependency(directory):
    """Use the locked real library built by the verifier's Rust compiler."""
    command = [
        "cargo",
        "+1.99.1",
        "build",
        "--locked",
        "--lib",
        "--message-format=json",
        "--target-dir",
        str(directory / "dependencies"),
    ]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=600)
    (directory / "dependency-build.log").write_text(result.stdout + result.stderr)
    result.check_returncode()
    artifacts = []
    json_features = None
    serde_artifacts = []
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item["target"]["name"] == "serde_json":
            json_features = item["features"]
            artifacts.extend(Path(path) for path in item["filenames"] if path.endswith(".rlib"))
        if item.get("reason") == "compiler-artifact" and item["target"]["name"] == "serde":
            serde_artifacts.extend(Path(path) for path in item["filenames"] if path.endswith(".rlib"))
    if len(artifacts) != 1 or len(serde_artifacts) != 1:
        raise ValueError("Expected exactly one locked serde_json and serde rlib")
    if json_features is not None and "preserve_order" in json_features:
        raise ValueError("Value equality/observation contracts require the pinned sorted-map configuration")
    if json_features is None or "float_roundtrip" not in json_features:
        raise ValueError("Value serialization relation requires the float_roundtrip library feature")
    artifact = artifacts[0]
    return (
        artifact,
        serde_artifacts[0],
        {
            "compiler": "1.99.1",
            "command": command,
            "input_sha256": {
                name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in ("Cargo.toml", "Cargo.lock")
            },
            "library_sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
            "serde_library_sha256": hashlib.sha256(serde_artifacts[0].read_bytes()).hexdigest(),
            "trusted_specification": JSON_SPECS,
            "trusted_methods": sorted(JSON_METHODS),
            "trusted_laws": ["value_encoding", "value_parse_consistency", "map_entries_unique", *JSON_CANONICAL_LAWS],
            "canonical_value_laws_sha256": JSON_CANONICAL_SPEC_SHA256,
            "verified_lemmas": ["map_pairs_unfold"],
            "features": json_features,
            "opaque_types": 4,
            "transparent_types": ["serde_json::Value"],
        },
    )


# The full production crate is always verified. These mutations intentionally
# invalidate global model invariants; check their immediate consequence in a
# named theorem or actual affected body rather than exploring inconsistent histories.
# Preserve the filter in the report so negative evidence has an explicit scope.
NEGATIVE_FUNCTIONS = {
    "control_retry_does_not_restart": "stable_step",
    "control_effect_skips_step": "stable_step",
    "environment_control_no_retry_fairness": "fair_retry_event",
    "environment_control_no_io_fairness": "fair_io_event",
    "environment_control_errors_forever": "stable_response_at",
    "environment_control_crashes_forever": "stable_response_at",
    "gc_retry_does_not_restart": "stable_step",
    "environment_gc_sync_lies": "history_valid",
    "environment_gc_no_retry_fairness": "fair_retry_event",
    "environment_gc_errors_forever": "stable_response_at",
}


# Replay controls check the changed production body (or immediate consumer of
# a weakened device/target contract). Other modules' invariants need not be
# re-proved under an intentionally inconsistent adapter in a negative run.
REPLAY_NEGATIVE_FUNCTIONS = {
    "replay_rejects_legal_partial": "read_frame",
    "replay_rejects_valid_header": "read_frame",
    "replay_rejects_empty_frame": "read_frame",
    "replay_rejects_valid_trailer": "read_frame",
    "recovery_rejects_valid_cursor": "recover",
    "recovery_rejects_usable_sequence": "recover",
    "recovery_rejects_confirmed_boundary": "recover",
    "replay_mislabels_trailer_error": "read_frame",
    "recovery_mislabels_frame_io_error": "recover",
    "replay_driver_ignores_sync_error": "finish",
    "replay_driver_ignores_seek_error": "finish",
    "environment_replay_hash_unbound": "read_frame",
    "environment_replay_apply_drops_record": "recover",
    "environment_replay_wrong_read_bytes": "read_frame",
    "environment_replay_wrong_seek_identity": "finish",
    "environment_replay_seek_lies": "finish",
    "environment_replay_sync_lies": "finish",
    "replay_driver_skips_sync": "finish",
    "replay_driver_skips_seek": "finish",
    "replay_driver_skips_truncate": "finish",
    "replay_driver_keeps_buffer": "finish",
    "replay_driver_skips_floor": "recover",
    "replay_driver_skips_apply": "recover",
    "replay_driver_wrong_sequence": "recover",
    "replay_driver_wrong_header_hash_span": "read_frame",
    "replay_driver_omits_header_from_frame_hash": "read_frame",
    "replay_driver_truncates_corrupt_frame": "read_frame",
    "replay_driver_accepts_bad_partial_magic": "read_frame",
    "replay_driver_skips_trailer_validation": "read_frame",
}
NEGATIVE_FUNCTIONS.update(REPLAY_NEGATIVE_FUNCTIONS)
NEGATIVE_MODULES = dict.fromkeys(REPLAY_NEGATIVE_FUNCTIONS, "wal_recovery")


CATALOG_TARGET_NEGATIVES = {
    "catalog_log_keeps_rebuild_indexes": ("catalog_log", "begin_replay"),
    "catalog_log_keeps_uncertain_plan": ("catalog_log", "begin_replay"),
    "catalog_log_forgets_confirmed_floor": ("catalog_log", "begin_replay"),
    "catalog_log_confirms_partial_apply": ("catalog_log", "confirm_replay"),
    "catalog_log_confirms_wrong_end": ("catalog_log", "confirm_replay"),
    "catalog_log_confirms_wrong_sequence": ("catalog_log", "confirm_replay"),
    "catalog_log_keeps_replay_cursor": ("catalog_log", "replay_record"),
    "catalog_log_changes_replay_history": ("catalog_log", "replay_record"),
    "catalog_log_changes_partial_history": ("catalog_log", "replay_record"),
    "catalog_log_appends_before_recovery": ("catalog_log", "begin_append"),
    "catalog_log_changes_pending_operations": ("catalog_log", "begin_append"),
    "catalog_log_rejects_valid_append": ("catalog_log", "begin_append"),
    "catalog_log_skips_post_sync_apply": ("catalog_log", "finish_append"),
    "catalog_log_skips_append_confirmation": ("catalog_log", "finish_append"),
    "catalog_confirms_before_wal_write": ("catalog_log", "append_to"),
    "catalog_ignores_wal_write_error": ("catalog_log", "append_to"),
    "catalog_writes_without_admission": ("catalog_log", "append_to"),
    "catalog_target_skips_apply": ("catalog_target", "apply"),
    "catalog_target_wrong_payload": ("catalog_target", "apply"),
    "catalog_target_accepts_decode_error": ("catalog_target", "apply"),
    "catalog_target_ignores_apply_error": ("catalog_target", "apply"),
    "catalog_log_records_wrong_payload": ("catalog_log", "replay_record"),
    "catalog_log_plan_wrong_payload": ("catalog_log", "begin_append"),
    "catalog_log_applies_wrong_plan_bytes": ("catalog_log", "finish_append"),
    "environment_target_loses_validity": ("wal_recovery", "recover"),
}
for _name, (_module, _function) in CATALOG_TARGET_NEGATIVES.items():
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CATALOG_EVENT_NEGATIVES = {
    "catalog_event_wrong_id_length": "valid_hex",
    "catalog_event_skips_final_digit": "valid_hex",
    "catalog_event_accepts_uppercase": "valid_hex",
    "catalog_event_ignores_staged_prefix": "valid_staged_key",
    "catalog_event_receipt_without_owners": "valid_receipts",
    "catalog_event_receipt_retains_owner": "valid_receipts",
    "catalog_event_receipt_skips_id": "valid_receipts",
    "catalog_event_ignores_child_names": "valid_staging_edges",
    "catalog_event_ignores_node_name": "valid_staging_edges",
    "catalog_event_ignores_graph_compatibility": "valid_staging_edges",
    "catalog_event_rejects_absent_receipts": "valid_receipts",
    "catalog_event_accepts_short_id": "valid_hex",
}
for _name, _function in CATALOG_EVENT_NEGATIVES.items():
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "catalog_event"


CATALOG_DECODE_MUTATIONS = (
    (
        "catalog_decode_omits_last_path",
        "resolve_paths",
        "rust/catalog_decode.rs",
        "while i < names.len() {",
        "while i < names.len() && i != names.len() - 1 {",
    ),
    (
        "catalog_decode_uses_raw_identity",
        "resolve_paths",
        "rust/catalog_decode.rs",
        "let path = resolver.resolve(&names[i], encoding)?;",
        "let path = names[i].clone();",
    ),
    (
        "catalog_decode_loses_path_set",
        "resolve_paths",
        "rust/catalog_decode.rs",
        "Ok(R::Keys::from_values(result))",
        "Ok(R::Keys::from_values(Vec::new()))",
    ),
    (
        "catalog_decode_release_becomes_owner",
        "append_owners",
        "rust/catalog_decode.rs",
        "None => Operation::Release(owner),",
        "None => Operation::Owner(owner, BTreeSet::new()),",
    ),
    (
        "catalog_decode_flips_sealing",
        "append_packs",
        "rust/catalog_decode.rs",
        "Operation::Pack(key, sealed)",
        "Operation::Pack(key, !sealed)",
    ),
    (
        "catalog_decode_swaps_mark_receipt",
        "append_deletions",
        "rust/catalog_decode.rs",
        "let op = if reclaimed {",
        "let op = if !reclaimed {",
    ),
    (
        "catalog_decode_drops_graph_children",
        "graph",
        "rust/catalog_decode.rs",
        "result.insert(key, children);",
        "result.insert(key, Vec::new());",
    ),
    (
        "catalog_decode_drops_edge_children",
        "graph_operations",
        "rust/catalog_decode.rs",
        "Operation::Edges(key, children)",
        "Operation::Edges(key, Vec::new())",
    ),
    (
        "catalog_decode_skips_receipt_policy",
        "construct",
        "rust/catalog_decode.rs",
        "if !crate::catalog_event::valid_receipts(&wire) {",
        "if false {",
    ),
    (
        "catalog_decode_skips_graph_admission",
        "construct",
        "rust/catalog_decode.rs",
        "if !crate::catalog_event::valid_staging_edges(known, &additions) {",
        "if false {",
    ),
    (
        "catalog_decode_accepts_unknown_encoding",
        "construct",
        "rust/catalog_decode.rs",
        "if matches!(encoding, Encoding::Unsupported) {",
        "if false {",
    ),
    (
        "catalog_decode_marks_as_reclaimed",
        "construct",
        "rust/catalog_decode.rs",
        "append_deletions(resolver, wire.deleted, encoding, false, &mut operations)",
        "append_deletions(resolver, wire.deleted, encoding, true, &mut operations)",
    ),
    (
        "catalog_decode_loses_receipts",
        "construct",
        "rust/catalog_decode.rs",
        "Operation::Receipts(wire.receipts)",
        "Operation::Receipts(Vec::new())",
    ),
    (
        "catalog_decode_receipts_before_updates",
        "construct",
        "rust/catalog_decode.rs",
        "operations.push(receipts);",
        "operations.insert(0, receipts);",
    ),
    (
        "catalog_decode_spurious_receipt_error",
        "decode",
        "rust/catalog_decode.rs",
        "let result = construct(resolver, wire, known);",
        "let result = Err(Failure::Receipts);",
    ),
    (
        "catalog_decode_misclassifies_schema_error",
        "decode",
        "rust/catalog_decode.rs",
        "Err(error) => return Err(Failure::Wire(error)),",
        "Err(_) => return Err(Failure::Receipts),",
    ),
    (
        "environment_catalog_set_loses_key",
        "resolve_paths",
        "rust/catalog_decode.rs",
        "ensures result@ == values@.to_set(),",
        "ensures result@.subset_of(values@.to_set()),",
    ),
    (
        "environment_catalog_resolver_wrong_identity",
        "resolve_paths",
        "rust/catalog_decode.rs",
        "&& result->Ok_0 == identity(old(self)@, *name, encoding),",
        "&& true,",
    ),
    (
        "environment_catalog_iterator_loses_entry",
        "owned_entries",
        "verification/common/verus/collections.rs",
        "remaining(iter).to_set() == map@.kv_pairs()",
        "remaining(iter).to_set().subset_of(map@.kv_pairs())",
    ),
)
for _name, _function, *_ in CATALOG_DECODE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "catalog_decode"


COMMIT_EXECUTION_MUTATIONS = (
    ("commit_execution_skips_retain", "CommitEffect::Retain => effects.retain(),", "CommitEffect::Retain => Ok(()),"),
    (
        "commit_execution_skips_wal",
        "CommitEffect::AppendWal => effects.append(),",
        "CommitEffect::AppendWal => Ok(()),",
    ),
    ("commit_execution_skips_apply", "let applied = effects.apply();", "let applied: Result<(), E::Error> = Ok(());"),
    (
        "commit_execution_skips_consume",
        "CommitEffect::ConsumeStaging => effects.consume(),",
        "CommitEffect::ConsumeStaging => Ok(()),",
    ),
    ("commit_execution_unarmed_apply", "                effects.arm_apply();\n", ""),
    ("commit_execution_confirms_error", "if applied.is_ok() {", "if applied.is_err() {"),
    ("commit_execution_swallows_error", "        outcome?;", "        let _ = outcome;"),
    (
        "commit_execution_false_completion",
        "None => return Ok(commit_finished(&protocol)),",
        "None => return Ok(false),",
    ),
    (
        "commit_execution_wrong_dispatch",
        "CommitEffect::Retain => effects.retain(),",
        "CommitEffect::Retain => effects.append(),",
    ),
    ("environment_commit_retain_lies", "result.is_ok() ==> final(self)@.retained,", "true,"),
    (
        "environment_commit_acceptance_lies",
        "result.is_ok() ==> final(self)@.accepted && !final(self)@.poisoned,",
        "result.is_ok() ==> !final(self)@.poisoned,",
    ),
    (
        "environment_commit_uncertainty_reusable",
        "result.is_err() && final(self)@.accepted ==> final(self)@.poisoned,",
        "true,",
    ),
    (
        "environment_commit_apply_clears_guard",
        "final(self)@ == (CommitView { applied: final(self)@.applied, ..old(self)@ }),",
        "final(self)@ == (CommitView { applied: final(self)@.applied, poisoned: final(self)@.poisoned, ..old(self)@ }),",
    ),
    ("environment_commit_cleanup_lies", "result.is_ok() ==> final(self)@.consumed,", "true,"),
)
for _name, *_ in COMMIT_EXECUTION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = "execute_commit"
    NEGATIVE_MODULES[_name] = "protocol"


CATALOG_RECOVERY_MUTATIONS = (
    (
        "catalog_recovery_skips_begin",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "        let rebuilt = self\n            .log\n            .begin_replay(size)\n            .map_err(|error| crate::wal_recovery::Failure::Apply(Error::State(error)))?;",
        "        let rebuilt = false;",
    ),
    (
        "catalog_recovery_skips_confirmation",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "        let confirmed = self.log.confirm_replay(boundary.end, boundary.sequence);\n        #[cfg(straw_verus)]\n        proof! { assert(confirmed.is_ok()); }\n        confirmed.map_err(|error| crate::wal_recovery::Failure::Apply(Error::State(error)))?;",
        "",
    ),
    (
        "catalog_recovery_ignores_floor",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "let durable = self.log.floor();",
        "let durable = 0;",
    ),
    (
        "catalog_recovery_wrong_cursor",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "let offset = self.log.offset();",
        "let offset = 0;",
    ),
    (
        "catalog_recovery_wrong_sequence",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "let sequence = self.log.sequence();",
        "let sequence = 0;",
    ),
    (
        "catalog_recovery_wrong_kind",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "            crate::framing::Kind::Catalog,\n",
        "            crate::framing::Kind::Queue,\n",
    ),
    (
        "catalog_recovery_without_barrier",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "            durable,\n            true,",
        "            durable,\n            false,",
    ),
    (
        "catalog_recovery_confirms_failed_barrier",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        "        let boundary = crate::wal_recovery::recover(\n            device,\n            self,\n            crate::framing::Kind::Catalog,\n            offset,\n            sequence,\n            durable,\n            true,\n        )?;",
        "        let boundary = match crate::wal_recovery::recover(\n            device,\n            self,\n            crate::framing::Kind::Catalog,\n            offset,\n            sequence,\n            durable,\n            true,\n        ) {\n            Ok(boundary) => boundary,\n            Err(error) => {\n                let _ = self.log.confirm_replay(self.log.offset(), self.log.sequence());\n                return Err(error);\n            }\n        };",
    ),
    (
        "catalog_recovery_wrong_confirmation",
        "catalog_target",
        "recover",
        "rust/catalog_target.rs",
        ".confirm_replay(boundary.end, boundary.sequence)",
        ".confirm_replay(offset, sequence)",
    ),
    (
        "environment_replay_target_partial_success",
        "wal_recovery",
        "recover",
        "rust/wal_device.rs",
        "result.is_ok() ==> final(self).complete(),",
        "true,",
    ),
    (
        "environment_replay_target_changes_context",
        "wal_recovery",
        "recover",
        "rust/wal_device.rs",
        "ensures final(self).valid(), final(self).context() == old(self).context(),",
        "ensures final(self).valid(),",
    ),
    (
        "environment_recovery_omits_complete",
        "catalog_target",
        "recover",
        "rust/wal_recovery.rs",
        "result.is_ok() ==> final(target).complete(),",
        "true,",
    ),
    (
        "environment_recovery_omits_context",
        "catalog_target",
        "recover",
        "rust/wal_recovery.rs",
        "ensures final(device).loan() == old(device).loan(), final(device).valid(), final(target).valid(), final(target).context() == old(target).context(),",
        "ensures final(device).loan() == old(device).loan(), final(device).valid(), final(target).valid(),",
    ),
    (
        "environment_recovery_omits_durable",
        "catalog_target",
        "recover",
        "rust/wal_recovery.rs",
        "result.is_ok() && writable ==> final(device)@.durable == result->Ok_0.end,",
        "true,",
    ),
)
for _name, _module, _function, *_ in CATALOG_RECOVERY_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


GC_BATCH_MUTATIONS = (
    (
        "gc_batch_unmarked_unlink",
        "if !matches!(protocol::delete_next(&deletion), Some(DeleteEffect::Unlink)) {",
        "if false {",
    ),
    ("gc_batch_wrong_unlink_target", "effects.unlink(index)", "effects.unlink(0)"),
    ("gc_batch_skip_unlinks", "while index < count {", "while false {"),
    ("gc_batch_omit_parent", "        parents.insert(removal.parent);", ""),
    (
        "gc_batch_skip_absent_parent",
        "        parents.insert(removal.parent);",
        "        if removal.bytes.is_some() { parents.insert(removal.parent); }",
    ),
    ("gc_batch_skip_sync", "effects.sync_parent(parent)", "if false { effects.sync_parent(parent) } else { Ok(()) }"),
    (
        "gc_batch_incomplete_barriers",
        "let Some(parent) = remaining.next() else {",
        "let Some(parent) = None::<&E::Parent> else {",
    ),
    (
        "gc_batch_receipt_before_sync",
        "            ReclaimEffect::SyncParents => {",
        "            ReclaimEffect::SyncParents => {\n                if let Err(error) = effects.receipt() { return Err(Failure::Effect(error)); }",
    ),
    (
        "gc_batch_unlink_error_success",
        "            Err(error) => return Err(Failure::Effect(error)),\n        };",
        "            Err(_) => return Ok(Statistics { files, bytes }),\n        };",
    ),
    (
        "gc_batch_ignore_sync_error",
        "                    if let Err(error) = effects.sync_parent(parent) {\n                        return Err(Failure::Effect(error));\n                    }",
        "                    let _ = effects.sync_parent(parent);",
    ),
    (
        "gc_batch_skip_receipt",
        "                if let Err(error) = effects.receipt() {\n                    return Err(Failure::Effect(error));\n                }",
        "",
    ),
    (
        "gc_batch_ignore_receipt_error",
        "                if let Err(error) = effects.receipt() {\n                    return Err(Failure::Effect(error));\n                }",
        "                let _ = effects.receipt();",
    ),
    ("gc_batch_spurious_protocol_failure", "if !protocol::delete_batch_finished(&pending, true) {", "if true {"),
    (
        "gc_batch_unchecked_bytes",
        "bytes = match crate::bounds::add(bytes, size) {",
        "bytes = match Some(bytes + size) {",
    ),
    ("environment_gc_batch_unlink_lies", "&& final(self)@.absent.contains(index as nat)", "&& true"),
    ("environment_gc_batch_wrong_parent", "&& result->Ok_0.parent == old(self)@.parents[index as int],", "&& true,"),
    (
        "environment_gc_batch_sync_lies",
        "&& under(old(self)@, Set::empty().insert(*parent)).subset_of(final(self)@.durable_absent),",
        "&& true,",
    ),
    (
        "environment_gc_batch_receipt_lies",
        "result.is_ok() ==> final(self).confirmed() && final(self)@.receipted == indices(final(self)@),",
        "result.is_ok() ==> final(self).confirmed(),",
    ),
    (
        "environment_gc_batch_receipt_loses_durability",
        "final(self)@.durable_absent == old(self)@.durable_absent,",
        "true,",
    ),
)
for _name, *_ in GC_BATCH_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = "collect"
    NEGATIVE_MODULES[_name] = "gc_batch"


QUEUE_APPLICATION_MUTATIONS = (
    (
        "application_arm_clears_quarantine",
        "queue_application",
        "arm",
        "rust/queue_application.rs",
        "pub(crate) fn arm(journal: &mut State) {\n    journal.poisoned = true;",
        "pub(crate) fn arm(journal: &mut State) {\n    journal.poisoned = false;",
    ),
    (
        "application_before_error_ignored",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "    if let Err(error) = hooks.before() {\n        return Err(Error::Before(error));\n    }",
        "    let _ = hooks.before();",
    ),
    (
        "application_after_error_ignored",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "    if let Err(error) = hooks.after() {\n        return Err(Error::After(error));\n    }",
        "    let _ = hooks.after();",
    ),
    (
        "application_after_called_first",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "if let Err(error) = hooks.before() {",
        "if let Err(error) = hooks.after() {",
    ),
    (
        "application_before_called_last",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "if let Err(error) = hooks.after() {",
        "if let Err(error) = hooks.before() {",
    ),
    (
        "application_wrong_business_input",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "queue_history::transaction(state, epoch, order, sealed, events, caches);",
        "queue_history::transaction(state, epoch, order, sealed, &Value::Null, caches);",
    ),
    (
        "application_body_error_ignored",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "    if let Err(error) = result {\n        return Err(Error::Body(error));\n    }",
        "    let _ = result;",
    ),
    (
        "application_body_failure_wrong_index",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "return Err(Error::Body(error));",
        "return Err(Error::Body(queue_history::Failure { index: 0, error: error.error }));",
    ),
    (
        "application_confirm_keeps_quarantine",
        "queue_application",
        "confirm",
        "rust/queue_application.rs",
        "let Confirmation { private: (), .. } = token;\n    journal.poisoned = false;",
        "let Confirmation { private: (), .. } = token;\n    journal.poisoned = true;",
    ),
    (
        "application_gate_ignores_poison",
        "queue_application",
        "permitted",
        "rust/queue_application.rs",
        'method == "state" || (!journal.poisoned && !closed)',
        'method == "state" || !closed',
    ),
    (
        "application_gate_ignores_closed",
        "queue_application",
        "permitted",
        "rust/queue_application.rs",
        'method == "state" || (!journal.poisoned && !closed)',
        'method == "state" || !journal.poisoned',
    ),
    (
        "application_restore_reuses_poisoned",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "    if journal.poisoned {\n        return Err(RestoreError::Unavailable);\n    }",
        "    if false {\n        return Err(RestoreError::Unavailable);\n    }",
    ),
    (
        "application_restore_omits_guard",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "    arm(journal);\n    let mut result = Restored {",
        "    let mut result = Restored {",
    ),
    (
        "application_restore_wrong_initial_order",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "        order: 0,\n        sealed: false,",
        "        order: 1,\n        sealed: false,",
    ),
    (
        "application_restore_wrong_initial_state",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "        state: queue_history::initial(),",
        "        state: Value::Null,",
    ),
    (
        "application_restore_clears_on_error",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "    if let Err(error) = outcome {\n        return Err(RestoreError::History(error));",
        "    if let Err(error) = outcome {\n        journal.poisoned = false;\n        return Err(RestoreError::History(error));",
    ),
    (
        "application_restore_ignores_partial_failure",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "    if let Err(error) = outcome {\n        return Err(RestoreError::History(error));\n    }",
        "    let _ = outcome;",
    ),
    (
        "application_restore_changes_sequence",
        "queue_application",
        "restore",
        "rust/queue_application.rs",
        "    journal.poisoned = false;\n    Ok(result)",
        "    journal.poisoned = false;\n    journal.sequence = 0;\n    Ok(result)",
    ),
    (
        "application_hooks_omit_before_observation",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "ensures final(self)@ == old(self)@.push((true, result.is_ok())),",
        "ensures final(self)@ == old(self)@,",
    ),
    (
        "application_hooks_invert_after_result",
        "queue_application",
        "apply",
        "rust/queue_application.rs",
        "ensures final(self)@ == old(self)@.push((false, result.is_ok())),",
        "ensures final(self)@ == old(self)@.push((false, !result.is_ok())),",
    ),
    (
        "application_restored_prefix_includes_header",
        "journal_equivalence",
        "restored_prefix",
        "verification/wal/verus/journal_equivalence.rs",
        "&& crate::queue_history::folded(before, value.business(),\n                    crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))),",
        "&& crate::queue_history::folded(before, value.business(),\n                    crate::queue_history::flatten(journal.transactions@)),",
    ),
)
for _name, _module, _function, *_ in QUEUE_APPLICATION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_HISTORY_MUTATIONS = (
    (
        "history_initial_wrong_table",
        "queue_history",
        "initial",
        "rust/queue_history.rs",
        'fields.insert("readers".to_owned(), Value::Object(Map::new()));',
        'fields.insert("reader".to_owned(), Value::Object(Map::new()));',
    ),
    (
        "history_initial_wrong_prefix",
        "queue_history",
        "initial",
        "rust/queue_history.rs",
        'fields.insert("retired_prefix".to_owned(), Value::Number(0u64.into()));',
        'fields.insert("retired_prefix".to_owned(), Value::Number(1u64.into()));',
    ),
    (
        "history_lookup_wrong_root",
        "queue_history",
        "lookup",
        "rust/queue_history.rs",
        'let tasks = "tasks".index_into(state)',
        'let tasks = "readers".index_into(state)',
    ),
    (
        "history_admission_inverted",
        "queue_history",
        "prepare",
        "rust/queue_history.rs",
        "if !queue_task::valid_target(prepared.kind, selected) {",
        "if queue_task::valid_target(prepared.kind, selected) {",
    ),
    (
        "history_preparation_wrong_state",
        "queue_history",
        "apply",
        "rust/queue_history.rs",
        "let prepared = prepare(state, event, caches)?;",
        "let prepared = prepare(&Value::Null, event, caches)?;",
    ),
    (
        "history_body_wrong_kind",
        "queue_history",
        "apply",
        "rust/queue_history.rs",
        "queue_replay::body(state, epoch, order, sealed, event, prepared.kind, caches);",
        'queue_replay::body(state, epoch, order, sealed, event, "Sealed", caches);',
    ),
    (
        "history_body_error_hidden",
        "queue_history",
        "apply",
        "rust/queue_history.rs",
        "Err(error) => return Err(Error::Body(error)),",
        "Err(error) => return Ok(()),",
    ),
    (
        "history_receipt_forced_recorded",
        "queue_history",
        "apply",
        "rust/queue_history.rs",
        "let outcome = queue_replay::finish(state, event, recorded);",
        "let outcome = queue_replay::finish(state, event, true);",
    ),
    (
        "history_receipt_error_hidden",
        "queue_history",
        "apply",
        "rust/queue_history.rs",
        "Err(error) => Err(Error::Receipt(error)),",
        "Err(error) => Ok(()),",
    ),
    (
        "history_transaction_repeats_first",
        "queue_history",
        "transaction",
        "rust/queue_history.rs",
        "let outcome = apply(state, epoch, order, sealed, &events[index], caches);",
        "let outcome = apply(state, epoch, order, sealed, &events[0], caches);",
    ),
    (
        "history_transaction_reverses",
        "queue_history",
        "transaction",
        "rust/queue_history.rs",
        "let outcome = apply(state, epoch, order, sealed, &events[index], caches);",
        "let outcome = apply(state, epoch, order, sealed, &events[events.len() - index - 1], caches);",
    ),
    (
        "history_transaction_skips_last",
        "queue_history",
        "transaction",
        "rust/queue_history.rs",
        "while index < events.len() {",
        "while index + 1 < events.len() {",
    ),
    (
        "history_transaction_wrong_failure_index",
        "queue_history",
        "transaction",
        "rust/queue_history.rs",
        "let failure = Failure { index, error };",
        "let failure = Failure { index: 0, error };",
    ),
    (
        "history_recovery_applies_header",
        "queue_history",
        "recover",
        "rust/queue_history.rs",
        "let first = if frames.is_empty() { 0 } else { 1 };",
        "let first = 0;",
    ),
    (
        "history_recovery_repeats_first",
        "queue_history",
        "recover",
        "rust/queue_history.rs",
        "let outcome = transaction(state, epoch, order, sealed, &frames[index], caches);",
        "let outcome = transaction(state, epoch, order, sealed, &frames[first], caches);",
    ),
    (
        "history_recovery_skips_last",
        "queue_history",
        "recover",
        "rust/queue_history.rs",
        "while index < frames.len() {",
        "while index + 1 < frames.len() {",
    ),
    (
        "history_recovery_wrong_application_index",
        "queue_history",
        "recover",
        "rust/queue_history.rs",
        "index,\n                cause: ReplayCause::Application(error),",
        "index: first,\n                cause: ReplayCause::Application(error),",
    ),
    (
        "history_recovery_wrong_bookkeeping_index",
        "queue_history",
        "recover",
        "rust/queue_history.rs",
        "index,\n                cause: ReplayCause::Bookkeeping(error),",
        "index: 0,\n                cause: ReplayCause::Bookkeeping(error),",
    ),
    (
        "history_journal_wrong_source",
        "journal_state",
        "apply_business",
        "rust/journal_state.rs",
        "            &self.transactions,\n            caches,",
        "            &[],\n            caches,",
    ),
    (
        "history_fold_reverses_frames",
        "queue_history",
        "frames_fold",
        "rust/queue_history.rs",
        "else { flatten(frames.drop_last()) + event_list(frames.last()) }",
        "else { event_list(frames.last()) + flatten(frames.drop_last()) }",
    ),
    (
        "history_wal_includes_header",
        "journal_equivalence",
        "physical_prefix_business",
        "verification/wal/verus/journal_equivalence.rs",
        "&& crate::queue_history::folded(before, after,\n                    crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))),",
        "&& crate::queue_history::folded(before, after,\n                    crate::queue_history::flatten(journal.transactions@)),",
    ),
)
for _name, _module, _function, *_ in QUEUE_HISTORY_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_REPLAY_MUTATIONS = (
    (
        "retire_reversed_worker",
        "queue_reader",
        "keep_worker",
        "rust/queue_reader.rs",
        "    !matches_worker(entry, worker)",
        "    matches_worker(entry, worker)",
    ),
    (
        "retire_mutates_kept_entry",
        "queue_reader",
        "keep_worker",
        "rust/queue_reader.rs",
        "    !matches_worker(entry, worker)",
        "    *entry = Value::Null;\n    !matches_worker(entry, worker)",
    ),
    (
        "retire_skips_filter",
        "queue_reader",
        "retire",
        "rust/queue_reader.rs",
        "    table.retain(predicate);",
        "    let _ = predicate;",
    ),
    (
        "retire_wrong_worker_field",
        "queue_admin",
        "reader_retired",
        "rust/queue_admin.rs",
        'let worker = "worker_id".index_into(event)',
        'let worker = "attempt_id".index_into(event)',
    ),
    (
        "retire_wrong_root",
        "queue_admin",
        "reader_retired",
        "rust/queue_admin.rs",
        '    let Some(readers) = "readers".index_into_mut(state) else {\n        return Err(Error::State);\n    };\n    match queue_reader::retire(readers, worker)',
        '    let Some(readers) = "producers".index_into_mut(state) else {\n        return Err(Error::State);\n    };\n    match queue_reader::retire(readers, worker)',
    ),
    (
        "retire_library_allows_new_keys",
        "queue_reader",
        "retire",
        "verification/common/verus/serde_json.rs",
        "ensures object(*final(map)).dom().subset_of(object(*old(map)).dom()),",
        "ensures true,",
    ),
    (
        "retire_library_omits_missing_entries",
        "queue_reader",
        "retire",
        "verification/common/verus/serde_json.rs",
        "forall|key: String| object(*old(map)).contains_key(key) ==> exists|value: &mut Value, keep: bool|",
        "forall|key: String| object(*old(map)).contains_key(key) && object(*final(map)).contains_key(key) ==> exists|value: &mut Value, keep: bool|",
    ),
    (
        "retire_library_loses_entry_binding",
        "queue_reader",
        "retire",
        "verification/common/verus/serde_json.rs",
        "(keep ==> object(*final(map))[key] == *final(value));",
        "true;",
    ),
    (
        "retire_library_inverts_decision",
        "queue_reader",
        "retire",
        "verification/common/verus/serde_json.rs",
        "&& keep == object(*final(map)).contains_key(key)",
        "&& keep != object(*final(map)).contains_key(key)",
    ),
    (
        "replay_item_wrong_lease",
        "queue_replay",
        "apply_item",
        "rust/queue_replay.rs",
        "transition::leased(state, item)",
        "transition::leased(state, event)",
    ),
    (
        "replay_item_wrong_failure",
        "queue_replay",
        "apply_item",
        "rust/queue_replay.rs",
        "transition::failed(state, id, event)",
        "transition::failed(state, id, item)",
    ),
    (
        "replay_batch_repeats_first",
        "queue_replay",
        "many",
        "rust/queue_replay.rs",
        "apply_item(state, &items[index], event, leased, deadlines)",
        "apply_item(state, &items[0], event, leased, deadlines)",
    ),
    (
        "replay_batch_reverses",
        "queue_replay",
        "many",
        "rust/queue_replay.rs",
        "apply_item(state, &items[index], event, leased, deadlines)",
        "apply_item(state, &items[items.len() - 1 - index], event, leased, deadlines)",
    ),
    (
        "replay_batch_omits_last",
        "queue_replay",
        "many",
        "rust/queue_replay.rs",
        "while index < items.len() {",
        "while index + 1 < items.len() {",
    ),
    (
        "replay_batch_swallows_error",
        "queue_replay",
        "many",
        "rust/queue_replay.rs",
        "            return Err(error);\n        }\n        #[cfg(straw_verus)]\n        proof! {\n            push",
        "            return Ok(());\n        }\n        #[cfg(straw_verus)]\n        proof! {\n            push",
    ),
    (
        "replay_batch_wrong_field",
        "queue_replay",
        "batch",
        "rust/queue_replay.rs",
        'let name = if leased { "leases" } else { "task_ids" };',
        'let name = if leased { "task_ids" } else { "leases" };',
    ),
    (
        "replay_dispatch_wrong_leased",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "batch(state, event, true, deadlines)?;",
        "batch(state, event, false, deadlines)?;",
    ),
    (
        "replay_dispatch_wrong_expired",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "batch(state, event, false, deadlines)?;",
        "batch(state, event, true, deadlines)?;",
    ),
    (
        "replay_dispatch_wrong_yield",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        'let yielding = kind == "Yielded";',
        'let yielding = kind == "TaskProgress";',
    ),
    (
        "replay_dispatch_wrong_receipt",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "transition::complete(state, receipt, lease)",
        "transition::complete(state, lease, receipt)",
    ),
    (
        "replay_dispatch_omits_seal",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "        *sealed = true;",
        "        *sealed = *sealed;",
    ),
    (
        "replay_dispatch_wrong_receipt_flag",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "            Ok(()) => Ok(true),\n            Err(e) => Err(Error::Metadata(e)),",
        "            Ok(()) => Ok(false),\n            Err(e) => Err(Error::Metadata(e)),",
    ),
    (
        "replay_dispatch_unknown_accepted",
        "queue_replay",
        "body",
        "rust/queue_replay.rs",
        "        return Err(Error::Unknown);",
        "        return Ok(false);",
    ),
    (
        "replay_finish_skips_receipt",
        "queue_replay",
        "finish",
        "rust/queue_replay.rs",
        "    if recorded {\n        Ok(())\n    } else {\n        admin::remember(state, event)\n    }",
        "    if recorded {\n        admin::remember(state, event)\n    } else {\n        Ok(())\n    }",
    ),
)
for _name, _module, _function, *_ in QUEUE_REPLAY_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_ADMIN_MUTATIONS = (
    (
        "admin_capacity_accepts_exhaustion",
        "queue_admin",
        "has_capacity",
        "rust/queue_admin.rs",
        "order.checked_add(count).is_some()",
        "true",
    ),
    (
        "admin_capacity_ignores_count",
        "queue_admin",
        "has_capacity",
        "rust/queue_admin.rs",
        "order.checked_add(count).is_some()",
        "order.checked_add(0).is_some()",
    ),
    (
        "admin_submit_wrong_increment",
        "queue_admin",
        "submit",
        "rust/queue_admin.rs",
        "let Some(next) = order.checked_add(1) else",
        "let Some(next) = order.checked_add(0) else",
    ),
    (
        "admin_submit_wrong_order",
        "queue_admin",
        "submit",
        "rust/queue_admin.rs",
        "queue_state::submit(state, spec, *order as u64)",
        "queue_state::submit(state, spec, 0)",
    ),
    (
        "admin_submit_increments_on_error",
        "queue_admin",
        "submit",
        "rust/queue_admin.rs",
        "    if let Err(error) = queue_state::submit(state, spec, *order as u64) {\n        return Err(Error::Task(error));\n    }\n    *order = next;",
        "    let previous = *order;\n    *order = next;\n    if let Err(error) = queue_state::submit(state, spec, previous as u64) {\n        return Err(Error::Task(error));\n    }",
    ),
    (
        "admin_submit_swallows_error",
        "queue_admin",
        "submit",
        "rust/queue_admin.rs",
        "    if let Err(error) = queue_state::submit(state, spec, *order as u64) {\n        return Err(Error::Task(error));\n    }",
        "    if let Err(error) = queue_state::submit(state, spec, *order as u64) {\n        return Ok(());\n    }",
    ),
    (
        "admin_batch_repeats_first",
        "queue_admin",
        "submit_many",
        "rust/queue_admin.rs",
        "submit(state, order, &specs[index])",
        "submit(state, order, &specs[0])",
    ),
    (
        "admin_batch_reverses",
        "queue_admin",
        "submit_many",
        "rust/queue_admin.rs",
        "submit(state, order, &specs[index])",
        "submit(state, order, &specs[specs.len() - 1 - index])",
    ),
    (
        "admin_batch_omits_last",
        "queue_admin",
        "submit_many",
        "rust/queue_admin.rs",
        "while index < specs.len() {",
        "while index < specs.len() && index < specs.len() - 1 {",
    ),
    (
        "admin_batch_skips_tasks",
        "queue_admin",
        "submit_many",
        "rust/queue_admin.rs",
        "submit(state, order, &specs[index])",
        "Ok::<(), Error>(())",
    ),
    (
        "admin_yield_does_not_allocate",
        "queue_admin",
        "progress",
        "rust/queue_admin.rs",
        "let next = if yielding {",
        "let next = if false {",
    ),
    (
        "admin_ordinary_progress_allocates",
        "queue_admin",
        "progress",
        "rust/queue_admin.rs",
        "let next = if yielding {",
        "let next = if true {",
    ),
    (
        "admin_progress_wrong_yielding",
        "queue_admin",
        "progress",
        "rust/queue_admin.rs",
        "queue_transition::progress(state, event, yielding, *order as u64)",
        "queue_transition::progress(state, event, !yielding, *order as u64)",
    ),
    (
        "admin_array_wrong_field",
        "queue_admin",
        "array",
        "rust/queue_admin.rs",
        "name.index_into(event)",
        '"tasks".index_into(event)',
    ),
    (
        "admin_producer_wrong_table",
        "queue_admin",
        "continue_producer",
        "rust/queue_admin.rs",
        '"producers".index_into_mut(state)',
        '"readers".index_into_mut(state)',
    ),
    (
        "admin_producer_wrong_event",
        "queue_admin",
        "continue_producer",
        "rust/queue_admin.rs",
        "queue_receipt::continue_producer(producers, event)",
        "queue_receipt::continue_producer(producers, &Value::Null)",
    ),
    (
        "admin_submitted_wrong_list",
        "queue_admin",
        "submitted_event",
        "rust/queue_admin.rs",
        'let specs = array(event, "tasks")?;',
        'let specs = array(event, "leases")?;',
    ),
    (
        "admin_submitted_skips_batch",
        "queue_admin",
        "submitted_event",
        "rust/queue_admin.rs",
        "submit_many(state, order, specs)?;",
        "// Intentionally omit the submission batch.",
    ),
    (
        "admin_producer_before_batch",
        "queue_admin",
        "submitted_event",
        "rust/queue_admin.rs",
        "submit_many(state, order, specs)?;",
        "continue_producer(state, event)?;\n    submit_many(state, order, specs)?;",
    ),
    (
        "admin_epoch_wrong_identity",
        "queue_admin",
        "apply_epoch",
        "rust/queue_admin.rs",
        'queue_state::identity(event, "epoch")',
        'queue_state::identity(event, "task_id")',
    ),
    (
        "admin_epoch_omits_assignment",
        "queue_admin",
        "apply_epoch",
        "rust/queue_admin.rs",
        "*epoch = id.clone();",
        "let _ = id;",
    ),
    (
        "admin_epoch_wrong_revocations",
        "queue_admin",
        "apply_epoch",
        "rust/queue_admin.rs",
        'let revoked = array(event, "revoked")?;',
        'let revoked = array(event, "task_ids")?;',
    ),
    (
        "admin_epoch_wrong_transition",
        "queue_admin",
        "apply_epoch",
        "rust/queue_admin.rs",
        "queue_state::edit_many(state, revoked, queue_state::Change::Retry)",
        "queue_state::edit_many(state, revoked, queue_state::Change::Terminal(crate::queue_task::Terminal::Failed))",
    ),
    (
        "admin_receipt_wrong_table",
        "queue_admin",
        "remember",
        "rust/queue_admin.rs",
        '"requests".index_into_mut(state)',
        '"producers".index_into_mut(state)',
    ),
    (
        "admin_receipt_wrong_event",
        "queue_admin",
        "remember",
        "rust/queue_admin.rs",
        "crate::request::remember(requests, event);",
        "crate::request::remember(requests, &Value::Null);",
    ),
    (
        "admin_reader_done_wrong_list",
        "queue_admin",
        "reader_done",
        "rust/queue_admin.rs",
        'let leases = array(event, "leases")?;',
        'let leases = array(event, "tasks")?;',
    ),
    (
        "admin_reader_done_wrong_table",
        "queue_admin",
        "reader_done",
        "rust/queue_admin.rs",
        '    let leases = array(event, "leases")?;\n    let Some(readers) = "readers".index_into_mut(state)',
        '    let leases = array(event, "leases")?;\n    let Some(readers) = "tasks".index_into_mut(state)',
    ),
    (
        "admin_reader_done_swallows_error",
        "queue_admin",
        "reader_done",
        "rust/queue_admin.rs",
        "    match queue_reader::done(readers, leases) {\n        Ok(()) => Ok(()),\n        Err(error) => Err(Error::Reader(error)),\n    }",
        "    match queue_reader::done(readers, leases) {\n        Ok(()) => Ok(()),\n        Err(error) => Ok(()),\n    }",
    ),
    (
        "admin_model_loses_counter_sum",
        "queue_admin",
        "capacity_rules_out_order_error",
        "rust/queue_admin.rs",
        "order as int + specs.len() == next as int && states.len() == specs.len() + 1",
        "order <= next && states.len() == specs.len() + 1",
    ),
)
for _name, _module, _function, *_ in QUEUE_ADMIN_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_TRANSITION_MUTATIONS = (
    (
        "transition_wrong_field",
        "queue_transition",
        "get",
        "rust/queue_transition.rs",
        "key.index_into(value).unwrap_or(&Value::Null)",
        '"lease".index_into(value).unwrap_or(&Value::Null)',
    ),
    (
        "transition_wrong_task_table",
        "queue_transition",
        "task",
        "rust/queue_transition.rs",
        'let Value::Object(table) = get(state, "tasks")',
        'let Value::Object(table) = get(state, "readers")',
    ),
    (
        "transition_releases_wrong_attempt",
        "queue_transition",
        "release_current",
        "rust/queue_transition.rs",
        'queue_reader::identity(get(task(state, id), "lease"))',
        'queue_reader::identity(get(task(state, id), "spec"))',
    ),
    (
        "transition_release_wrong_table",
        "queue_transition",
        "release_current",
        "rust/queue_transition.rs",
        'let attempt = attempt.clone();\n    let Some(readers) = "readers".index_into_mut(state)',
        'let attempt = attempt.clone();\n    let Some(readers) = "tasks".index_into_mut(state)',
    ),
    (
        "transition_skips_release",
        "queue_transition",
        "finish",
        "rust/queue_transition.rs",
        "    if release {\n        release_current(state, id)?;",
        "    if false {\n        release_current(state, id)?;",
    ),
    (
        "transition_ignores_release_error",
        "queue_transition",
        "finish",
        "rust/queue_transition.rs",
        "        release_current(state, id)?;",
        "        let _ = release_current(state, id);",
    ),
    (
        "transition_complete_wrong_status",
        "queue_transition",
        "complete",
        "rust/queue_transition.rs",
        "queue_state::Change::Terminal(queue_task::Terminal::Completed),\n        true,",
        "queue_state::Change::Terminal(queue_task::Terminal::Cancelled),\n        true,",
    ),
    (
        "transition_complete_keeps_reader",
        "queue_transition",
        "complete",
        "rust/queue_transition.rs",
        "queue_state::Change::Terminal(queue_task::Terminal::Completed),\n        true,",
        "queue_state::Change::Terminal(queue_task::Terminal::Completed),\n        false,",
    ),
    (
        "transition_complete_wrong_receipt",
        "queue_transition",
        "complete",
        "rust/queue_transition.rs",
        "queue_receipt::record(state, receipt, event_lease)",
        "queue_receipt::record(state, event_lease, receipt)",
    ),
    (
        "transition_complete_skips_receipt",
        "queue_transition",
        "complete",
        "rust/queue_transition.rs",
        "queue_receipt::record(state, receipt, event_lease)",
        "Ok::<(), queue_receipt::Error>(())",
    ),
    (
        "transition_register_wrong_input",
        "queue_transition",
        "register_current",
        "rust/queue_transition.rs",
        'get(get(task(state, id), "spec"), "input_ref")',
        'get(get(task(state, id), "spec"), "metadata")',
    ),
    (
        "transition_register_wrong_lease",
        "queue_transition",
        "register_current",
        "rust/queue_transition.rs",
        "queue_reader::register(readers, event_lease, &input)",
        "queue_reader::register(readers, &input, event_lease)",
    ),
    (
        "transition_lease_wrong_change",
        "queue_transition",
        "leased",
        "rust/queue_transition.rs",
        "queue_state::edit(state, id, queue_state::Change::Lease(event_lease))",
        "queue_state::edit(state, id, queue_state::Change::Retry)",
    ),
    (
        "transition_lease_ignores_task_error",
        "queue_transition",
        "leased",
        "rust/queue_transition.rs",
        "    if let Err(error) = queue_state::edit(state, id, queue_state::Change::Lease(event_lease)) {\n        return Err(Error::Task(error));\n    }",
        "    let _ = queue_state::edit(state, id, queue_state::Change::Lease(event_lease));",
    ),
    (
        "transition_expiration_releases_reader",
        "queue_transition",
        "failed",
        "rust/queue_transition.rs",
        'let release = get(event, "type").as_str() == Some("Failed");',
        'let release = get(event, "type").as_str() == Some("Expired");',
    ),
    (
        "transition_cancellation_releases_reader",
        "queue_transition",
        "cancelled",
        "rust/queue_transition.rs",
        "queue_state::Change::Terminal(queue_task::Terminal::Cancelled),\n        false,",
        "queue_state::Change::Terminal(queue_task::Terminal::Cancelled),\n        true,",
    ),
    (
        "transition_progress_wrong_snapshot",
        "queue_transition",
        "progress_current",
        "rust/queue_transition.rs",
        'let snapshot = get(task(state, id), "lease").clone();',
        'let snapshot = get(task(state, id), "spec").clone();',
    ),
    (
        "transition_progress_wrong_input",
        "queue_transition",
        "progress",
        "rust/queue_transition.rs",
        'progress_current(state, id, get(event, "input_ref"))?;',
        'progress_current(state, id, get(event, "metadata"))?;',
    ),
    (
        "transition_progress_skips_roots",
        "queue_transition",
        "progress",
        "rust/queue_transition.rs",
        'progress_current(state, id, get(event, "input_ref"))?;',
        "// Intentionally omit root registration.",
    ),
    (
        "transition_progress_yield_inverted",
        "queue_transition",
        "progress",
        "rust/queue_transition.rs",
        "    if yielding {\n        finish(state, id, change, true)?;",
        "    if !yielding {\n        finish(state, id, change, true)?;",
    ),
    (
        "transition_progress_wrong_order",
        "queue_transition",
        "progress",
        "rust/queue_transition.rs",
        "    let change = queue_state::Change::Progress {\n        event,\n        yielding,\n        order,\n    };",
        "    let change = queue_state::Change::Progress {\n        event,\n        yielding,\n        order: 0,\n    };",
    ),
    (
        "transition_model_drops_reader_frame",
        "queue_transition",
        "no_release_preserves_readers",
        "rust/queue_state.rs",
        'queue_task::unchanged(before, after, set!["tasks"@]) && tasks(before) is Object',
        "tasks(before) is Object",
    ),
)
QUEUE_TRANSITION_MUTATIONS += (
    (
        "transition_roots_without_string_identity",
        "queue_transition_roots",
        "copied_attempt",
        "verification/protocol/verus/queue_transition_roots.rs",
        "requires queue_metadata::same_value(left, right), crate::environment::string_content_contract(),",
        "requires queue_metadata::same_value(left, right),",
    ),
    (
        "transition_roots_without_snapshot_binding",
        "queue_transition_roots",
        "current_preserves_roots",
        "rust/queue_transition.rs",
        "        ensures crate::queue_metadata::same_value(snapshot, lease(before, id))\n            && reader_progress_result(before, after, snapshot, new_input, result),",
        "        ensures reader_progress_result(before, after, snapshot, new_input, result),",
    ),
    (
        "transition_roots_loses_peer_readers",
        "queue_transition_roots",
        "current_preserves_roots",
        "rust/queue_transition.rs",
        "&& queue_reader::table(readers(after)) == queue_reader::table(readers(before)).insert(id, queue_reader::table(readers(after))[id])",
        "&& true",
    ),
    (
        "transition_roots_task_clobbers_readers",
        "queue_transition_roots",
        "progress_preserves_roots",
        "rust/queue_state.rs",
        'queue_task::unchanged(before, after, set!["tasks"@]) && tasks(before) is Object',
        "tasks(before) is Object",
    ),
    (
        "transition_roots_wrong_input_source",
        "queue_transition_roots",
        "progress_preserves_roots",
        "rust/queue_transition.rs",
        '        ensures current_progress(before, middle, id, field(event, "input_ref"@), Ok(()))\n            && queue_state::edit_result',
        '        ensures current_progress(before, middle, id, field(event, "metadata"@), Ok(()))\n            && queue_state::edit_result',
    ),
)
for _name, _module, _function, *_ in QUEUE_TRANSITION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_STATE_MUTATIONS = (
    (
        "queue_state_wrong_retry",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "Change::Retry => queue_task::retry(task),",
        "Change::Retry => queue_task::terminal(task, Terminal::Failed),",
    ),
    (
        "queue_state_wrong_lease",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "Change::Lease(lease) => queue_task::leased(task, lease),",
        "Change::Lease(lease) => queue_task::terminal(task, Terminal::Completed),",
    ),
    (
        "queue_state_failure_loses_evidence",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "Change::Failed(event) => queue_task::failed(task, event),",
        "Change::Failed(event) => queue_task::retry(task),",
    ),
    (
        "queue_state_wrong_terminal",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "Change::Terminal(status) => queue_task::terminal(task, status),",
        "Change::Terminal(status) => queue_task::terminal(task, Terminal::Cancelled),",
    ),
    (
        "queue_state_wrong_yielding",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "queue_task::progress(task, event, yielding, order)",
        "queue_task::progress(task, event, !yielding, order)",
    ),
    (
        "queue_state_wrong_order",
        "queue_state",
        "dispatch",
        "rust/queue_state.rs",
        "queue_task::progress(task, event, yielding, order)",
        "queue_task::progress(task, event, yielding, 0)",
    ),
    (
        "queue_state_edit_wrong_table",
        "queue_state",
        "edit",
        "rust/queue_state.rs",
        '    let Some(value) = "tasks".index_into_mut(state) else {\n        return Err(Error::Target);\n    };\n    let Value::Object(table) = value else {\n        return Err(Error::Target);\n    };\n    let Some(task) = table.get_mut(id) else',
        '    let Some(value) = "readers".index_into_mut(state) else {\n        return Err(Error::Target);\n    };\n    let Value::Object(table) = value else {\n        return Err(Error::Target);\n    };\n    let Some(task) = table.get_mut(id) else',
    ),
    (
        "queue_state_edit_wrong_task",
        "queue_state",
        "edit",
        "rust/queue_state.rs",
        "table.get_mut(id)",
        'table.get_mut(&"wrong".to_owned())',
    ),
    (
        "queue_state_accepts_missing_task",
        "queue_state",
        "edit",
        "rust/queue_state.rs",
        "let Some(task) = table.get_mut(id) else {\n        return Err(Error::Target);",
        "let Some(task) = table.get_mut(id) else {\n        return Ok(());",
    ),
    (
        "queue_state_swallows_task_error",
        "queue_state",
        "edit",
        "rust/queue_state.rs",
        "Err(error) => Err(Error::Task(error)),",
        "Err(error) => Ok(()),",
    ),
    (
        "queue_state_submit_wrong_table",
        "queue_state",
        "submit",
        "rust/queue_state.rs",
        'let id = identity(spec, "task_id")?;\n    let Some(value) = "tasks".index_into_mut(state)',
        'let id = identity(spec, "task_id")?;\n    let Some(value) = "readers".index_into_mut(state)',
    ),
    (
        "queue_state_submit_wrong_id",
        "queue_state",
        "submit",
        "rust/queue_state.rs",
        "table.insert(id.clone(), queue_task::submitted(spec, order));",
        'table.insert("wrong".to_owned(), queue_task::submitted(spec, order));',
    ),
    (
        "queue_state_submit_wrong_order",
        "queue_state",
        "submit",
        "rust/queue_state.rs",
        "queue_task::submitted(spec, order)",
        "queue_task::submitted(spec, 0)",
    ),
    (
        "queue_state_rejects_valid",
        "queue_state",
        "edit",
        "rust/queue_state.rs",
        "pub fn edit(state: &mut Value, id: &String, change: Change<'_>) -> Result<(), Error> {",
        "pub fn edit(state: &mut Value, id: &String, change: Change<'_>) -> Result<(), Error> {\n    return Err(Error::Target);",
    ),
    (
        "queue_state_many_repeats_first",
        "queue_state",
        "edit_many",
        "rust/queue_state.rs",
        "let Value::String(id) = &ids[index] else",
        "let Value::String(id) = &ids[0] else",
    ),
    (
        "queue_state_many_reverses_order",
        "queue_state",
        "edit_many",
        "rust/queue_state.rs",
        "let Value::String(id) = &ids[index] else",
        "let Value::String(id) = &ids[ids.len() - 1 - index] else",
    ),
    (
        "queue_state_many_omits_last",
        "queue_state",
        "edit_many",
        "rust/queue_state.rs",
        "while index < ids.len() {",
        "while index < ids.len() && index < ids.len() - 1 {",
    ),
    (
        "queue_state_many_hides_failure",
        "queue_state",
        "edit_many",
        "rust/queue_state.rs",
        "            return Err(error);\n        }",
        "            return Ok(());\n        }",
    ),
    (
        "queue_state_identity_wrong_field",
        "queue_state",
        "identity",
        "rust/queue_state.rs",
        "match name.index_into(value)",
        'match "task_id".index_into(value)',
    ),
    (
        "queue_state_model_loses_other_roots",
        "queue_state",
        "trace_frame",
        "rust/queue_state.rs",
        'queue_task::unchanged(before, after, set!["tasks"@]) && tasks(before) is Object',
        "tasks(before) is Object",
    ),
    (
        "queue_state_model_loses_peer_tasks",
        "queue_state",
        "trace_frame",
        "rust/queue_state.rs",
        "&& table(after).contains_key(id) && table(after) == table(before).insert(id, table(after)[id])",
        "&& table(after).contains_key(id)",
    ),
)
for _name, _module, _function, *_ in QUEUE_STATE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_READER_MUTATIONS = (
    (
        "reader_contains_skips_last",
        "queue_reader",
        "contains",
        "rust/queue_reader.rs",
        "while index < items.len() {",
        "while index < items.len() && index < items.len() - 1 {",
    ),
    (
        "reader_contains_checks_first",
        "queue_reader",
        "contains",
        "rust/queue_reader.rs",
        "if items[index] == *input {",
        "if items[0] == *input {",
    ),
    (
        "reader_wrong_lease",
        "queue_reader",
        "create",
        "rust/queue_reader.rs",
        'entry.insert("lease".to_owned(), lease.clone());',
        'entry.insert("lease".to_owned(), input.clone());',
    ),
    (
        "reader_missing_initial_root",
        "queue_reader",
        "create",
        "rust/queue_reader.rs",
        "Value::Array(vec![input.clone()])",
        "Value::Array(Vec::new())",
    ),
    (
        "reader_loses_previous_roots",
        "queue_reader",
        "extend",
        "rust/queue_reader.rs",
        "        roots.push(input.clone());",
        "        roots.clear();\n        roots.push(input.clone());",
    ),
    (
        "reader_omits_new_root",
        "queue_reader",
        "extend",
        "rust/queue_reader.rs",
        "        roots.push(input.clone());",
        "",
    ),
    (
        "reader_duplicates_existing_root",
        "queue_reader",
        "extend",
        "rust/queue_reader.rs",
        "    if !contains(roots, input) {",
        "    if true {",
    ),
    (
        "reader_register_wrong_identity",
        "queue_reader",
        "register",
        "rust/queue_reader.rs",
        "\n    table.insert(id.clone(), create(lease, input));",
        '\n    table.insert("wrong".to_owned(), create(lease, input));',
    ),
    (
        "reader_register_rejects_valid",
        "queue_reader",
        "register",
        "rust/queue_reader.rs",
        "pub fn register(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {",
        "pub fn register(readers: &mut Value, lease: &Value, input: &Value) -> Result<(), Error> {\n    return Err(Error::State);",
    ),
    (
        "reader_progress_overwrites_old_roots",
        "queue_reader",
        "progress",
        "rust/queue_reader.rs",
        "                extend(entry, input)?;",
        "                *entry = create(lease, input);",
    ),
    (
        "reader_progress_keeps_null_entry",
        "queue_reader",
        "progress",
        "rust/queue_reader.rs",
        "                *entry = create(lease, input);",
        "                return Ok(());",
    ),
    (
        "reader_progress_omits_new_attempt",
        "queue_reader",
        "progress",
        "rust/queue_reader.rs",
        "            table.insert(id.clone(), create(lease, input));",
        "",
    ),
    (
        "reader_remove_wrong_attempt",
        "queue_reader",
        "remove",
        "rust/queue_reader.rs",
        "    table.remove(id);",
        '    table.remove(&"wrong".to_owned());',
    ),
    (
        "reader_done_repeats_first",
        "queue_reader",
        "done",
        "rust/queue_reader.rs",
        "identity(&leases[index])",
        "identity(&leases[0])",
    ),
    (
        "reader_done_omits_last",
        "queue_reader",
        "done",
        "rust/queue_reader.rs",
        "while index < leases.len() {",
        "while index < leases.len() && index < leases.len() - 1 {",
    ),
    (
        "reader_done_swallows_missing_identity",
        "queue_reader",
        "done",
        "rust/queue_reader.rs",
        "            return Err(Error::Identity);\n        };\n        remove(readers, id)?;",
        "            return Ok(());\n        };\n        remove(readers, id)?;",
    ),
    (
        "reader_wrong_worker",
        "queue_reader",
        "matches_worker",
        "rust/queue_reader.rs",
        'let owner = "worker_id".index_into(lease)',
        'let owner = "attempt_id".index_into(lease)',
    ),
    (
        "environment_reader_equality_unobserved",
        "queue_reader",
        "contains",
        "verification/common/verus/serde_json.rs",
        "            r ==> forall|depth: nat| observe(*left, depth) == observe(*right, depth);",
        "            true;",
    ),
    (
        "environment_reader_equality_unbound",
        "queue_reader",
        "contains",
        "verification/common/verus/serde_json.rs",
        "ensures r == equal(*left, *right),",
        "ensures true,",
    ),
    (
        "environment_reader_mutation_unframed",
        "queue_reader",
        "extend",
        "verification/common/verus/serde_json.rs",
        "                && (forall|name: Seq<char>| name != key@ ==> field(*final(value), name) == field(*old(value), name))",
        "",
    ),
    (
        "receipt_wrong_lease",
        "queue_receipt",
        "submission",
        "rust/queue_receipt.rs",
        'fields.insert("lease".to_owned(), lease.clone());',
        'fields.insert("lease".to_owned(), receipt.clone());',
    ),
    (
        "receipt_missing_payload",
        "queue_receipt",
        "submission",
        "rust/queue_receipt.rs",
        '    fields.insert("receipt".to_owned(), receipt.clone());',
        "",
    ),
    (
        "receipt_wrong_submission_id",
        "queue_receipt",
        "record",
        "rust/queue_receipt.rs",
        '"submission_id".index_into(receipt)',
        '"task_id".index_into(receipt)',
    ),
    (
        "receipt_omits_commit",
        "queue_receipt",
        "record",
        "rust/queue_receipt.rs",
        "        items.push(receipt.clone());",
        "",
    ),
    (
        "receipt_wrong_commit",
        "queue_receipt",
        "record",
        "rust/queue_receipt.rs",
        "        items.push(receipt.clone());",
        "        items.push(lease.clone());",
    ),
    (
        "receipt_loses_prior_commits",
        "queue_receipt",
        "record",
        "rust/queue_receipt.rs",
        "        items.push(receipt.clone());",
        "        items.clear();\n        items.push(receipt.clone());",
    ),
    (
        "receipt_accepts_invalid_commit_table",
        "queue_receipt",
        "record",
        "rust/queue_receipt.rs",
        "let Value::Array(items) = value else {\n            return Err(Error::State);",
        "let Value::Array(items) = value else {\n            return Ok(());",
    ),
    (
        "producer_wrong_continuation",
        "queue_receipt",
        "continue_producer",
        "rust/queue_receipt.rs",
        'let value = "producer_state".index_into(event)',
        'let value = "producer_id".index_into(event)',
    ),
    (
        "producer_updates_wrong_id",
        "queue_receipt",
        "continue_producer",
        "rust/queue_receipt.rs",
        "table.insert(id.clone(), value.clone());",
        'table.insert("wrong".to_owned(), value.clone());',
    ),
)
for _name, _module, _function, *_ in QUEUE_READER_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_TASK_MUTATIONS = (
    (
        "queue_task_accepts_missing",
        "queue_task",
        "valid_target",
        "rust/queue_task.rs",
        'kind == "Submitted" || matches!(task, Value::Object(_))',
        "true",
    ),
    (
        "queue_task_rejects_present",
        "queue_task",
        "valid_target",
        "rust/queue_task.rs",
        'kind == "Submitted" || matches!(task, Value::Object(_))',
        "false",
    ),
    (
        "queue_task_omits_initial_generation",
        "queue_task",
        "submitted",
        "rust/queue_task.rs",
        '    fields.insert("generation".to_owned(), Value::Number(0u64.into()));',
        "",
    ),
    (
        "queue_task_wrong_initial_spec",
        "queue_task",
        "submitted",
        "rust/queue_task.rs",
        'fields.insert("spec".to_owned(), spec.clone());',
        'fields.insert("spec".to_owned(), Value::Null);',
    ),
    (
        "queue_task_wrong_retry_count",
        "queue_task",
        "retry",
        "rust/queue_task.rs",
        "crate::bounds::add(failures, 1)",
        "crate::bounds::add(failures, 0)",
    ),
    (
        "queue_task_retry_at_limit",
        "queue_task",
        "retry",
        "rust/queue_task.rs",
        "if failures < limit {",
        "if failures <= limit {",
    ),
    (
        "queue_task_retains_retry_lease",
        "queue_task",
        "retry",
        "rust/queue_task.rs",
        '    fields.insert("lease".to_owned(), Value::Null);\n    Ok(())\n}\n\n#[cfg_attr(straw_verus, verus_spec(result =>\n    requires crate::environment::string_order_contract(),\n    ensures result.is_ok() == (*old(task) is Object),',
        "    Ok(())\n}\n\n#[cfg_attr(straw_verus, verus_spec(result =>\n    requires crate::environment::string_order_contract(),\n    ensures result.is_ok() == (*old(task) is Object),",
    ),
    (
        "queue_task_accepts_exhaustion",
        "queue_task",
        "retry",
        "rust/queue_task.rs",
        "return Err(Error::Counter);",
        "return Ok(());",
    ),
    (
        "queue_task_wrong_terminal",
        "queue_task",
        "terminal",
        "rust/queue_task.rs",
        'Terminal::Cancelled => "cancelled",',
        'Terminal::Cancelled => "completed",',
    ),
    (
        "queue_task_wrong_generation",
        "queue_task",
        "leased",
        "rust/queue_task.rs",
        'let generation = match "generation".index_into(lease)',
        'let generation = match "task_id".index_into(lease)',
    ),
    (
        "queue_task_wrong_lease",
        "queue_task",
        "leased",
        "rust/queue_task.rs",
        'fields.insert("lease".to_owned(), lease.clone());',
        'fields.insert("lease".to_owned(), Value::Null);',
    ),
    (
        "queue_task_wrong_failure_reason",
        "queue_task",
        "failure",
        "rust/queue_task.rs",
        'let reason = match "type".index_into(event)',
        'let reason = match "failure".index_into(event)',
    ),
    (
        "queue_task_forces_terminal_failure",
        "queue_task",
        "failed",
        "rust/queue_task.rs",
        "    if retryable {\n        retry(task)?;",
        "    if false {\n        retry(task)?;",
    ),
    (
        "queue_task_omits_failure_payload",
        "queue_task",
        "failed",
        "rust/queue_task.rs",
        '    fields.insert("failure".to_owned(), value);',
        "",
    ),
    (
        "queue_task_wrong_progress_input",
        "queue_task",
        "progress",
        "rust/queue_task.rs",
        'let input = match "input_ref".index_into(event)',
        'let input = match "metadata".index_into(event)',
    ),
    (
        "queue_task_omits_progress_metadata",
        "queue_task",
        "progress",
        "rust/queue_task.rs",
        '        copy_present(fields, event, "metadata");',
        "",
    ),
    (
        "queue_task_yields_ordinary_progress",
        "queue_task",
        "progress",
        "rust/queue_task.rs",
        "    if yielding {\n        let Value::Object(fields) = task else {",
        "    if true {\n        let Value::Object(fields) = task else {",
    ),
    (
        "queue_task_wrong_yield_order",
        "queue_task",
        "progress",
        "rust/queue_task.rs",
        '        fields.insert("pending_order".to_owned(), Value::Number(order.into()));',
        '        fields.insert("pending_order".to_owned(), Value::Number(0u64.into()));',
    ),
    (
        "environment_task_mutation_unframed",
        "queue_task",
        "progress",
        "verification/common/verus/serde_json.rs",
        "                && (forall|name: Seq<char>| name != key@ ==> field(*final(value), name) == field(*old(value), name))",
        "",
    ),
    (
        "environment_task_mutation_wrong_field",
        "queue_task",
        "progress",
        "verification/common/verus/serde_json.rs",
        "&& field(*final(value), key@) == Some(*final(item))",
        "&& field(*final(value), Seq::empty()) == Some(*final(item))",
    ),
    (
        "environment_task_missing_mutates",
        "queue_task",
        "progress",
        "verification/common/verus/serde_json.rs",
        "None => field(*old(value), key@).is_none() && *final(value) == *old(value),",
        "None => field(*old(value), key@).is_none(),",
    ),
)
for _name, _module, _function, *_ in QUEUE_TASK_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_EVENT_MUTATIONS = (
    (
        "queue_event_omits_submitted",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        'collect(array_field(event, "tasks")?, Some("task_id"))?',
        "Vec::new()",
    ),
    (
        "queue_event_wrong_lease_field",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        'collect(array_field(event, "leases")?, Some("task_id"))?',
        'collect(array_field(event, "tasks")?, Some("task_id"))?',
    ),
    (
        "queue_event_wrong_epoch_field",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        'collect(array_field(event, "revoked")?, None)?',
        'collect(array_field(event, "task_ids")?, None)?',
    ),
    (
        "queue_event_omits_failed",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        'kind == "Expired" || kind == "Failed"',
        'kind == "Expired"',
    ),
    (
        "queue_event_omits_progress",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        'kind == "Cancelled" || kind == "Yielded" || kind == "TaskProgress"',
        'kind == "Cancelled" || kind == "Yielded"',
    ),
    (
        "queue_event_wrong_completed_source",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        '"receipt".index_into(event)',
        '"lease".index_into(event)',
    ),
    (
        "queue_event_repeats_first_id",
        "queue_event",
        "collect",
        "rust/queue_event.rs",
        "string_field(&items[index], name)?",
        "string_field(&items[0], name)?",
    ),
    (
        "queue_event_reverses_order",
        "queue_event",
        "collect",
        "rust/queue_event.rs",
        "result.push(id.to_owned());",
        "result.insert(0, id.to_owned());",
    ),
    (
        "queue_event_omits_last_id",
        "queue_event",
        "collect",
        "rust/queue_event.rs",
        "while index < items.len() {",
        "while index < items.len() && index < items.len() - 1 {",
    ),
    (
        "queue_event_accepts_invalid_id",
        "queue_event",
        "collect",
        "rust/queue_event.rs",
        "Some(name) => string_field(&items[index], name)?,",
        'Some(name) => match string_field(&items[index], name) { Ok(id) => id, Err(_) => "" },',
    ),
    (
        "queue_event_rejects_valid",
        "queue_event",
        "prepare",
        "rust/queue_event.rs",
        '    let kind = string_field(event, "type")?;',
        '    return Err(Error::Identity);\n    let kind = string_field(event, "type")?;',
    ),
    (
        "environment_queue_index_wrong_key",
        "queue_event",
        "string_field",
        "verification/common/verus/serde_json.rs",
        "ensures (match r { Some(item) => Some(*item), None => None }) == field(*value, key@);",
        "ensures (match r { Some(item) => Some(*item), None => None }) == field(*value, Seq::empty());",
    ),
    (
        "environment_queue_index_unconstrained",
        "queue_event",
        "array_field",
        "verification/common/verus/serde_json.rs",
        "ensures (match r { Some(item) => Some(*item), None => None }) == field(*value, key@);",
        "ensures true;",
    ),
    (
        "environment_queue_target_parse_inconsistent",
        "queue_event_equivalence",
        "parsed_targets_equal",
        "verification/common/verus/serde_json.rs",
        "ensures observe(left, depth) == observe(right, depth),",
        "ensures true,",
    ),
    (
        "queue_event_spec_ignores_order",
        "queue_event",
        "collect",
        "rust/queue_event.rs",
        "Seq::new(items.len(), |i: int| identity(items[i], key)->Some_0)",
        "Seq::new(items.len(), |_i: int| identity(items[0], key)->Some_0)",
    ),
)
for _name, _module, _function, *_ in QUEUE_EVENT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


GC_IO_MUTATIONS = (
    (
        "gc_io_stat_missing_skips_unlink",
        "gc_io",
        "unlink",
        "let removed = effects.remove(index, &path)?;",
        "let removed = if metadata.is_some() { effects.remove(index, &path)? } else { false };",
    ),
    (
        "gc_io_wrong_target",
        "gc_io",
        "unlink",
        "let path = effects.resolve(index)?;",
        "let path = effects.resolve(0)?;",
    ),
    (
        "gc_io_wrong_parent",
        "gc_io",
        "unlink",
        "parent: effects.parent(index, &path),",
        "parent: effects.parent(0, &path),",
    ),
    (
        "gc_io_ignores_unlink_error",
        "gc_io",
        "unlink",
        "let removed = effects.remove(index, &path)?;",
        "let removed = match effects.remove(index, &path) { Ok(value) => value, Err(_) => false };",
    ),
    (
        "gc_io_skips_directory_sync",
        "gc_io",
        "sync_parent",
        "Some(directory) => effects.sync(&directory)?,",
        "Some(_) => (),",
    ),
    (
        "gc_io_ignores_directory_sync_error",
        "gc_io",
        "sync_parent",
        "Some(directory) => effects.sync(&directory)?,",
        "Some(directory) => { let _ = effects.sync(&directory); },",
    ),
    ("gc_io_skips_parent_repair", "gc_io", "sync_parent", "            effects.repair(parent)?;", ""),
    (
        "gc_io_ignores_parent_repair_error",
        "gc_io",
        "sync_parent",
        "            effects.repair(parent)?;",
        "            let _ = effects.repair(parent);",
    ),
    ("gc_io_all_errors_missing", "gc_io", "classify", "if error.is_missing() {", "if true {"),
    ("gc_io_missing_is_error", "gc_io", "classify", "if error.is_missing() {", "if false {"),
    (
        "gc_io_success_is_missing",
        "gc_io",
        "classify",
        "        Ok(value) => Ok(Some(value)),",
        "        Ok(_) => Ok(None),",
    ),
    ("environment_gc_io_unlink_lies", "gc_io", "unlink", "&& final(self)@.absent.contains(index as nat),", "&& true,"),
    (
        "environment_gc_io_wrong_directory",
        "gc_io",
        "sync_parent",
        "&& (result->Ok_0.is_some() ==> result->Ok_0->Some_0@ == *parent),",
        "&& true,",
    ),
    (
        "environment_gc_io_sync_lies",
        "gc_io",
        "sync_parent",
        "&& under(old(self)@, Set::empty().insert(directory@)).subset_of(final(self)@.durable_absent),",
        "&& true,",
    ),
    (
        "environment_gc_io_repair_lies",
        "gc_io",
        "sync_parent",
        "&& under(old(self)@, Set::empty().insert(*parent)).subset_of(final(self)@.durable_absent),",
        "&& true,",
    ),
    (
        "directory_io_skips_first",
        "directory_io",
        "sync_all",
        "let mut state = directory::start(directories.len() as u64);",
        "let mut state = directory::advance(directory::start(directories.len() as u64), true);",
    ),
    (
        "directory_io_wrong_directory",
        "directory_io",
        "sync_all",
        "effects.sync(&directories[index as usize])",
        "effects.sync(&directories[0])",
    ),
    (
        "directory_io_skips_all",
        "directory_io",
        "sync_all",
        "let mut state = directory::start(directories.len() as u64);",
        "return Ok(());\n    let mut state = directory::start(directories.len() as u64);",
    ),
    ("directory_io_ignores_error", "directory_io", "sync_all", "        outcome?;", "        let _ = outcome;"),
    ("directory_io_rejects_completion", "directory_io", "sync_all", "if !directory::finished(&state) {", "if true {"),
    (
        "environment_directory_barrier_lies",
        "directory_io",
        "sync_all",
        "result.is_ok() ==> final(self).success_context() == old(self).success_context() && final(self)@.synced.contains(*path),",
        "result.is_ok() ==> final(self).success_context() == old(self).success_context(),",
    ),
    (
        "environment_directory_loses_prior_sync",
        "directory_io",
        "sync_all",
        "old(self)@.synced.subset_of(final(self)@.synced),",
        "true,",
    ),
)
for _name, _module, _function, *_ in GC_IO_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


GC_RECEIPT_MUTATIONS = (
    (
        "gc_receipt_omits_name",
        "event",
        "values.push(Value::String(paths[i].clone()));",
        "if i > 0 { values.push(Value::String(paths[i].clone())); }",
    ),
    (
        "gc_receipt_marks_instead",
        "event",
        'object.insert("reclaimed".to_owned(), Value::Array(values));',
        'object.insert("deleted".to_owned(), Value::Array(values));',
    ),
    ("gc_receipt_wrong_encoding", "event", "Value::Number(1u64.into())", "Value::Number(0u64.into())"),
    (
        "gc_receipt_changes_owners",
        "event",
        "Value::Object(object)\n}",
        'object.insert("owners".to_owned(), Value::Null);\n    Value::Object(object)\n}',
    ),
    ("gc_receipt_skips_nonempty", "record", "if paths.is_empty() {", "if true {"),
    ("gc_receipt_writes_empty", "record", "if paths.is_empty() {", "if false {"),
    (
        "gc_receipt_ignores_append_error",
        "record",
        "crate::catalog_append::append(log, resolver, run, value, device)?;",
        "let _ = crate::catalog_append::append(log, resolver, run, value, device);",
    ),
    ("gc_receipt_wrong_targets", "record", "let value = event(paths);", "let value = event(&[]);"),
    (
        "gc_receipt_wrong_run",
        "record",
        "crate::catalog_append::append(log, resolver, run, value, device)?;",
        'crate::catalog_append::append(log, resolver, "wrong-run", value, device)?;',
    ),
    (
        "gc_receipt_erases_other_pending",
        "reclaim_fold",
        "pending: s.pending.difference(retired),",
        "pending: Set::empty(),",
    ),
    (
        "gc_receipt_erases_tombstones",
        "reclaim_one",
        "packs: s.packs.remove_keys(retired), ..s",
        "packs: s.packs.remove_keys(retired), deleted: s.deleted.difference(retired), ..s",
    ),
)
for _name, _function, *_ in GC_RECEIPT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "gc_receipt"


GC_MARK_MUTATIONS = (
    ("gc_mark_skips_candidates", "mark", "rust/gc_mark.rs", "if !candidates.is_empty() {", "if false {"),
    (
        "gc_mark_marks_live",
        "mark",
        "rust/gc_mark.rs",
        "let paths = snapshot(&candidates);",
        "let paths = snapshot(&live);",
    ),
    (
        "gc_mark_omits_name",
        "event",
        "rust/gc_mark.rs",
        "values.push(Value::String(paths[i].clone()));",
        "if i > 0 { values.push(Value::String(paths[i].clone())); }",
    ),
    ("gc_mark_wrong_encoding", "event", "rust/gc_mark.rs", "Value::Number(1u64.into())", "Value::Number(0u64.into())"),
    (
        "gc_mark_reclaims_instead",
        "event",
        "rust/gc_mark.rs",
        'object.insert("deleted".to_owned(), Value::Array(values));',
        'object.insert("reclaimed".to_owned(), Value::Array(values));',
    ),
    (
        "gc_mark_ignores_append_error",
        "mark",
        "rust/gc_mark.rs",
        "crate::catalog_append::append(log, resolver, run, value, device)?;",
        "let _ = crate::catalog_append::append(log, resolver, run, value, device);",
    ),
    (
        "gc_mark_repeats_receipted",
        "mark",
        "rust/gc_mark.rs",
        "let targets = snapshot(&log.indexes().pending_deletions);",
        "let targets = snapshot(&log.indexes().deleted);",
    ),
    (
        "gc_mark_forgets_old_pending",
        "mark",
        "rust/gc_mark.rs",
        "let targets = snapshot(&log.indexes().pending_deletions);",
        "let targets = snapshot(&candidates);",
    ),
    (
        "gc_mark_snapshot_skips_entry",
        "snapshot",
        "rust/gc_mark.rs",
        "result.push(path.clone());",
        "if result.is_empty() { result.push(path.clone()); }",
    ),
    (
        "environment_json_new_not_empty",
        "event",
        "verification/common/verus/serde_json.rs",
        "ensures entries(r) == Seq::<(String, Value)>::empty();",
        "ensures true;",
    ),
    (
        "environment_json_number_wrong",
        "event",
        "verification/common/verus/serde_json.rs",
        "ensures unsigned(r) == Some(number);",
        "ensures true;",
    ),
)
for _name, _function, *_ in GC_MARK_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "gc_mark"


CLOSURE_MUTATIONS = (
    (
        "closure_drops_successor",
        "collect",
        "rust/closure.rs",
        "crate::traversal::schedule(&mut frontier, child_key, child);",
        "let _ = (child_key, child);",
    ),
    (
        "closure_omits_node_pack",
        "collect",
        "rust/closure.rs",
        "paths.insert(expansion.pack);",
        "let _ = expansion.pack;",
    ),
    (
        "closure_omits_cached_descendants",
        "collect",
        "rust/closure.rs",
        "add_cached(&mut paths, cached_paths);",
        "let _ = cached_paths;",
    ),
    (
        "closure_caches_failed_prefix",
        "collect",
        "rust/closure.rs",
        "let expansion = source.expand(&key, &reference)?;",
        "let expansion = match source.expand(&key, &reference) { Ok(value) => value, Err(error) => { cache.insert(root_key.clone(), copy_paths(&paths)); return Err(error); } };",
    ),
    (
        "closure_skips_frontier",
        "collect",
        "rust/closure.rs",
        "let Some((key, reference)) = crate::traversal::next(&mut frontier) else {",
        "let Some((key, reference)) = None::<(String, S::Reference)> else {",
    ),
    (
        "closure_caches_empty_result",
        "collect",
        "rust/closure.rs",
        "cache.insert(root_key, copied);",
        "cache.insert(root_key, BTreeSet::new());",
    ),
    (
        "closure_wrong_cached_root",
        "collect",
        "rust/closure.rs",
        "if let Some(paths) = cache.get(&root_key) {",
        'if let Some(paths) = cache.get(&"other".to_owned()) {',
    ),
    ("closure_allows_tombstone", "resolve", "rust/closure.rs", "if reclaimed(&paths, deleted) {", "if false {"),
    ("closure_rejects_all_roots", "resolve", "rust/closure.rs", "if reclaimed(&paths, deleted) {", "if true {"),
    (
        "environment_closure_partial_children",
        "collect",
        "rust/closure.rs",
        "&& keys(result->Ok_0.children@) == old(self)@.nodes[*key].children",
        "&& keys(result->Ok_0.children@).subset_of(old(self)@.nodes[*key].children)",
    ),
    (
        "environment_closure_wrong_pack",
        "collect",
        "rust/closure.rs",
        "result->Ok_0.pack == old(self)@.nodes[*key].pack",
        "true",
    ),
    (
        "environment_closure_wrong_binding",
        "collect",
        "rust/closure.rs",
        "&& bound(old(self)@, result->Ok_0.children@),",
        "&& true,",
    ),
    (
        "environment_closure_partial_cache",
        "collect",
        "rust/closure.rs",
        "cache[k]@ == packs(g, k)",
        "cache[k]@.subset_of(packs(g, k))",
    ),
)
for _name, _function, *_ in CLOSURE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "closure"


CATALOG_TRANSACTION_MUTATIONS = (
    ("transaction_skips_binding", "execute", "effect(effects.check_binding())?;", ""),
    ("transaction_ignores_nested", "execute", "if effects.active() {", "if false {"),
    ("transaction_rejects_every_entry", "execute", "if effects.active() {", "if true {"),
    ("transaction_wrong_inode", "locked", "if !crate::identities::same_log(expected, observed) {", "if false {"),
    ("transaction_skips_refresh", "locked", "effect(effects.refresh(lock))?;", ""),
    ("transaction_skips_context", "locked", "effect(effects.check_context())?;", ""),
    ("transaction_leaves_active", "execute", "effects.set_active(false);", ""),
    (
        "transaction_invokes_twice",
        "locked",
        "let value = effect(effects.invoke(lock))?;",
        "let _ = effect(effects.invoke(lock))?;\n    let value = effect(effects.invoke(lock))?;",
    ),
    (
        "transaction_unlocks_before_callback",
        "execute",
        "let result = locked(effects, &lock);",
        "effect(lock.release())?;\n    let result = locked(effects, &lock);",
    ),
    ("transaction_ignores_unlock_error", "finish", "effect(unlocked)?;", ""),
    (
        "transaction_overwrites_original_error",
        "finish",
        "        Err(error) => Err(error),\n    }\n}",
        "        Err(error) => { effect(unlocked)?; Err(error) },\n    }\n}",
    ),
    (
        "environment_transaction_wrong_lock",
        "execute",
        "ensures result.is_ok() ==> result->Ok_0@.identity == self@.current,",
        "ensures true,",
    ),
    ("environment_transaction_false_acquire", "execute", "result.is_ok() ==> final(self)@.held,", "true,"),
    ("environment_transaction_refresh_unready", "locked", "result.is_ok() ==> final(self)@.ready,", "true,"),
    (
        "environment_transaction_callback_rebinds",
        "locked",
        "final(self)@.active ==> final(self)@.expected == old(self)@.expected\n                && final(self)@.refreshed,",
        "final(self)@.active ==> final(self)@.refreshed,",
    ),
)
for _name, _function, *_ in CATALOG_TRANSACTION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "catalog_transaction"


FILE_ADAPTER_MUTATIONS = (
    ("source_ignores_uncertainty", "wal_source", "length", "rust/wal_source.rs", "if self.uncertain {", "if false {"),
    ("source_rejects_all_lengths", "wal_source", "length", "rust/wal_source.rs", "if self.uncertain {", "if true {"),
    (
        "source_read_forgets_quarantine",
        "wal_source",
        "read",
        "rust/wal_source.rs",
        "self.uncertain = true;\n        let input = &mut self.input;\n        let bytes",
        "self.uncertain = false;\n        let input = &mut self.input;\n        let bytes",
    ),
    (
        "source_keeps_buffer",
        "wal_source",
        "discard_buffer",
        "rust/wal_source.rs",
        "*input = Some(Input::Plain(input.take().unwrap().into_file()));",
        "let _ = input;",
    ),
    ("source_omits_truncate", "wal_source", "truncate", "rust/wal_source.rs", "file.truncate(end)?;", "let _ = end;"),
    ("source_keeps_stale_length", "wal_source", "truncate", "rust/wal_source.rs", "self.size = end;", "let _ = end;"),
    ("source_omits_barrier", "wal_source", "sync", "rust/wal_source.rs", "sync.sync(file)?;", "let _ = (sync, file);"),
    (
        "source_wrong_seek_offset",
        "wal_source",
        "seek",
        "rust/wal_source.rs",
        "let outcome = file.seek(offset);",
        "let outcome = file.seek(0);",
    ),
    (
        "source_seek_loses_failed_handle",
        "wal_source",
        "seek",
        "rust/wal_source.rs",
        "let outcome = file.seek(offset);",
        "file.seek(offset)?;\n        let outcome: Result<(), F::Error> = Ok(());",
    ),
    (
        "source_seek_drops_buffer_mode",
        "wal_source",
        "seek",
        "rust/wal_source.rs",
        "*slot = Some(if buffered {",
        "*slot = Some(if false {",
    ),
    (
        "environment_source_wrong_buffer_identity",
        "wal_source",
        "from_file",
        "rust/wal_source.rs",
        "ensures result.loan() == self.loan(), result.valid(), result@ == self@,",
        "ensures result.loan() == self.loan(), result.valid(), result@.bytes == self@.bytes && result@.cursor == self@.cursor && result@.durable == self@.durable,",
    ),
    (
        "environment_source_false_barrier",
        "wal_source",
        "sync",
        "rust/wal_source.rs",
        "&& final(file)@.durable == final(file)@.bytes.len(),",
        "&& true,",
    ),
    (
        "environment_source_empty_read_forbidden",
        "wal_device",
        "empty_read_matches_any_cursor",
        "rust/wal_device.rs",
        "length == 0 || (cursor + length <= bytes.len()",
        "false || (cursor + length <= bytes.len()",
    ),
    ("sink_wrong_position", "wal_sink", "inspect", "rust/wal_sink.rs", "cursor: self.file.position()?,", "cursor: 0,"),
    ("sink_omits_write", "wal_sink", "write", "rust/wal_sink.rs", "self.file.write_all(part)?;", "let _ = part;"),
    (
        "sink_omits_queue_sync",
        "wal_sink",
        "sync",
        "rust/wal_sink.rs",
        'self.hooks.point(&mut self.file, "before_file_sync")?;\n                self.file.sync_all()?;',
        'self.hooks.point(&mut self.file, "before_file_sync")?;',
    ),
    (
        "sink_omits_catalog_sync",
        "wal_sink",
        "sync",
        "rust/wal_sink.rs",
        '.point(&mut self.file, "before_storage_catalog_sync")?;\n                self.file.sync_all()?;',
        '.point(&mut self.file, "before_storage_catalog_sync")?;',
    ),
    (
        "environment_sink_hook_changes_file",
        "wal_sink",
        "write",
        "rust/wal_sink.rs",
        "result.is_ok() ==> final(file)@ == old(file)@,",
        "true,",
    ),
    (
        "environment_sink_false_sync",
        "wal_sink",
        "sync",
        "rust/wal_sink.rs",
        "&& final(self)@.durable == final(self)@.bytes.len(),",
        "&& true,",
    ),
)
for _name, _module, _function, *_ in FILE_ADAPTER_MUTATIONS:
    _owner = {"wal_source": "Source::", "wal_sink": "Appender::"}.get(_module, "")
    NEGATIVE_FUNCTIONS[_name] = _owner + _function
    NEGATIVE_MODULES[_name] = _module


CLOSURE_NODE_MUTATIONS = (
    (
        "closure_node_drops_dependencies",
        "children",
        "if let Err(error) = append(digester, dependencies, &mut output) {",
        "if let Err(error) = append(digester, &[], &mut output) {",
    ),
    (
        "closure_node_reorders_children",
        "children",
        "if let Err(error) = append(digester, records, &mut output) {",
        "if let Err(error) = append(digester, dependencies, &mut output) {",
    ),
    (
        "closure_node_wrong_digest",
        "append",
        "let key = digester.digest(&values[i])?;",
        "let key = digester.digest(&Value::Null)?;",
    ),
    (
        "closure_node_wrong_reference",
        "append",
        "output.push((key, values[i].clone()));",
        "output.push((key, Value::Null));",
    ),
    (
        "closure_node_wrong_kind",
        "kind",
        'if crate::catalog_json::get(reference, "manifest").is_some() {',
        'if crate::catalog_json::get(reference, "segment").is_some() {',
    ),
    ("closure_node_accepts_invalid_records", "children", "return Err(Failure::Records);", "return Ok(Vec::new());"),
    (
        "closure_node_wrong_path",
        "path",
        'let Some(value) = crate::catalog_json::get(segment, "path") else {',
        'let Some(value) = crate::catalog_json::get(segment, "ordinal") else {',
    ),
    (
        "environment_closure_node_unbound_digest",
        "append",
        "result.is_ok() ==> result->Ok_0 == (old(self)@)(crate::json_specs::content(*value)),",
        "true,",
    ),
)
for _name, _function, *_ in CLOSURE_NODE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "closure_node"


JOURNAL_STATE_MUTATIONS = (
    (
        "journal_accepts_empty_events",
        "journal_recovery",
        "valid_events",
        "rust/journal_recovery.rs",
        "if items.is_empty() {",
        "if false {",
    ),
    (
        "journal_skips_event_validation",
        "journal_recovery",
        "valid_events",
        "rust/journal_recovery.rs",
        "while index < items.len() {",
        "while false {",
    ),
    (
        "journal_accepts_nontext_type",
        "journal_recovery",
        "valid_event",
        "rust/journal_recovery.rs",
        "kind.as_str().is_some()",
        "true",
    ),
    (
        "journal_accepts_missing_type",
        "journal_recovery",
        "valid_event",
        "rust/journal_recovery.rs",
        'let Some(kind) = crate::catalog_json::get(value, "type") else {\n        return false;',
        'let Some(kind) = crate::catalog_json::get(value, "type") else {\n        return true;',
    ),
    (
        "journal_parses_wrong_payload",
        "journal_recovery",
        "apply",
        "rust/journal_recovery.rs",
        "serde_json::from_slice::<Value>(payload)",
        "serde_json::from_slice::<Value>(&[])",
    ),
    (
        "journal_omits_transaction",
        "journal_recovery",
        "apply",
        "rust/journal_recovery.rs",
        "        self.transactions.push(value);",
        "        let _ = value;",
    ),
    (
        "journal_discards_prior_transaction",
        "journal_recovery",
        "apply",
        "rust/journal_recovery.rs",
        "        self.transactions.push(value);",
        "        self.transactions.clear();\n        self.transactions.push(value);",
    ),
    (
        "journal_accepts_invalid_events",
        "journal_recovery",
        "apply",
        "rust/journal_recovery.rs",
        "if !valid_events(&value) {",
        "if false {",
    ),
    (
        "journal_spurious_events_error",
        "journal_recovery",
        "apply",
        "rust/journal_recovery.rs",
        "if !valid_events(&value) {",
        "if true {",
    ),
    (
        "journal_replay_omits_reset",
        "journal_recovery",
        "recover",
        "rust/journal_recovery.rs",
        "    transactions.clear();",
        "",
    ),
    (
        "journal_replay_wrong_kind",
        "journal_recovery",
        "recover",
        "rust/journal_recovery.rs",
        "        crate::framing::Kind::Queue,",
        "        crate::framing::Kind::Catalog,",
    ),
    (
        "journal_replay_skips_barrier",
        "journal_recovery",
        "recover",
        "rust/journal_recovery.rs",
        "        writable,\n    );",
        "        false,\n    );",
    ),
    (
        "journal_replay_omits_output",
        "journal_recovery",
        "recover",
        "rust/journal_recovery.rs",
        "    *transactions = target.transactions;",
        "",
    ),
    (
        "journal_recovery_wrong_sequence",
        "journal_state",
        "recover",
        "rust/journal_state.rs",
        "        self.sequence = boundary.sequence;",
        "        self.sequence = 0;",
    ),
    (
        "journal_recovery_wrong_cursor",
        "journal_state",
        "recover",
        "rust/journal_state.rs",
        "        self.offset = boundary.end;",
        "        self.offset = 0;",
    ),
    (
        "journal_recovery_omits_quarantine",
        "journal_state",
        "recover",
        "rust/journal_state.rs",
        "        self.poisoned = true;\n        let boundary =",
        "        self.poisoned = false;\n        let boundary =",
    ),
    (
        "journal_append_omits_quarantine",
        "journal_state",
        "append_to",
        "rust/journal_state.rs",
        "        self.poisoned = true;\n        let appended =",
        "        self.poisoned = false;\n        let appended =",
    ),
    (
        "journal_append_wrong_receipt",
        "journal_state",
        "append_to",
        "rust/journal_state.rs",
        "        Ok(self.sequence - 1)",
        "        Ok(self.sequence)",
    ),
    (
        "journal_append_ignores_read_only",
        "journal_state",
        "append_to",
        "rust/journal_state.rs",
        "if self.poisoned || self.read_only {",
        "if self.poisoned {",
    ),
    (
        "journal_append_spurious_unavailable",
        "journal_state",
        "append_to",
        "rust/journal_state.rs",
        "if self.poisoned || self.read_only {",
        "if true {",
    ),
    (
        "environment_journal_parse_unbound",
        "journal_recovery",
        "apply",
        "verification/common/verus/serde_json.rs",
        "ensures r.is_ok() ==> parses(bytes@, r->Ok_0);",
        "ensures true;",
    ),
)
JOURNAL_STATE_MUTATIONS += (
    (
        "prefix_projection_advances_partial",
        "prefix_projection",
        "exact_states",
        "verification/wal/verus/prefix_projection.rs",
        "if units == 3 { pending } else { before }",
        "if units >= 2 { pending } else { before }",
    ),
    (
        "prefix_projection_loses_old_state",
        "prefix_projection",
        "exact_states",
        "verification/wal/verus/prefix_projection.rs",
        "if units == 3 { pending } else { before }",
        "pending",
    ),
    (
        "prefix_projection_forgets_uncertain",
        "prefix_projection",
        "cross_projection",
        "verification/wal/verus/prefix_projection.rs",
        "        states(durable, current, if durable == current { 0 } else { 3 })",
        "        states(durable, current, 0)",
    ),
    (
        "prefix_projection_rounds_up",
        "prefix_projection",
        "complete_prefix_bound",
        "verification/wal/verus/prefix_projection.rs",
        "pub open spec fn complete(kept: nat) -> nat { kept / 3 }",
        "pub open spec fn complete(kept: nat) -> nat { (kept + 2) / 3 }",
    ),
    (
        "prefix_projection_atomic_logs",
        "prefix_projection",
        "independent_recovery",
        "verification/wal/verus/prefix_projection.rs",
        "ISet::new(|pair: (Q, C)| cross_states(qd, qc).contains(pair.0) && cross_states(cd, cc).contains(pair.1))",
        "ISet::new(|pair: (Q, C)| pair == (qd, cd) || pair == (qc, cc))",
    ),
)
for _name, _module, _function, *_ in JOURNAL_STATE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CATALOG_FRAME_MUTATIONS = (
    (
        "catalog_frame_accepts_wrong_run",
        "catalog_frame",
        "matches_run",
        "rust/catalog_frame.rs",
        "Some(value) => value == run,",
        "Some(value) => value != run,",
    ),
    (
        "catalog_frame_skips_run",
        "catalog_frame",
        "decode",
        "rust/catalog_frame.rs",
        "if !matches_run(&event, self.run) {",
        "if false {",
    ),
    (
        "catalog_frame_wrong_bytes",
        "catalog_frame",
        "decode",
        "rust/catalog_frame.rs",
        "serde_json::from_slice::<serde_json::Value>(raw)",
        'serde_json::from_slice::<serde_json::Value>(b"{}")',
    ),
    (
        "catalog_frame_rejects_valid_run",
        "catalog_frame",
        "decode",
        "rust/catalog_frame.rs",
        "if !matches_run(&event, self.run) {",
        "if true {",
    ),
    (
        "catalog_frame_drops_operations",
        "catalog_frame",
        "decode",
        "rust/catalog_frame.rs",
        "                Ok(operations)\n            }\n            Err(error) => Err(Error::Event(error)),",
        "                Ok(Vec::new())\n            }\n            Err(error) => Err(Error::Event(error)),",
    ),
    (
        "catalog_frame_wrong_known_graph",
        "catalog_frame",
        "decode",
        "rust/catalog_frame.rs",
        "catalog_decode::decode(self.resolver, &event, known)",
        "catalog_decode::decode(self.resolver, &event, &HashMap::new())",
    ),
    (
        "catalog_frame_keeps_closure_cache",
        "catalog_frame",
        "reset",
        "rust/catalog_frame.rs",
        "        self.closures.clear();",
        "",
    ),
    (
        "environment_catalog_decode_meaning_unbound",
        "catalog_target",
        "apply",
        "rust/catalog_target.rs",
        "result.is_ok() ==> crate::catalog_frame::meaning(old(self)@, raw@,\n                crate::catalog_state::edge_view(known@), result->Ok_0@)",
        "true",
    ),
    (
        "environment_catalog_reset_changes_namespace",
        "catalog_frame",
        "reset",
        "rust/catalog_decode.rs",
        "#[cfg_attr(straw_verus, verus_spec(requires old(self).valid(), ensures final(self).valid(), final(self)@ == old(self)@))]",
        "#[cfg_attr(straw_verus, verus_spec(requires old(self).valid(), ensures final(self).valid()))]",
    ),
)
for _name, _module, _function, *_ in CATALOG_FRAME_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


REQUEST_MUTATIONS = (
    ("request_ignores_conflict", "lookup", "rust/request.rs", "if previous != digest {", "if false {"),
    (
        "request_returns_digest_as_response",
        "lookup",
        "rust/request.rs",
        "Ok(Some(pair[1].clone()))",
        "Ok(Some(pair[0].clone()))",
    ),
    (
        "request_null_means_absent",
        "lookup",
        "rust/request.rs",
        "Ok(Some(pair[1].clone()))",
        "if pair[1].is_null() { Ok(None) } else { Ok(Some(pair[1].clone())) }",
    ),
    (
        "request_refuses_new_id",
        "lookup",
        "rust/request.rs",
        "let Some(prior) = requests.get(key) else {\n        return Ok(None);",
        "let Some(prior) = requests.get(key) else {\n        return Err(Conflict);",
    ),
    ("request_forgets_receipt", "remember", "rust/request.rs", "    requests.insert(key.clone(), receipt);", ""),
    (
        "request_wrong_id",
        "remember",
        "rust/request.rs",
        "requests.insert(key.clone(), receipt)",
        'requests.insert("other".to_owned(), receipt)',
    ),
    (
        "request_wrong_digest",
        "remember",
        "rust/request.rs",
        'let digest = copy_field(event, "request_digest");',
        'let digest = copy_field(event, "response");',
    ),
    (
        "request_wrong_saved_response",
        "remember",
        "rust/request.rs",
        'let response = copy_field(event, "response");',
        "let response = Value::Null;",
    ),
    ("request_peeks_digest", "response", "rust/request.rs", "Some(&pair[1])", "Some(&pair[0])"),
    (
        "environment_json_clone_loses_content",
        "lookup",
        JSON_SPECS,
        "content(r) == content(*value), text(r) == text(*value)",
        "true, text(r) == text(*value)",
    ),
    (
        "environment_json_clone_changes_digest",
        "copy_field",
        JSON_SPECS,
        "content(r) == content(*value), text(r) == text(*value),\n            forall|depth: nat| observe(r, depth) == observe(*value, depth)",
        "content(r) == content(*value)",
    ),
    (
        "environment_json_get_hides_receipt",
        "lookup",
        JSON_SPECS,
        "None => !contains_borrowed_key(object(*value), key)",
        "None => true",
    ),
    (
        "environment_json_insert_loses_receipt",
        "remember",
        JSON_SPECS,
        "object(*final(value)) == object(*old(value)).insert(key, item)",
        "true",
    ),
)
for _name, _function, *_ in REQUEST_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "request"


CATALOG_APPEND_MUTATIONS = (
    (
        "catalog_append_wrong_run",
        "prepare",
        "rust/catalog_append.rs",
        "Value::String(run.to_owned())",
        'Value::String("foreign".to_owned())',
    ),
    (
        "catalog_append_changes_owner",
        "prepare",
        "rust/catalog_append.rs",
        "    let value = Value::Object(object);",
        '    object.insert("owners".to_owned(), Value::Null);\n    let value = Value::Object(object);',
    ),
    (
        "catalog_append_serializes_null",
        "prepare",
        "rust/catalog_append.rs",
        "serde_json::to_vec(&value)",
        "serde_json::to_vec(&Value::Null)",
    ),
    (
        "catalog_append_drops_operations",
        "prepare",
        "rust/catalog_append.rs",
        "Ok(Prepared {\n        operations,\n        payload,\n    })",
        "Ok(Prepared { operations: Vec::new(), payload })",
    ),
    (
        "catalog_append_skips_write",
        "append",
        "rust/catalog_append.rs",
        "let result = log.append_to(device, prepared.operations, &prepared.payload);",
        "let result: Result<(), crate::catalog_log::AppendFailure<D::Error>> = Ok(());",
    ),
    (
        "catalog_append_swallows_write_error",
        "append",
        "rust/catalog_append.rs",
        "Err(error) => Err(Error::Append(error)),",
        "Err(_error) => Ok(()),",
    ),
    (
        "catalog_append_forwards_wrong_run",
        "append",
        "rust/catalog_append.rs",
        "prepare(resolver, run, event, &log.indexes().staging_edges)?",
        'prepare(resolver, "foreign", event, &log.indexes().staging_edges)?',
    ),
    (
        "catalog_append_forwards_wrong_event",
        "append",
        "rust/catalog_append.rs",
        "prepare(resolver, run, event, &log.indexes().staging_edges)?",
        "prepare(resolver, run, Value::Null, &log.indexes().staging_edges)?",
    ),
    (
        "catalog_append_refuses_valid_event",
        "prepare",
        "rust/catalog_append.rs",
        "    let Value::Object(mut object) = event else {",
        "    if true { return Err(Error::Decode(catalog_decode::Failure::Wire(crate::catalog_json::DecodeError::Object))); }\n    let Value::Object(mut object) = event else {",
    ),
    (
        "environment_json_serialization_loses_value",
        "prepare",
        JSON_SPECS,
        "ensures r.is_ok() ==> serializes(value, r->Ok_0@);",
        "ensures true;",
    ),
    (
        "environment_json_value_roundtrip_loses_meaning",
        "prepare",
        JSON_SPECS,
        "ensures parses(raw, value),",
        "ensures true,",
    ),
    (
        "environment_json_insert_loses_fields",
        "prepare",
        JSON_SPECS,
        "forall|name: Seq<char>| field_in(entries(*final(value)), name)",
        "forall|name: Seq<char>| true || field_in(entries(*final(value)), name)",
    ),
)
for _name, _function, *_ in CATALOG_APPEND_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "catalog_append"


CATALOG_EQUIVALENCE_MUTATIONS = (
    (
        "environment_json_parse_inconsistent",
        "catalog_json_equivalence",
        "parsed_wire_equal",
        JSON_SPECS,
        "ensures observe(left, depth) == observe(right, depth),",
        "ensures true,",
    ),
    (
        "environment_string_contents_distinct",
        "catalog_json_equivalence",
        "strings_equal",
        "verification/common/verus/environment.rs",
        "forall|left: String, right: String| left@ == right@ ==> left == right",
        "true",
    ),
    (
        "json_observation_ignores_numbers",
        "catalog_json_equivalence",
        "wire_equal",
        JSON_SPECS,
        "Json::Number(n) => Observation::Number(n)",
        "Json::Number(_n) => Observation::Number(None)",
    ),
    (
        "json_observation_ignores_array_order",
        "catalog_json_equivalence",
        "strings_equal",
        JSON_SPECS,
        "|i: int| observe(items[i], (depth - 1) as nat)",
        "|_i: int| observe(items[0], (depth - 1) as nat)",
    ),
    (
        "json_observation_ignores_object_keys",
        "catalog_json_equivalence",
        "owners_equal",
        JSON_SPECS,
        "(items[i].0@, observe(items[i].1, (depth - 1) as nat))",
        "(Seq::<char>::empty(), observe(items[i].1, (depth - 1) as nat))",
    ),
    (
        "json_observation_ignores_booleans",
        "catalog_json_equivalence",
        "packs_equal",
        JSON_SPECS,
        "Json::Boolean(b) => Observation::Boolean(b)",
        "Json::Boolean(_b) => Observation::Boolean(false)",
    ),
    (
        "catalog_replay_edges_dropped",
        "catalog_equivalence",
        "actual_run",
        "rust/catalog_state.rs",
        "Operation::Edges(k, children) => State { edges: s.edges.insert(k, children@), ..s }",
        "Operation::Edges(_k, _children) => s",
    ),
    (
        "catalog_replay_receipts_replaced",
        "catalog_equivalence",
        "actual_run",
        "rust/catalog_state.rs",
        "Operation::Receipts(ids) => State { receipts: s.receipts.union(ids@.to_set()), ..s }",
        "Operation::Receipts(ids) => State { receipts: ids@.to_set(), ..s }",
    ),
    (
        "catalog_replay_reclaim_clears_tombstone",
        "catalog_equivalence",
        "actual_run",
        "rust/catalog_state.rs",
        "State { pending: s.pending.remove(p), packs: s.packs.remove(p), ..s }",
        "State { pending: s.pending.remove(p), packs: s.packs.remove(p), deleted: s.deleted.remove(p), ..s }",
    ),
    (
        "catalog_equivalence_overlay_erases_old",
        "catalog_equivalence",
        "edges_fold",
        "verification/wal/verus/catalog_equivalence.rs",
        "Map::new(base.dom().union(added.dom()), |key: String|",
        "Map::new(added.dom(), |key: String|",
    ),
    (
        "catalog_equivalence_history_ignores_payload",
        "catalog_equivalence",
        "raw_history_state_equal",
        "verification/wal/verus/catalog_equivalence.rs",
        "crate::catalog_log::payloads(left) == crate::catalog_log::payloads(right),",
        "true,",
    ),
    (
        "catalog_equivalence_partial_state_authorized",
        "catalog_equivalence",
        "completed_states_equal",
        "verification/wal/verus/catalog_equivalence.rs",
        "crate::catalog_log::valid(left), crate::catalog_log::valid(right), left.complete, right.complete,",
        "crate::catalog_log::valid(left), crate::catalog_log::valid(right),",
    ),
)
for _name, _module, _function, *_ in CATALOG_EQUIVALENCE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_METADATA_MUTATIONS = (
    (
        "metadata_wrong_consumer",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        'path: vec!["consumers".to_owned(), identifier(event, "consumer_id")?]',
        'path: vec!["batches".to_owned(), identifier(event, "consumer_id")?]',
    ),
    (
        "metadata_wrong_batch_payload",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        'replacement: Some(crate::request::copy_field(event, "batch"))',
        'replacement: Some(crate::request::copy_field(event, "state"))',
    ),
    (
        "metadata_ready_stays_false",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        "replacement: Some(Value::Bool(true))",
        "replacement: Some(Value::Bool(false))",
    ),
    (
        "metadata_checkpoint_drops_payload",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        'replacement: Some(crate::request::copy_field(event, "checkpoint"))',
        "replacement: Some(Value::Null)",
    ),
    (
        "metadata_release_keeps_null",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        "replacement: None,",
        "replacement: Some(Value::Null),",
    ),
    (
        "metadata_retention_wrong_positions",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        'replacement: Some(crate::request::copy_field(event, "positions"))',
        'replacement: Some(crate::request::copy_field(event, "prefix"))',
    ),
    (
        "metadata_receipt_wrong_reply",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        "replacement: Some(Value::Array(vec![digest, response]))",
        "replacement: Some(Value::Array(vec![response, digest]))",
    ),
    (
        "metadata_refuses_valid_event",
        "queue_metadata",
        "prepare",
        "rust/queue_metadata.rs",
        'let mut edits = if kind == "ConsumerState" {',
        'if kind == "ConsumerState" { return Err(Error::Event); }\n    let mut edits = if kind == "ConsumerState" {',
    ),
    (
        "metadata_skips_business_update",
        "queue_metadata",
        "apply",
        "rust/queue_metadata.rs",
        "crate::json_update::update(state, &edit.path, edit.replacement).is_err()",
        "false",
    ),
    (
        "metadata_swallows_invalid_state",
        "queue_metadata",
        "apply",
        "rust/queue_metadata.rs",
        "return Err(Error::State);",
        "return Ok(());",
    ),
    (
        "json_update_wrong_leaf",
        "json_update",
        "update",
        "rust/json_update.rs",
        "map.insert(path[0].clone(), item);",
        'map.insert("wrong".to_owned(), item);',
    ),
    ("json_update_skips_remove", "json_update", "update", "rust/json_update.rs", "map.remove(&path[0]);", ""),
    (
        "json_update_accepts_empty_path",
        "json_update",
        "update",
        "rust/json_update.rs",
        "if path.is_empty() {\n        return Err(InvalidPath);",
        "if path.is_empty() {\n        return Ok(());",
    ),
    (
        "json_update_skips_nested_write",
        "json_update",
        "update",
        "rust/json_update.rs",
        "update(child, &path[1..], replacement)",
        "Ok(())",
    ),
    (
        "environment_json_mutation_loses_frame",
        "json_update",
        "update",
        "verification/common/verus/serde_json.rs",
        "Some(value) => borrowed_key_mutated(object(*old(map)), object(*final(map)), key, *value, *final(value))",
        "Some(value) => true",
    ),
    (
        "environment_json_readonly_borrow_mutates",
        "json_update",
        "update",
        "verification/common/verus/serde_json.rs",
        "&& (*final(value) == *value ==> *final(map) == *old(map))",
        "&& true",
    ),
    (
        "environment_json_remove_keeps_key",
        "json_update",
        "update",
        "verification/common/verus/serde_json.rs",
        "ensures borrowed_key_removed(object(*old(map)), object(*final(map)), key),",
        "ensures true,",
    ),
)
for _name, _module, _function, *_ in QUEUE_METADATA_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


JOURNAL_APPEND_MUTATIONS = (
    (
        "journal_encode_accepts_invalid_events",
        "journal_append",
        "encode",
        "rust/journal_append.rs",
        "if !crate::journal_recovery::valid_events(events) {",
        "if false {",
    ),
    (
        "journal_encode_refuses_valid_events",
        "journal_append",
        "encode",
        "rust/journal_append.rs",
        "if !crate::journal_recovery::valid_events(events) {",
        "if true {",
    ),
    (
        "journal_encode_serializes_null",
        "journal_append",
        "encode",
        "rust/journal_append.rs",
        "serde_json::to_vec(events)",
        "serde_json::to_vec(&Value::Null)",
    ),
    (
        "journal_append_discards_payload",
        "journal_append",
        "append",
        "rust/journal_append.rs",
        "log.append_to(writer, &payload)",
        "log.append_to(writer, &[])",
    ),
    (
        "journal_append_wrong_receipt_sequence",
        "journal_append",
        "append",
        "rust/journal_append.rs",
        "Ok(sequence)\n}",
        "Ok(0)\n}",
    ),
    (
        "journal_append_skips_poison_guard",
        "journal_append",
        "append",
        "rust/journal_append.rs",
        "if log.poisoned || log.read_only {",
        "if log.read_only {",
    ),
    (
        "journal_append_swallows_write_failure",
        "journal_append",
        "append",
        "rust/journal_append.rs",
        "Err(error) => return Err(Error::Write(error)),",
        "Err(error) => return Ok(log.sequence),",
    ),
    (
        "journal_frame_ignores_source_event",
        "journal_equivalence",
        "recovered_frame_observations",
        "rust/journal_append.rs",
        "exists|raw: Seq<u8>| encoded(events, raw)",
        "exists|raw: Seq<u8>| raw.len() <= usize::MAX",
    ),
    (
        "journal_history_ignores_encoding",
        "journal_equivalence",
        "encoded_history_observations",
        "verification/wal/verus/journal_equivalence.rs",
        "==> crate::journal_append::encoded(original[i], raw[i])",
        "==> crate::journal_recovery::events(original[i])",
    ),
    (
        "journal_recovery_ignores_payload",
        "journal_equivalence",
        "repeated_recovery_observations",
        "rust/journal_recovery.rs",
        "==> crate::json_specs::parses(raw[i], transactions[i]) && events(transactions[i])",
        "==> events(transactions[i])",
    ),
    (
        "environment_journal_parse_observations_change",
        "journal_equivalence",
        "encoded_history_observations",
        "verification/common/verus/serde_json.rs",
        "ensures observe(left, depth) == observe(right, depth),",
        "ensures true,",
    ),
    (
        "environment_journal_encoding_unbound",
        "journal_append",
        "encode",
        "verification/common/verus/serde_json.rs",
        "ensures parses(raw, value),",
        "ensures true,",
    ),
)
for _name, _module, _function, *_ in JOURNAL_APPEND_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CROSS_LOG_MUTATIONS = (
    (
        "cross_accept_without_retention",
        "queue_step",
        "verification/protocol/verus/cross_log.rs",
        "&& s.recipes[stage].subset_of(owned(s, key.queue))",
        "&& true",
    ),
    (
        "cross_accept_reuses_id",
        "step_safe",
        "verification/protocol/verus/cross_log.rs",
        "&& !s.accepted.contains_key(key) && s.recipes.contains_key(stage)",
        "&& true && s.recipes.contains_key(stage)",
    ),
    (
        "cross_prune_before_retirement",
        "queue_step",
        "verification/protocol/verus/cross_log.rs",
        "s.ready.contains(queue) && keeps(s, queue, keep)",
        "s.ready.contains(queue)",
    ),
    (
        "cross_prune_peer_queue",
        "queue_step",
        "verification/protocol/verus/cross_log.rs",
        "queues: s.queues.insert(queue, owned(s, queue).intersect(keep))",
        "queues: s.queues.insert(queue + 1, owned(s, queue + 1).intersect(keep))",
    ),
    (
        "cross_mark_live_pack",
        "protection_step",
        "verification/protocol/verus/cross_log.rs",
        "s.sealed.contains(p) && !protected(s, p)",
        "s.sealed.contains(p)",
    ),
    (
        "cross_receipt_before_barrier",
        "step_safe",
        "verification/protocol/verus/cross_log.rs",
        "s.dead.contains(p) && !s.durable_names.contains(p)",
        "s.dead.contains(p)",
    ),
    (
        "cross_receipt_forgets_fence",
        "step_preserves_fences",
        "verification/protocol/verus/cross_log.rs",
        "State { reclaimed: s.reclaimed.insert(p), ..s }",
        "State { reclaimed: s.reclaimed.insert(p), dead: s.dead.remove(p), ..s }",
    ),
    (
        "cross_cleanup_consumes_restage",
        "old_handoff_preserves_restage",
        "verification/protocol/verus/cross_log.rs",
        "if s.consumed.contains(key) { s } else {",
        "if false { s } else {",
    ),
    (
        "cross_recovery_forgets_cache",
        "cache_step",
        "verification/protocol/verus/cross_log.rs",
        "cached: recover_cache(s, q)",
        "cached: s.cached",
    ),
    (
        "cross_conflict_returns_cached",
        "cached_retry_after_gc",
        "verification/protocol/verus/cross_log.rs",
        "else if s.cached[k].digest == digest",
        "else if true",
    ),
    (
        "cross_retention_drops_dependency",
        "retained_closure_is_complete",
        "verification/protocol/verus/cross_log.rs",
        "owned(s, queue).union(s.recipes[stage])",
        "owned(s, queue).union(Set::empty())",
    ),
    (
        "cross_recovery_loses_acceptance",
        "step_preserves_receipt",
        "verification/protocol/verus/cross_log.rs",
        "State { ready: s.ready.insert(q), cached: recover_cache(s, q), ..s }",
        "State { ready: s.ready.insert(q), cached: recover_cache(s, q), accepted: Map::empty(), ..s }",
    ),
    (
        "cross_unlink_without_mark",
        "protection_step",
        "verification/protocol/verus/cross_log.rs",
        "Action::Unlink(p) => s.dead.contains(p)",
        "Action::Unlink(p) => true",
    ),
    (
        "environment_composition_restores_away_live_names",
        "protection_step",
        "verification/common/verus/environment.rs",
        "current.subset_of(restored) && restored.subset_of(durable)",
        "restored.subset_of(durable)",
    ),
)
for _name, _function, *_ in CROSS_LOG_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = "cross_log"


QUEUE_ROOT_MUTATIONS = (
    (
        "roots_omit_producers",
        "queue_roots",
        "collect",
        "rust/queue_roots.rs",
        '    if let Err(error) = references(digester, field_value(state, "producers"), &mut roots) {\n        return Err(Error::Digest(error));\n    }',
        "",
    ),
    (
        "roots_omit_checkpoints",
        "queue_roots",
        "collect",
        "rust/queue_roots.rs",
        '    if let Err(error) = references(digester, field_value(state, "checkpoints"), &mut roots) {\n        return Err(Error::Digest(error));\n    }',
        "",
    ),
    (
        "roots_skip_prefix_commit",
        "queue_roots",
        "include",
        "rust/queue_roots.rs",
        "position >= prefix && !positions.contains(&position)",
        "position > prefix && !positions.contains(&position)",
    ),
    (
        "roots_include_processed_commit",
        "queue_roots",
        "include",
        "rust/queue_roots.rs",
        "position >= prefix && !positions.contains(&position)",
        "position >= prefix",
    ),
    (
        "roots_use_wrong_task_field",
        "queue_roots",
        "selected_input",
        "rust/queue_roots.rs",
        'Selection::Task => field_value(field_value(value, "spec"), "input_ref"),',
        'Selection::Task => field_value(field_value(value, "spec"), "result_ref"),',
    ),
    (
        "roots_drop_unfinished_batches",
        "queue_roots",
        "include",
        "rust/queue_roots.rs",
        'Selection::Batch(finished) => !contains(finished, field_value(value, "batch_id")),',
        'Selection::Batch(finished) => contains(finished, field_value(value, "batch_id")),',
    ),
)
for _name, _module, _function, *_ in QUEUE_ROOT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_GC_MUTATIONS = (
    (
        "queue_gc_retire_ignores_error",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "self.execute(roots.prefix, &roots.positions)?;",
        "let _ = self.execute(roots.prefix, &roots.positions);",
    ),
    (
        "queue_gc_owner_omits_prefix",
        "queue_gc",
        "owners",
        "rust/queue_gc.rs",
        "let mut owner = prefix.to_owned();",
        "let mut owner = String::new();",
    ),
    (
        "queue_gc_prunes_peer_owner",
        "queue_gc",
        "prunable",
        "rust/queue_gc.rs",
        "owner.as_str().as_bytes().starts_with(prefix.as_bytes()) && !live.contains(owner)",
        "!live.contains(owner)",
    ),
    (
        "queue_gc_ignores_tombstoned_path",
        "queue_gc",
        "require",
        "rust/queue_gc.rs",
        "paths.is_empty() || crate::closure::reclaimed(paths, deleted)",
        "paths.is_empty()",
    ),
    (
        "queue_gc_release_wrong_owner",
        "queue_gc",
        "release_value",
        "rust/queue_gc.rs",
        "updates.insert(names[i].clone(), Value::Null);",
        'updates.insert("wrong-owner".to_owned(), Value::Null);',
    ),
    (
        "queue_gc_release_ignores_append_error",
        "queue_gc",
        "release",
        "rust/queue_gc.rs",
        "crate::catalog_append::append(log, resolver, run, value, device)?;",
        "let _ = crate::catalog_append::append(log, resolver, run, value, device);",
    ),
    (
        "queue_gc_ignores_missing_owner",
        "gc_collection",
        "run_queue",
        "rust/gc_collection.rs",
        "crate::queue_gc::require(\n"
        "            &self.log.indexes().owners,\n"
        "            &self.log.indexes().deleted,\n"
        "            &live,\n"
        "        )",
        "Ok::<(),crate::queue_gc::OwnershipError>(())",
    ),
    (
        "queue_gc_prunes_before_check",
        "gc_collection",
        "run_queue",
        "rust/gc_collection.rs",
        "let (prefix, live) = prepared.parts();",
        "let (prefix, live) = prepared.parts();\n"
        "        let premature=crate::queue_gc::prunable(&self.log.indexes().owners,prefix,&live);\n"
        "        let _=crate::queue_gc::release(self.log,self.resolver,self.run,&premature,self.io.device);",
    ),
    (
        "queue_collection_prunes_peer_prefix",
        "gc_collection",
        "run_queue",
        "rust/gc_collection.rs",
        "let dead = crate::queue_gc::prunable(&self.log.indexes().owners, prefix, &live);",
        'let dead = crate::queue_gc::prunable(&self.log.indexes().owners, "", &live);',
    ),
    (
        "queue_collection_skips_release",
        "gc_collection",
        "run_queue",
        "rust/gc_collection.rs",
        "crate::queue_gc::release(self.log, self.resolver, self.run, &dead, self.io.device)",
        "Ok::<(),crate::catalog_append::Error<R::Error,D::Error>>(())",
    ),
    (
        "queue_collection_releases_publication_owner",
        "gc_collection",
        "run_queue",
        "rust/gc_collection.rs",
        "if !crate::queue_gc::not_publication_owners(&dead) {",
        "if false {",
    ),
)

for _name, _module, _function, *_ in QUEUE_GC_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


NAMESPACE_MUTATIONS = (
    (
        "identity_accepts_backslash",
        "namespace_identity",
        "advance",
        "rust/namespace_identity.rs",
        "if byte == b'\\\\'",
        "if byte == b'x'",
    ),
    (
        "target_appends_parent_component",
        "namespace_target",
        "restore",
        "rust/namespace_target.rs",
        "PartKind::Parent => {\n                source.pop(&mut resolved);\n            }",
        "PartKind::Parent => {\n                source.push(&mut resolved, part);\n            }",
    ),
    (
        "catalog_legacy_bypasses_namespace",
        "catalog_namespace",
        "resolve",
        "rust/catalog_namespace.rs",
        "Encoding::Legacy => source.legacy(cache, name),",
        "Encoding::Legacy => Ok(name.clone()),",
    ),
)
NAMESPACE_MUTATIONS += (
    (
        "namespace_step_ignores_relative_base",
        "namespace_step",
        "step",
        "rust/namespace_step.rs",
        "let path = if self.primitives.is_absolute(&target) {\n                    target\n                } else {\n                    self.primitives.join(&position, target)\n                };",
        "let path = target;",
    ),
    (
        "namespace_step_swallows_readlink_error",
        "namespace_step",
        "step",
        "rust/namespace_step.rs",
        "if matches!(\n                    self.primitives.classify(&error),\n                    crate::namespace_target::ErrorKind::Missing\n                        | crate::namespace_target::ErrorKind::NotLink\n                )",
        "if true",
    ),
    (
        "namespace_step_returns_original_name",
        "namespace_step",
        "step",
        "rust/namespace_step.rs",
        "let identity = self.primitives.relative(&resolved, prefix)?;",
        "let identity = prefix.clone();",
    ),
    (
        "environment_namespace_target_context_unbound",
        "namespace_step",
        "step",
        "rust/namespace_target.rs",
        "ensures final(source)@ == old(source)@, final(source).context() == old(source).context(),",
        "ensures final(source)@ == old(source)@,",
    ),
)
for _name, _module, _function, *_ in NAMESPACE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CATALOG_VALID_MUTATIONS = (
    (
        "catalog_adapter_bypasses_namespace",
        "catalog_namespace",
        "*resolve*",
        "rust/catalog_namespace.rs",
        "resolve(&mut self.source, &mut self.cache, name, encoding)",
        "Ok(name.clone())",
    ),
    (
        "catalog_target_forgets_resolver_validity",
        "catalog_target",
        "apply",
        "rust/catalog_target.rs",
        "self.decoder.valid() && crate::catalog_log::valid(self.log@)",
        "crate::catalog_log::valid(self.log@)",
    ),
)
CATALOG_VALID_MUTATIONS += (
    (
        "catalog_source_uses_other_name",
        "catalog_namespace",
        "*identity*",
        "rust/catalog_namespace.rs",
        "let mut parts = self.factory.parts(name);",
        'let mut parts = self.factory.parts("wrong");',
    ),
    (
        "catalog_source_bypasses_legacy",
        "catalog_namespace",
        "PathSource::legacy",
        "rust/catalog_namespace.rs",
        "let result = self.identity(cache, name.as_str());",
        "let result: Result<String, Self::Error> = Ok(name.clone());",
    ),
    (
        "catalog_namespace_starts_poisoned",
        "catalog_namespace",
        "*new*",
        "rust/catalog_namespace.rs",
        "        Self {\n            source,\n            cache: HashMap::new(),\n        }",
        '        let mut cache = HashMap::new();\n        cache.insert("unbound".to_owned(), "wrong".to_owned());\n        Self { source, cache }',
    ),
)
for _name, _module, _function, *_ in CATALOG_VALID_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


WRITER_RETIREMENT_MUTATIONS = (
    (
        "writer_skip_fence",
        "writer_retirement",
        "execute",
        "rust/writer_retirement.rs",
        "    effects.fence()?;",
        "    // fence omitted",
    ),
    (
        "writer_release_before_handle",
        "writer_registry",
        "reconcile_registered",
        "rust/writer_registry.rs",
        "if let Err(error) = recover_operation(io, &operations[index], recover, receipts) {",
        "if let Err(error) = release_operation(io, &operations[index]) {",
    ),
    (
        "writer_allow_result_rewrite",
        "writer_retirement",
        "remember_results",
        "rust/writer_retirement.rs",
        "if operation.done || !matches {",
        "if operation.done {",
    ),
    (
        "writer_allow_duplicate_sequence",
        "writer_retirement",
        "register_operation",
        "rust/writer_retirement.rs",
        "operations[operations.len() - 1].sequence >= sequence",
        "operations[operations.len() - 1].sequence > sequence",
    ),
    (
        "writer_wrong_operation_lookup",
        "writer_retirement",
        "operation_position",
        "rust/writer_retirement.rs",
        "if operations[index].sequence == sequence {",
        "if operations[index].sequence != sequence {",
    ),
    (
        "writer_settlement_drops_result",
        "writer_retirement",
        "settle_operation",
        "rust/writer_retirement.rs",
        "    operation.done = true;",
        "    operation.results = None;\n    operation.done = true;",
    ),
    (
        "writer_settle_unrelated_owner",
        "writer_retirement",
        "settle_named",
        "rust/writer_retirement.rs",
        "if named_operation(&operations[index].evidence, name, by_owner) {",
        "if true {",
    ),
    (
        "writer_registry_rejects_valid_event",
        "writer_registry",
        "decode",
        "rust/writer_registry.rs",
        '    let kind = string(value, "type")?;',
        '    return None;\n    let kind = string(value, "type")?;',
    ),
    (
        "writer_registry_wrong_sequence_field",
        "writer_registry",
        "decode",
        "rust/writer_registry.rs",
        'let sequence = integer(value, "sequence")?;',
        'let sequence = integer(value, "operation")?;',
    ),
    (
        "writer_registry_wrong_result_field",
        "writer_registry",
        "decode",
        "rust/writer_registry.rs",
        'let roots = values(value, "results")?;',
        'let roots = values(value, "dependencies")?;',
    ),
    (
        "writer_registry_repeats_first_event",
        "writer_registry",
        "decode_frame",
        "rust/writer_registry.rs",
        "match decode(&values[index])",
        "match decode(&values[0])",
    ),
    (
        "writer_registry_dispatches_wrong_owner_selector",
        "writer_registry",
        "apply_event",
        "rust/writer_registry.rs",
        "crate::writer_retirement::settle_named(operations, owner, true);",
        "crate::writer_retirement::settle_named(operations, owner, false);",
    ),
    (
        "writer_registry_applies_first_event_twice",
        "writer_registry",
        "apply_frame",
        "rust/writer_registry.rs",
        "apply_event(operations, receipts, frame, &events[index])",
        "apply_event(operations, receipts, frame, &events[0])",
    ),
    (
        "writer_registry_replays_wrong_frame",
        "writer_registry",
        "replay",
        "rust/writer_registry.rs",
        "apply_frame(operations, receipts, frame as u64, &frames[frame])",
        "apply_frame(operations, receipts, frame as u64, &frames[0])",
    ),
    (
        "writer_receipt_hides_durable_done",
        "writer_registry",
        "receipt",
        "rust/writer_registry.rs",
        "if operations[index].evidence.key.as_str() == key {",
        "if operations[index].evidence.key.as_str() == key && !operations[index].done {",
    ),
    (
        "writer_reconcile_releases_wrong_owner",
        "writer_registry",
        "release_operation",
        "rust/writer_registry.rs",
        "io.release(owner)?;",
        "io.release(&operation.evidence.key)?;",
    ),
    (
        "writer_reconcile_ignores_failed_stage",
        "writer_registry",
        "recover_operation",
        "rust/writer_registry.rs",
        "io.stage(&results.roots)?;",
        "let _ = io.stage(&results.roots);",
    ),
    (
        "writer_reconcile_settles_wrong_id",
        "writer_registry",
        "reconcile_registered",
        "rust/writer_registry.rs",
        "ids.push(operations[index].sequence);",
        "ids.push(0);",
    ),
    (
        "writer_gate_omits_lock",
        "writer_gate_flow",
        "acquire",
        "rust/writer_gate_flow.rs",
        "    if let Err(error) = primitives.lock(&file, nonblocking) {\n"
        "        return Err(Failure::Io(error));\n"
        "    }",
        "    // Missing exclusive acquisition",
    ),
    (
        "writer_gate_ignores_header",
        "writer_gate_flow",
        "acquire",
        "rust/writer_gate_flow.rs",
        "if raw.as_slice() != header {",
        "if false {",
    ),
    (
        "writer_gate_ignores_fence",
        "writer_gate_flow",
        "acquire",
        "rust/writer_gate_flow.rs",
        "if !allow_fenced {",
        "if false {",
    ),
    (
        "writer_gate_ignores_named_inode",
        "writer_gate_flow",
        "bound",
        "rust/writer_gate_flow.rs",
        "if held != self.expected || named != self.expected {",
        "if held != self.expected {",
    ),
    (
        "writer_marker_skips_file_sync",
        "writer_gate_flow",
        "persist_marker",
        "rust/writer_gate_flow.rs",
        "    io.sync_file(&file, marker)?;",
        "    // Missing marker file barrier",
    ),
    (
        "writer_marker_skips_directory_sync",
        "writer_gate_flow",
        "persist_marker",
        "rust/writer_gate_flow.rs",
        "    io.sync_directory(marker)",
        "    Ok(())",
    ),
    (
        "writer_drain_unlinks_unrelated_name",
        "writer_gate_flow",
        "drain",
        "rust/writer_gate_flow.rs",
        "            PayloadKind::Other => {}",
        "            PayloadKind::Other => { let _ = io.unlink(&entries[index].path); }",
    ),
    (
        "writer_drain_skips_directory_sync",
        "writer_gate_flow",
        "drain",
        "rust/writer_gate_flow.rs",
        "    io.sync_directory()",
        "    Ok(())",
    ),
    (
        "writer_owner_accepts_unregistered",
        "writer_ownership",
        "validate_owners",
        "rust/writer_ownership.rs",
        "if !registered_owner(operations, &held.owner) {",
        "if false {",
    ),
    (
        "writer_owner_accepts_uncovered_path",
        "writer_ownership",
        "validate_owners",
        "rust/writer_ownership.rs",
        "if !certified_owner(io, operations, &held.owner, &held.paths) {",
        "if false {",
    ),
    (
        "writer_seals_wrong_parent",
        "writer_ownership",
        "seal_candidates",
        "rust/writer_ownership.rs",
        "if io.same_parent(&parent, writer) {",
        "if !io.same_parent(&parent, writer) {",
    ),
    (
        "writer_seal_omits_open_check",
        "writer_ownership",
        "seal_candidates",
        "rust/writer_ownership.rs",
        "if !packs[index].1 {",
        "if true {",
    ),
    (
        "writer_auth_uses_wrong_root",
        "writer_ownership",
        "authenticate_all",
        "rust/writer_ownership.rs",
        "let root = roots[index].clone();",
        "let root = roots[0].clone();",
    ),
    (
        "writer_auth_drops_transitive_nodes",
        "writer_ownership",
        "authenticate_all",
        "rust/writer_ownership.rs",
        "authenticated.push(visited);",
        "authenticated.push(std::collections::BTreeSet::new());",
    ),
)

for _name, _module, _function, *_ in WRITER_RETIREMENT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


GC_COLLECTION_MUTATIONS = (
    (
        "gc_collection_wrong_snapshot",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "self.targets = targets;",
        "self.targets = Vec::new();",
    ),
    (
        "gc_collection_resolves_first_target",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "self.io.physical.resolve(&self.targets[index])",
        "self.io.physical.resolve(&self.targets[0])",
    ),
    (
        "gc_collection_unlinks_wrong_target",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "self.io.physical.remove(&self.targets[index], path)",
        'self.io.physical.remove(&"other".to_owned(), path)',
    ),
    (
        "gc_collection_receipt_wrong_run",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "            self.run,\n            &self.targets,",
        '            "wrong-run",\n            &self.targets,',
    ),
    (
        "gc_collection_receipt_empty_targets",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "            &self.targets,\n            self.io.device,",
        "            &Vec::new(),\n            self.io.device,",
    ),
    (
        "gc_collection_skip_repair_barriers",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "let outcome = crate::directory_io::sync_all(&directories, &mut self.io);",
        "let outcome: Result<(), RepairError<P::Error, D::Error>> = Ok(());",
    ),
    (
        "gc_collection_skip_parent_sync",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "let outcome = self.io.physical.sync_parent(directory);",
        "let outcome: Result<(), P::Error> = Ok(());",
    ),
    (
        "gc_collection_ignore_receipt_error",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "Err(error) => Err(Error::Receipt(error)),",
        "Err(_error) => Ok(()),",
    ),
    (
        "gc_collection_receipt_before_unlink",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "let outcome = gc_batch::collect(self);",
        "let _ = gc_batch::Effects::receipt(self);\n        let outcome = gc_batch::collect(self);",
    ),
    (
        "gc_collection_mark_wrong_run",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "crate::gc_mark::mark(self.log, self.resolver, self.run, self.io.device)",
        'crate::gc_mark::mark(self.log, self.resolver, "wrong-run", self.io.device)',
    ),
    (
        "gc_collection_omits_mark",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "crate::gc_mark::mark(self.log, self.resolver, self.run, self.io.device)",
        "Ok::<_,crate::catalog_append::Error<R::Error,D::Error>>(crate::gc_mark::Marked{live:self.log.indexes().roots(),targets:Vec::new()})",
    ),
    (
        "environment_gc_named_resolve_lies",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "ensures result.is_ok() ==> result->Ok_0 == (self@.paths)(*name)",
        "ensures true",
    ),
    (
        "environment_gc_named_sync_ancestor_lies",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "result.is_ok() ==> final(self)@.synced.contains(*parent),",
        "true,",
    ),
)
for _name, _module, _function, *_ in GC_COLLECTION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


NATIVE_FILE_MUTATIONS = (
    (
        "native_file_constructor_unbound_permission",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "requires permission@@.identity == raw::identity(*file),",
        "requires true,",
    ),
    (
        "native_file_seek_wrong_offset",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "            offset,\n            #[cfg(straw_verus)]",
        "            0,\n            #[cfg(straw_verus)]",
    ),
    (
        "native_file_truncate_wrong_end",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "            end,\n            #[cfg(straw_verus)]",
        "            0,\n            #[cfg(straw_verus)]",
    ),
    (
        "native_file_read_skips_io",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "raw::read(\n            self.file,\n            &mut bytes,\n            #[cfg(straw_verus)]\n            self.permission,\n        )",
        "Ok::<(),std::io::Error>(())",
    ),
    (
        "native_file_skips_sync",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "raw::sync(\n            self.file,\n            #[cfg(straw_verus)]\n            self.permission,\n        )",
        "Ok::<(),std::io::Error>(())",
    ),
    (
        "native_file_writes_empty",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "raw::write(\n            self.file,\n            bytes,",
        "raw::write(\n            self.file,\n            &[],",
    ),
    ("native_file_hash_omits_first", "file_adapter", "*", "rust/file_adapter.rs", "        digest.update(first);", ""),
    (
        "native_file_read_masks_error",
        "file_adapter",
        "*",
        "rust/file_adapter.rs",
        "    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {\n        let mut bytes = vec![0; length];\n        match raw::read(\n            self.file,\n            &mut bytes,\n            #[cfg(straw_verus)]\n            self.permission,\n        ) {\n            Ok(()) => Ok(bytes),\n            Err(error) => Err(L::io(error)),\n        }\n    }\n",
        "    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error> {\n        let mut bytes = vec![0; length];\n        match raw::read(\n            self.file,\n            &mut bytes,\n            #[cfg(straw_verus)]\n            self.permission,\n        ) {\n            Ok(()) => Ok(bytes),\n            Err(_error) => Ok(bytes),\n        }\n    }\n",
    ),
    (
        "environment_native_file_length_lies",
        "file_adapter",
        "*",
        "rust/file_primitives.rs",
        "ensures result.is_ok() ==> result->Ok_0 == permission@@.bytes.len(),",
        "ensures true,",
    ),
    (
        "environment_native_file_sync_lies",
        "file_adapter",
        "*",
        "rust/file_primitives.rs",
        "&& final(permission)@@.durable == final(permission)@@.bytes.len(),",
        "&& true,",
    ),
    (
        "file_source_accepts_invalid_file",
        "wal_source",
        "from_file",
        "rust/wal_source.rs",
        "        requires file.valid(),",
        "        requires true,",
    ),
)
for _name, _module, _function, *_ in NATIVE_FILE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_RETIREMENT_MUTATIONS = (
    (
        "retirement_wrong_prefix",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "Value::Number(prefix.into())",
        "Value::Number(0u64.into())",
    ),
    (
        "retirement_omits_sparse_position",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "values.push(Value::Number((*number).into()));",
        "if *number == 0 { values.push(Value::Number((*number).into())); }",
    ),
    (
        "retirement_skips_wal_append",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "crate::journal_append::append(self.journal, self.device, &events)",
        "Ok::<u64,crate::journal_append::Error<D::Error>>(0)",
    ),
    (
        "retirement_skips_application",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "crate::queue_metadata::apply(self.business, event)",
        "Ok::<(),crate::queue_metadata::Error>(())",
    ),
    (
        "retirement_skips_quarantine",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "crate::queue_application::arm(self.journal);",
        "",
    ),
    (
        "retirement_clears_quarantine_before_apply",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "crate::queue_application::arm(self.journal);",
        "crate::queue_application::arm(self.journal); self.journal.poisoned = false;",
    ),
    (
        "retirement_ignores_after_apply_error",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "return Err(Error::After(error));",
        "let _ = error;",
    ),
    (
        "retirement_accepts_unbound_snapshot",
        "queue_retirement",
        "*",
        "rust/queue_retirement.rs",
        "requires old(self).resource_valid(), roots.selected_for(old(self).queue()),",
        "requires old(self).resource_valid(),",
    ),
    (
        "gc_prepare_wrong_queue",
        "queue_gc_prepare",
        "*",
        "rust/queue_gc_prepare.rs",
        "crate::queue_roots::collect(digester, business, progress)",
        "crate::queue_roots::collect(digester, progress, progress)",
    ),
    (
        "gc_prepare_wrong_progress",
        "queue_gc_prepare",
        "*",
        "rust/queue_gc_prepare.rs",
        "crate::queue_roots::collect(digester, business, progress)",
        "crate::queue_roots::collect(digester, business, business)",
    ),
    (
        "gc_prepare_wrong_owner_prefix",
        "queue_gc_prepare",
        "*",
        "rust/queue_gc_prepare.rs",
        "crate::queue_gc::prepare(prefix, retired)",
        'crate::queue_gc::prepare("", retired)',
    ),
)

for _name, _module, _function, *_ in QUEUE_RETIREMENT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


JOURNAL_IO_MUTATIONS = (
    (
        "journal_io_recover_wrong_state",
        "journal_io",
        "recover",
        "rust/journal_io.rs",
        "let result = state.recover(&mut source);",
        "let mut replacement = State::new(false);\n    let result = replacement.recover(&mut source);",
    ),
    (
        "journal_io_append_wrong_state",
        "journal_io",
        "append",
        "rust/journal_io.rs",
        "let result = crate::journal_append::append(state, &mut writer, events);",
        "let mut replacement = State::new(false);\n"
        "    let result = crate::journal_append::append(&mut replacement, &mut writer, events);",
    ),
    (
        "journal_io_retire_wrong_state",
        "journal_io",
        "retire",
        "rust/journal_io.rs",
        "let result = crate::queue_retirement::commit(state, business, &mut writer, roots);",
        "let mut replacement = State::new(false);\n"
        "    let result = crate::queue_retirement::commit(&mut replacement, business, &mut writer, roots);",
    ),
    (
        "journal_io_recover_masks_failure",
        "journal_io",
        "recover",
        "rust/journal_io.rs",
        "source.finish();\n    result",
        "source.finish();\n    Ok(Boundary { sequence: 0, end: 0, incomplete: 0 })",
    ),
)
for _name, _module, _function, *_ in JOURNAL_IO_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CATALOG_RUNTIME_MUTATIONS = (
    (
        "catalog_runtime_install_keeps_old_permission",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "self.catalog.permission = fresh.opened.permission;",
        "",
    ),
    (
        "catalog_runtime_refresh_ignores_replay_error",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "Err(error) => Err(L::replay_error(error)),",
        "Err(_) => Ok(()),",
    ),
    (
        "catalog_runtime_context_ignores_ready",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "if !self.catalog.transaction_active || !self.catalog.log.ready() {",
        "if !self.catalog.transaction_active {",
    ),
    (
        "catalog_runtime_append_foreign_run",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "            &self.pool_run,\n            event,\n            &mut writer,",
        '            "foreign",\n            event,\n            &mut writer,',
    ),
    (
        "catalog_runtime_discards_writer_loan",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "        let result = crate::catalog_append::append(\n            &mut self.log,\n            &mut self.namespace,\n            &self.pool_run,\n            event,\n            &mut writer,\n        );\n        writer.finish();",
        "        let result = crate::catalog_append::append(\n            &mut self.log,\n            &mut self.namespace,\n            &self.pool_run,\n            event,\n            &mut writer,\n        );\n",
    ),
    (
        "environment_catalog_callback_changes_namespace",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "ensures final(catalog).valid(), final(catalog).fixed() == old(catalog).fixed(),\n"
        "            final(catalog).active(),",
        "ensures final(catalog).valid(),\n            final(catalog).active(),",
    ),
)
for _name, _module, _function, *_ in CATALOG_RUNTIME_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_PROGRESS_MUTATIONS = (
    (
        "progress_cache_wrong_reference",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "if *cached_ref == *reference {",
        "if true {",
    ),
    (
        "progress_cache_returns_reference",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "let result = cached_value.clone();",
        "let result = cached_ref.clone();",
    ),
    (
        "progress_ignores_member_count",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "if members.len() != 1 {",
        "if members.len() == 0 {",
    ),
    (
        "progress_reads_wrong_member",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "reader.read(&members[0])",
        "reader.read(reference)",
    ),
    (
        "progress_ignores_codec",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        'if get(&data.metadata, "codec").as_str() != Some("json.v1") {',
        "if false {",
    ),
    (
        "progress_ignores_version",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        'if get(value, "version").as_u64() != Some(1) {',
        "if false {",
    ),
    (
        "progress_ignores_numeric_type",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "if !numeric_values(positions) {",
        "if false {",
    ),
    (
        "progress_ignores_duplicates",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        "if !unique_numbers(positions) || !unique_texts(batches) {",
        "if false {",
    ),
    (
        "progress_selects_wrong_consumer",
        "queue_progress",
        "*",
        "rust/queue_progress.rs",
        'get(get(get(queue, "consumers"), "training"), "progress_ref")',
        'get(get(get(queue, "consumers"), "validation"), "progress_ref")',
    ),
    (
        "gc_prepare_progress_wrong_queue",
        "queue_gc_prepare",
        "*",
        "rust/queue_gc_prepare.rs",
        "crate::queue_progress::training(cache, &mut hooks, business)",
        "crate::queue_progress::training(cache, &mut hooks, &Value::Null)",
    ),
)
for _name, _module, _function, *_ in QUEUE_PROGRESS_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


REFERENCE_VALIDATION_MUTATIONS = (
    (
        "reference_validation_drops_dependency",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "while dependencies.len() > 0 {",
        "while dependencies.len() > 1 {",
    ),
    (
        "reference_validation_overwrites_root_members",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "if is_root {\n            root_records = records;\n        }",
        "if !is_root {\n            root_records = records;\n        }",
    ),
    (
        "reference_validation_scopes_dependencies",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "if is_root { task } else { None },",
        "task,",
    ),
    (
        "reference_validation_tags_dependency_as_root",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "crate::traversal::schedule(&mut frontier, child, (dependency, false));",
        "crate::traversal::schedule(&mut frontier, child, (dependency, true));",
    ),
    (
        "reference_validation_ignores_node_error",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "Err(e) => return Err(Error::Read(e)),",
        "Err(_) => (Vec::new(), Vec::new()),",
    ),
    (
        "reference_validation_skips_root",
        "reference_validation",
        "*",
        "rust/reference_validation.rs",
        "crate::traversal::schedule(&mut frontier, root_key, (reference.clone(), true));",
        "",
    ),
)
for _name, _module, _function, *_ in REFERENCE_VALIDATION_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


JOURNAL_OWNER_MUTATIONS = (
    (
        "journal_owner_bypass_mode",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "if read_only && !recover {",
        "if false {",
    ),
    (
        "journal_owner_wrong_open_mode",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "io.open(&path, recover, read_only)",
        "io.open(&path, true, read_only)",
    ),
    (
        "journal_owner_wrong_open_access",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "io.open(&path, recover, read_only)",
        "io.open(&path, recover, false)",
    ),
    (
        "journal_owner_skip_ancestor_barriers",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "        if let Err(error) = io.mkdir(&path) {\n            return Err(Error::Io(error));\n        }",
        "",
    ),
    (
        "journal_owner_skip_file_barrier",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "        if let Err(error) = io.sync_new(&mut opened) {\n"
        "            return Err(Error::Io(error));\n"
        "        }",
        "",
    ),
    (
        "journal_owner_skip_parent_barrier",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "        if let Err(error) = io.sync_parent(&path, &mut opened) {\n"
        "            return Err(Error::Io(error));\n"
        "        }",
        "",
    ),
    (
        "journal_owner_wrong_identity",
        "journal_owner",
        "header",
        "rust/journal_owner.rs",
        'object.insert("identity".to_owned(), identity.clone());',
        'object.insert("identity".to_owned(), Value::Null);',
    ),
    (
        "journal_owner_wrong_header_kind",
        "journal_owner",
        "header",
        "rust/journal_owner.rs",
        'Value::String("Run".to_owned())',
        'Value::String("Test".to_owned())',
    ),
    (
        "journal_owner_ignore_header",
        "journal_owner",
        "matches_header",
        "rust/journal_owner.rs",
        "transactions[0] == *expected",
        "true",
    ),
    (
        "journal_owner_skip_run_admission",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "if !matches_header(&journal.state.transactions, &expected) {",
        "if false {",
    ),
    (
        "journal_owner_skip_run_append",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "journal.append_with::<L, _>(&events, Borrowed(&mut io))?;",
        "",
    ),
    (
        "journal_owner_skip_recovery",
        "journal_owner",
        "open",
        "rust/journal_owner.rs",
        "journal.recover_with::<L, _>(Borrowed(&mut io))?;",
        "",
    ),
    (
        "journal_owner_ignore_closed",
        "journal_owner",
        "*close*",
        "rust/journal_owner.rs",
        "self.file = None;",
        "let _ = &self.file;",
    ),
    (
        "journal_owner_wrong_initial_access",
        "journal_owner",
        "from_opened",
        "rust/journal_owner.rs",
        "state: State::new(read_only),",
        "state: State::new(false),",
    ),
    (
        "journal_owner_clear_quarantine",
        "journal_owner",
        "append_with",
        "rust/journal_owner.rs",
        "        match crate::journal_io::append::<L, H>(\n            file,",
        "        self.state.poisoned = false;\n"
        "        match crate::journal_io::append::<L, H>(\n"
        "            file,",
    ),
    (
        "journal_owner_clear_read_only",
        "journal_owner",
        "append_with",
        "rust/journal_owner.rs",
        "        match crate::journal_io::append::<L, H>(\n            file,",
        "        self.state.read_only = false;\n"
        "        match crate::journal_io::append::<L, H>(\n"
        "            file,",
    ),
    (
        "journal_owner_append_wrong_state",
        "journal_owner",
        "append_with",
        "rust/journal_owner.rs",
        "        match crate::journal_io::append::<L, H>(\n            file,\n            &mut self.state,",
        "        let mut unrelated = State::new(false);\n"
        "        match crate::journal_io::append::<L, H>(\n"
        "            file,\n"
        "            &mut unrelated,",
    ),
    (
        "journal_owner_recover_wrong_state",
        "journal_owner",
        "recover_with",
        "rust/journal_owner.rs",
        "        match crate::journal_io::recover::<L, S>(\n            file,\n            &mut self.state,",
        "        let mut unrelated = State::new(false);\n"
        "        match crate::journal_io::recover::<L, S>(\n"
        "            file,\n"
        "            &mut unrelated,",
    ),
    (
        "journal_owner_mask_append_failure",
        "journal_owner",
        "append_with",
        "rust/journal_owner.rs",
        "Err(error) => Err(Error::Append(error)),",
        "Err(_error) => Ok(0),",
    ),
)
for _name, _module, _function, *_ in JOURNAL_OWNER_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


CATALOG_GC_MUTATIONS = (
    (
        "gc_failed_point_keeps_ready",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "            Err(error) => {\n"
        "                self.log.invalidate();\n"
        "                Err(Error::Point(error))\n"
        "            }",
        "            Err(error) => {\n                Err(Error::Point(error))\n            }",
    ),
    (
        "gc_failed_receipt_point_keeps_ready",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        '        if let Err(error) = self.io.device.point("before_gc_receipt") {\n'
        "            self.log.invalidate();\n"
        "            return Err(Error::Point(error));\n"
        "        }",
        '        if let Err(error) = self.io.device.point("before_gc_receipt") {\n'
        "            return Err(Error::Point(error));\n"
        "        }",
    ),
    (
        "gc_failed_repair_keeps_ready",
        "gc_collection",
        "*",
        "rust/gc_collection.rs",
        "        if outcome.is_err() {\n            self.log.invalidate();\n        }",
        "",
    ),
    (
        "catalog_gc_callback_skips_collection",
        "catalog_gc",
        "*",
        "rust/catalog_gc.rs",
        "        match catalog.collect_queue_native::<L, H, P>(self.prepared, self.hooks, self.physical) {\n"
        "            Ok(value) => Ok(value),\n"
        "            Err(error) => Err(L::queue_error(error)),\n"
        "        }",
        "        Ok(crate::gc_collection::Collected {\n"
        "            statistics: crate::gc_batch::Statistics { files:0, bytes:0 }, retained:0,\n"
        "        })",
    ),
    (
        "environment_directory_success_changes_writer",
        "directory_io",
        "*",
        "rust/directory_io.rs",
        "result.is_ok() ==> final(self).success_context() == old(self).success_context() && "
        "final(self)@.synced.contains(*path),",
        "result.is_ok() ==> final(self)@.synced.contains(*path),",
    ),
    (
        "environment_catalog_drops_callback_admissibility",
        "catalog_runtime",
        "*",
        "rust/catalog_runtime.rs",
        "&& (self.call.is_some() ==> self.call->Some_0.admissible(self.catalog.fixed()))",
        "",
    ),
)
for _name, _module, _function, *_ in CATALOG_GC_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


REFERENCE_NODE_MUTATIONS = (
    (
        "reference_node_skips_usage_types",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if get(reference, "records").as_u64().is_none()\n'
        '        || get(reference, "payload_bytes").as_u64().is_none()\n'
        '        || get(reference, "tokens").as_u64().is_none()',
        "if false",
    ),
    (
        "reference_node_skips_count",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if refs.len() as u64 != count(reference, "records") {',
        "if false {",
    ),
    (
        "reference_node_permits_duplicate",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        "if !known.insert(key) {",
        "if !known.insert(key) && false {",
    ),
    (
        "reference_node_skips_task",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if get(get(&env, "metadata"), "task_id").as_str() != Some(wanted) {',
        "if false {",
    ),
    (
        "reference_node_skips_attempt",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if get(get(&env, "metadata"), "attempt_id").as_str() != Some(wanted) {',
        "if false {",
    ),
    (
        "reference_node_skips_reference_digest",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if get(reference, "digest").as_str() != Some(actual.as_str())\n'
        '        || get(&manifest, "digest").as_str() != Some(actual.as_str())',
        'if get(&manifest, "digest").as_str() != Some(actual.as_str())',
    ),
    (
        "reference_node_skips_manifest_digest",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if get(reference, "digest").as_str() != Some(actual.as_str())\n'
        '        || get(&manifest, "digest").as_str() != Some(actual.as_str())',
        'if get(reference, "digest").as_str() != Some(actual.as_str())',
    ),
    (
        "reference_node_skips_payload_total",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if payload_bytes != count(reference, "payload_bytes") || tokens != count(reference, "tokens") {',
        'if tokens != count(reference, "tokens") {',
    ),
    (
        "reference_node_skips_token_total",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'if payload_bytes != count(reference, "payload_bytes") || tokens != count(reference, "tokens") {',
        'if payload_bytes != count(reference, "payload_bytes") {',
    ),
    (
        "reference_node_drops_returned_dependency",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        "let dependencies = copies(deps);",
        "let dependencies = Vec::new();",
    ),
    (
        "reference_node_drops_dependency_accounting",
        "reference_node",
        "*",
        "rust/reference_node.rs",
        'let payload = add_values(deps, "payload_bytes", payload)?;',
        "let payload = payload;",
    ),
)
for _name, _module, _function, *_ in REFERENCE_NODE_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


RECORD_MANIFEST_MUTATIONS = (
    (
        "record_manifest_allows_forward_local",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        'if n >= number(item(parent, "manifest"), "ordinal") {',
        'if n > number(item(parent, "manifest"), "ordinal") {',
    ),
    (
        "record_manifest_wrong_segment",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        'item(item(parent, "manifest"), "segment").clone(),',
        "Value::Null,",
    ),
    (
        "record_manifest_retains_local_field",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        '    map.remove("local");',
        "",
    ),
    (
        "record_manifest_skips_dependency_manifest",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        '    map.insert("manifest".to_owned(), reference);',
        "",
    ),
    (
        "record_manifest_drops_records",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        "while i < records.len() {",
        "while i < records.len() && i < 1 {",
    ),
    (
        "record_manifest_drops_dependencies",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        "while i < deps.len() {",
        "while i < deps.len() && false {",
    ),
    (
        "record_manifest_permits_mixed_schema",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        'if codec != Some("record-set.v2") || version != 2 {',
        'if codec != Some("record-set.v2") && version != 2 {',
    ),
    (
        "record_manifest_rewrites_v1",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        'if codec == Some("record-set.v1") && version == 1 {\n        return Ok(value);',
        'if codec == Some("record-set.v1") && version == 1 {\n        return Ok(Value::Null);',
    ),
    (
        "record_manifest_reads_wrong_reference",
        "record_manifest",
        "*",
        "rust/record_manifest.rs",
        'reader.record(item(reference, "manifest"))',
        "reader.record(reference)",
    ),
    (
        "closure_node_skips_reclaimed_check",
        "closure_node",
        "live_pack",
        "rust/closure_node.rs",
        "    if deleted.contains(&pack) {\n        return Err(Admission::Reclaimed);\n    }",
        "",
    ),
)
for _name, _module, _function, *_ in RECORD_MANIFEST_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module

INDEX_RETRY_MUTATIONS = (
    (
        "index_reuses_wrong_reference",
        "record_index",
        "*",
        "rust/record_index.rs",
        "if *cached == *reference {",
        "if true {",
    ),
    (
        "index_preserves_stale_after_error",
        "record_index",
        "*",
        "rust/record_index.rs",
        "        self.last = None;",
        "",
    ),
    (
        "index_stores_wrong_cache_key",
        "record_index",
        "*",
        "rust/record_index.rs",
        "let cached = reference.clone();",
        "let cached = Value::Null;",
    ),
    (
        "index_selects_wrong_ordinal",
        "record_index",
        "*",
        "rust/record_index.rs",
        "let member = &records[ordinal as usize];",
        "let member = &records[0];",
    ),
    (
        "extent_retry_mislabels_permanent",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "Err(Failure::Permanent(error)) => return Err(Error::Permanent(error)),",
        "Err(Failure::Permanent(error)) => return Err(Error::Exhausted(error)),",
    ),
    (
        "extent_retry_mislabels_exhaustion",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "return Err(Error::Exhausted(error));",
        "return Err(Error::Permanent(error));",
    ),
    (
        "extent_retry_wrong_delay",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "observer.wait(retry + 1, delays.len(), *delay, &error);",
        "observer.wait(retry + 1, delays.len(), 0, &error);",
    ),
    (
        "extent_retry_omits_wait",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "observer.wait(retry + 1, delays.len(), *delay, &error);",
        "",
    ),
    (
        "extent_observer_omits_primitive_wait",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "        self.wait.wait(delay);",
        "",
    ),
    (
        "extent_calls_double_read",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        "        self.call.read()",
        "        let _ = self.call.read();\n        self.call.read()",
    ),
    (
        "extent_visibility_classifier_inverted",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        '    kind == "ExtentVisibility"',
        '    kind != "ExtentVisibility"',
    ),
    (
        "extent_exhaustion_leaks_internal_error",
        "extent_retry",
        "*",
        "rust/extent_retry.rs",
        '    *kind = "CorruptData".to_owned();',
        "",
    ),
)
for _name, _module, _function, *_ in INDEX_RETRY_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


QUEUE_COMMIT_MUTATIONS = (
    (
        "queue_public_owner_clobber",
        "queue_gc",
        "public_owner",
        "rust/queue_gc.rs",
        'not_publication(name) && !name.as_bytes().starts_with("queue:".as_bytes())',
        "not_publication(name)",
    ),
    (
        "handoff_wrong_prefix",
        "queue_handoff",
        "envelope",
        "rust/queue_handoff.rs",
        "parts.push(Value::String(prefix.to_owned()));",
        "parts.push(Value::String(String::new()));",
    ),
    (
        "handoff_wrong_sequence",
        "queue_handoff",
        "envelope",
        "rust/queue_handoff.rs",
        "parts.push(Value::Number(sequence.into()));",
        "parts.push(Value::Number(0u64.into()));",
    ),
    (
        "handoff_drop_business",
        "queue_handoff",
        "envelope",
        "rust/queue_handoff.rs",
        "items.push(business[index].clone());",
        "items.push(Value::Null);",
    ),
    (
        "handoff_wrong_marker_id",
        "queue_handoff",
        "marker_event",
        "rust/queue_handoff.rs",
        'object.insert("id".to_owned(), Value::String(id.to_owned()));',
        'object.insert("id".to_owned(), Value::String(String::new()));',
    ),
    ("handoff_omit_marker", "queue_handoff", "attach", "rust/queue_handoff.rs", "events.push(marker_event(&id));", ""),
    (
        "handoff_accept_misplaced",
        "queue_handoff",
        "recover",
        "rust/queue_handoff.rs",
        "if !no_markers(events) {",
        "if false {",
    ),
    (
        "handoff_accept_repeated",
        "queue_handoff",
        "recover",
        "rust/queue_handoff.rs",
        "if !no_markers(business) {",
        "if false {",
    ),
    (
        "handoff_ignore_identity",
        "queue_handoff",
        "recover",
        "rust/queue_handoff.rs",
        'if crate::queue_roots::field_value(&events[events.len() - 1], "id").as_str()\n        != Some(id.as_str())',
        "if false",
    ),
    (
        "handoff_accept_no_roots",
        "queue_handoff",
        "recover",
        "rust/queue_handoff.rs",
        "if found.is_empty() {",
        "if false {",
    ),
    (
        "handoff_skip_reference",
        "queue_handoff",
        "roots",
        "rust/queue_handoff.rs",
        "crate::queue_roots::references(digester, &business[i], &mut output)?;",
        "",
    ),
    (
        "handoff_forget_root",
        "queue_handoff",
        "values",
        "rust/queue_handoff.rs",
        "result.push(value);",
        "result.push(Value::Null);",
    ),
    ("handoff_forget_pending", "queue_handoff", "recover_into", "rust/queue_handoff.rs", "pending.push(value);", ""),
    (
        "queue_commit_skip_append",
        "queue_commit",
        "persist_and_apply",
        "rust/queue_commit.rs",
        "if let Err(error) = journal.append_with::<L, _>(events, Borrowed(hooks)) {",
        "if let Err(error) = Ok::<(),crate::journal_owner::Error<L::Error>>(()) {",
    ),
    (
        "queue_commit_forget_handoff",
        "queue_commit",
        "persist_and_apply",
        "rust/queue_commit.rs",
        "pending.push(value);",
        "",
    ),
    (
        "queue_commit_skip_quarantine",
        "queue_commit",
        "persist_and_apply",
        "rust/queue_commit.rs",
        "queue_application::arm(&mut journal.state);",
        "",
    ),
    (
        "queue_commit_skip_confirmation",
        "queue_commit",
        "persist_and_apply",
        "rust/queue_commit.rs",
        "queue_application::confirm(&mut journal.state, confirmation);",
        "",
    ),
)
for _name, _module, _function, *_ in QUEUE_COMMIT_MUTATIONS:
    NEGATIVE_FUNCTIONS[_name] = _function
    NEGATIVE_MODULES[_name] = _module


def cases(sources):
    yield "production", sources, False
    for name, _module, _function, path, old, new in (
        *QUEUE_COMMIT_MUTATIONS,
        *INDEX_RETRY_MUTATIONS,
        *RECORD_MANIFEST_MUTATIONS,
        *REFERENCE_NODE_MUTATIONS,
        *CATALOG_GC_MUTATIONS,
        *JOURNAL_OWNER_MUTATIONS,
        *REFERENCE_VALIDATION_MUTATIONS,
        *QUEUE_PROGRESS_MUTATIONS,
        *CATALOG_RUNTIME_MUTATIONS,
        *JOURNAL_IO_MUTATIONS,
        *QUEUE_RETIREMENT_MUTATIONS,
        *NATIVE_FILE_MUTATIONS,
        *GC_COLLECTION_MUTATIONS,
        *WRITER_RETIREMENT_MUTATIONS,
        *QUEUE_GC_MUTATIONS,
        *CATALOG_VALID_MUTATIONS,
        *NAMESPACE_MUTATIONS,
        *QUEUE_ROOT_MUTATIONS,
        *QUEUE_APPLICATION_MUTATIONS,
        *QUEUE_HISTORY_MUTATIONS,
        *QUEUE_REPLAY_MUTATIONS,
        *QUEUE_ADMIN_MUTATIONS,
        *QUEUE_TRANSITION_MUTATIONS,
        *QUEUE_STATE_MUTATIONS,
        *QUEUE_READER_MUTATIONS,
        *QUEUE_TASK_MUTATIONS,
        *QUEUE_EVENT_MUTATIONS,
    ):
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, module, _function, old, new in GC_IO_MUTATIONS:
        path = f"rust/{module}.rs"
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, _function, old, new in GC_RECEIPT_MUTATIONS:
        path = "rust/gc_receipt.rs"
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, _function, old, new in CATALOG_TRANSACTION_MUTATIONS:
        path = "rust/catalog_transaction.rs"
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, _module, _function, path, old, new in FILE_ADAPTER_MUTATIONS:
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, _function, old, new in CLOSURE_NODE_MUTATIONS:
        path = "rust/closure_node.rs"
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, _function, path, old, new in CLOSURE_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _function, path, old, new in GC_MARK_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _function, path, old, new in CROSS_LOG_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _module, _function, path, old, new in JOURNAL_APPEND_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _module, _function, path, old, new in QUEUE_METADATA_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _module, _function, path, old, new in CATALOG_EQUIVALENCE_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _function, path, old, new in CATALOG_APPEND_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _, path, old, new in REQUEST_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _, _, path, old, new in CATALOG_FRAME_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _, _, path, old, new in JOURNAL_STATE_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, old, new in GC_BATCH_MUTATIONS:
        path = "rust/gc_batch.rs"
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _, _, path, old, new in CATALOG_RECOVERY_MUTATIONS:
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, old, new in COMMIT_EXECUTION_MUTATIONS:
        path = "rust/protocol.rs"
        if sources[path].count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: sources[path].replace(old, new)}, True
    for name, _, path, old, new in CATALOG_DECODE_MUTATIONS:
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True
    for name, path, old, new in (
        (
            "catalog_event_wrong_id_length",
            "rust/catalog_event.rs",
            "bytes.len() - start != 64",
            "bytes.len() - start != 63",
        ),
        (
            "catalog_event_skips_final_digit",
            "rust/catalog_event.rs",
            "while i < bytes.len() {",
            "while i < bytes.len() - 1 {",
        ),
        (
            "catalog_event_accepts_uppercase",
            "rust/catalog_event.rs",
            "(b'a' <= byte && byte <= b'f')",
            "(b'A' <= byte && byte <= b'f')",
        ),
        (
            "catalog_event_ignores_staged_prefix",
            "rust/catalog_event.rs",
            "crate::framing::matches_at(bytes, 0, &prefix) && valid_hex(bytes, 7)",
            "valid_hex(bytes, 7)",
        ),
        ("catalog_event_receipt_without_owners", "rust/catalog_event.rs", "if !wire.has_owners {", "if false {"),
        (
            "catalog_event_receipt_retains_owner",
            "rust/catalog_event.rs",
            "if wire.owners[i].1.is_some() {",
            "if false {",
        ),
        (
            "catalog_event_receipt_skips_id",
            "rust/catalog_event.rs",
            "if !valid_handoff_id(&wire.receipts[i]) {",
            "if false {",
        ),
        (
            "catalog_event_ignores_child_names",
            "rust/catalog_event.rs",
            "if !valid_staged_key(key) || !valid_child_names(children) {",
            "if !valid_staged_key(key) {",
        ),
        (
            "catalog_event_ignores_node_name",
            "rust/catalog_event.rs",
            "if !valid_staged_key(key) || !valid_child_names(children) {",
            "if !valid_child_names(children) {",
        ),
        (
            "catalog_event_ignores_graph_compatibility",
            "rust/catalog_event.rs",
            "return crate::consumption::compatible(known, additions);",
            "return true;",
        ),
        (
            "catalog_event_rejects_absent_receipts",
            "rust/catalog_event.rs",
            "if !wire.has_receipts {\n        return true;",
            "if !wire.has_receipts {\n        return false;",
        ),
        (
            "catalog_event_accepts_short_id",
            "rust/catalog_event.rs",
            "if start > bytes.len() || bytes.len() - start != 64 {\n        return false;",
            "if start > bytes.len() || bytes.len() - start != 64 {\n        return true;",
        ),
        (
            "catalog_target_skips_apply",
            "rust/catalog_target.rs",
            "match self.log.replay_record(&operations, raw) {",
            "match Ok::<(), Failure>(()) {",
        ),
        (
            "catalog_target_wrong_payload",
            "rust/catalog_target.rs",
            "self.log.replay_record(&operations, raw)",
            "self.log.replay_record(&operations, &[])",
        ),
        (
            "catalog_target_accepts_decode_error",
            "rust/catalog_target.rs",
            "Err(error) => return Err(Error::Decode(error))",
            "Err(_error) => return Ok(())",
        ),
        (
            "catalog_target_ignores_apply_error",
            "rust/catalog_target.rs",
            "Err(error) => Err(Error::State(error))",
            "Err(_error) => Ok(())",
        ),
        (
            "catalog_log_records_wrong_payload",
            "rust/catalog_log.rs",
            "let record = Record { operations: ops@, payload: payload@ };",
            "let record = Record { operations: ops@, payload: Seq::empty() };",
        ),
        (
            "catalog_log_plan_wrong_payload",
            "rust/catalog_log.rs",
            "payload: verus_exec_expr!(Ghost(payload@))",
            "payload: verus_exec_expr!(Ghost(Seq::empty()))",
        ),
        (
            "catalog_log_applies_wrong_plan_bytes",
            "rust/catalog_log.rs",
            "payload: plan.payload@ };",
            "payload: Seq::empty() };",
        ),
        (
            "environment_target_loses_validity",
            "rust/wal_device.rs",
            "ensures final(self).valid(), final(self).context() == old(self).context(),",
            "ensures final(self).context() == old(self).context(),",
        ),
        (
            "replay_rejects_legal_partial",
            "rust/wal_recovery.rs",
            "if !crate::framing::partial_header_valid(kind, &partial) {",
            "if true {",
        ),
        (
            "replay_rejects_valid_header",
            "rust/wal_recovery.rs",
            "let length = match crate::framing::parse_header(kind, &header, sequence, &header_digest) {",
            "if header.len() == 56 { return Err(Failure::Malformed(Malformed::Header)); }\n    let length = match crate::framing::parse_header(kind, &header, sequence, &header_digest) {",
        ),
        (
            "replay_rejects_empty_frame",
            "rust/wal_recovery.rs",
            "if length > usize::MAX as u64 {",
            "if length == 0 || length > usize::MAX as u64 {",
        ),
        (
            "replay_rejects_valid_trailer",
            "rust/wal_recovery.rs",
            "if !crate::framing::trailer_valid(kind, &trailer, &frame_digest) {",
            "if true {",
        ),
        (
            "recovery_rejects_valid_cursor",
            "rust/wal_recovery.rs",
            "if start > size || durable > size {",
            "if true {",
        ),
        (
            "recovery_rejects_usable_sequence",
            "rust/wal_recovery.rs",
            "if sequence == u64::MAX {",
            "if sequence == 0 || sequence == u64::MAX {",
        ),
        (
            "recovery_rejects_confirmed_boundary",
            "rust/wal_recovery.rs",
            "if position < durable {",
            "if position <= durable {",
        ),
        (
            "replay_mislabels_trailer_error",
            "rust/wal_recovery.rs",
            "return Err(Failure::Malformed(Malformed::Trailer));",
            "return Err(Failure::Malformed(Malformed::Header));",
        ),
        (
            "recovery_mislabels_frame_io_error",
            "rust/wal_recovery.rs",
            "            Err(e) => return Err(e),",
            "            Err(_e) => return Err(Failure::Malformed(Malformed::Header)),",
        ),
        (
            "wal_append_ignores_cursor",
            "rust/wal_append.rs",
            "position.cursor != offset || position.size != offset",
            "position.size != offset",
        ),
        (
            "wal_append_ignores_length",
            "rust/wal_append.rs",
            "position.cursor != offset || position.size != offset",
            "position.cursor != offset",
        ),
        (
            "wal_append_changes_sequence",
            "rust/wal_append.rs",
            "framing::prefix(kind, sequence, payload.len() as u64)",
            "framing::prefix(kind, 0, payload.len() as u64)",
        ),
        (
            "wal_append_hashes_wrong_frame",
            "rust/wal_append.rs",
            "device.hash(&header, payload)",
            "device.hash(&prefix, payload)",
        ),
        (
            "wal_append_skips_payload",
            "rust/wal_append.rs",
            "AppendEffect::Payload => device.write(payload)",
            "AppendEffect::Payload => device.write(&[])",
        ),
        (
            "wal_append_writes_wrong_trailer",
            "rust/wal_append.rs",
            "AppendEffect::Trailer => device.write(&trailer)",
            "AppendEffect::Trailer => device.write(&header)",
        ),
        (
            "wal_append_skips_sync",
            "rust/wal_append.rs",
            "AppendEffect::Sync => device.sync()",
            "AppendEffect::Sync => Ok(())",
        ),
        (
            "wal_append_reports_error_as_success",
            "rust/wal_append.rs",
            "if let Err(error) = outcome {\n            return Err(Failure::Device(error));\n        }",
            "if let Err(_error) = outcome {\n            return Ok(Appended { end, sequence: sequence + 1 });\n        }",
        ),
        (
            "catalog_confirms_before_wal_write",
            "rust/catalog_log.rs",
            "if let Err(error) = crate::wal_append::append(",
            "let _ = self.finish_append();\n        if let Err(error) = crate::wal_append::append(",
        ),
        (
            "catalog_ignores_wal_write_error",
            "rust/catalog_log.rs",
            "return Err(AppendFailure::Write(error));",
            "return Ok(());",
        ),
        (
            "catalog_writes_without_admission",
            "rust/catalog_log.rs",
            "if let Err(error) = self.begin_append(operations, payload) {\n            return Err(AppendFailure::State(error));\n        }",
            "",
        ),
        (
            "environment_append_sync_lies",
            "rust/wal_device.rs",
            "&& final(self)@.durable == old(self)@.bytes.len(),",
            "&& true,",
        ),
        (
            "environment_append_changes_bytes",
            "rust/wal_device.rs",
            "result.is_ok() ==> final(self)@.bytes == old(self)@.bytes + part@,",
            "true,",
        ),
        (
            "environment_append_inspect_wrong_cursor",
            "rust/wal_device.rs",
            "&& result->Ok_0.cursor == old(self)@.cursor,",
            "&& true,",
        ),
        (
            "catalog_log_keeps_rebuild_indexes",
            "rust/catalog_log.rs",
            "self.indexes = Indexes::new();",
            "",
        ),
        (
            "catalog_log_keeps_uncertain_plan",
            "rust/catalog_log.rs",
            "self.pending = None;",
            "",
        ),
        (
            "catalog_log_forgets_confirmed_floor",
            "rust/catalog_log.rs",
            "let rebuilt = self.dirty();",
            "self.recovery.durable = 0;\n        let rebuilt = self.dirty();",
        ),
        (
            "catalog_log_confirms_partial_apply",
            "rust/catalog_log.rs",
            "if !self.complete\n            || self.pending.is_some()",
            "if false\n            || self.pending.is_some()",
        ),
        (
            "catalog_log_confirms_wrong_end",
            "rust/catalog_log.rs",
            "|| end != self.offset",
            "|| false",
        ),
        (
            "catalog_log_confirms_wrong_sequence",
            "rust/catalog_log.rs",
            "|| sequence != self.sequence",
            "|| false",
        ),
        (
            "catalog_log_keeps_replay_cursor",
            "rust/catalog_log.rs",
            "self.offset = end;\n        self.sequence += 1;",
            "self.sequence += 1;",
        ),
        (
            "catalog_log_appends_before_recovery",
            "rust/catalog_log.rs",
            "if !self.ready() {",
            "if false {",
        ),
        (
            "catalog_log_changes_pending_operations",
            "rust/catalog_log.rs",
            "operations: ops,\n            end,",
            "operations: Vec::new(),\n            end,",
        ),
        (
            "catalog_log_skips_post_sync_apply",
            "rust/catalog_log.rs",
            "self.indexes.apply(&plan.operations)",
            "Ok::<(), usize>(())",
        ),
        (
            "catalog_log_skips_append_confirmation",
            "rust/catalog_log.rs",
            "if !replay::confirm(&mut self.recovery, plan.end) {",
            "if false {",
        ),
        (
            "catalog_log_rejects_valid_append",
            "rust/catalog_log.rs",
            "if !self.ready() {",
            "if true {",
        ),
        (
            "catalog_log_changes_replay_history",
            "rust/catalog_log.rs",
            "let record = Record { operations: ops@, payload: payload@ };",
            "let record = Record { operations: Seq::empty(), payload: payload@ };",
        ),
        (
            "catalog_log_changes_partial_history",
            "rust/catalog_log.rs",
            "self.partial = Ghost(ops@.take(index as int));",
            "self.partial = Ghost(Seq::empty());",
        ),
        (
            "catalog_validation_accepts_unmarked",
            "rust/catalog_state.rs",
            "if !self.deleted.contains(path) && !marked.contains(path)",
            "if false",
        ),
        (
            "catalog_validation_ignores_history",
            "rust/catalog_state.rs",
            "if !self.deleted.contains(path) && !marked.contains(path)",
            "if !marked.contains(path)",
        ),
        (
            "catalog_validation_ignores_new_marks",
            "rust/catalog_state.rs",
            "marked.insert(path.clone());",
            "",
        ),
        (
            "catalog_validation_invents_mark",
            "rust/catalog_state.rs",
            "if !self.deleted.contains(path) && !marked.contains(path)",
            "marked.insert(path.clone());\n                    if !self.deleted.contains(path) && !marked.contains(path)",
        ),
        (
            "json_lookup_skips_entry",
            "rust/catalog_json.rs",
            "let mut it = object.iter();\n    #[cfg_attr",
            "let mut it = object.iter();\n    it.next();\n    #[cfg_attr",
        ),
        ("json_strings_skips_first", "rust/catalog_json.rs", "let mut i = 0;", "let mut i = 1;"),
        (
            "json_strings_changes_text",
            "rust/catalog_json.rs",
            "result.push(text.to_owned());",
            "result.push(String::new());",
        ),
        ("json_owners_disallows_release", "rust/catalog_json.rs", "if allow_null && item.is_null()", "if false"),
        ("json_edges_accepts_null", "rust/catalog_json.rs", "self::owners(v, false)", "self::owners(v, true)"),
        (
            "json_packs_changes_state",
            "rust/catalog_json.rs",
            "result.push((key.clone(), sealed));",
            "result.push((key.clone(), !sealed));",
        ),
        (
            "json_deleted_reads_wrong_field",
            "rust/catalog_json.rs",
            'let deleted = match get(value, "deleted")',
            'let deleted = match get(value, "reclaimed")',
        ),
        (
            "json_hides_receipt_presence",
            "rust/catalog_json.rs",
            'let has_receipts = get(value, "consumed_handoffs").is_some();',
            "let has_receipts = false;",
        ),
        ("environment_json_array_false_negative", JSON_SPECS, "None => !(json(*value) is Array)", "None => true"),
        (
            "environment_json_iterator_omits_entries",
            JSON_SPECS,
            "remaining(r).unref() == entries(*value)",
            "remaining(r).len() <= entries(*value).len()",
        ),
        (
            "staging_graph_ignores_conflict",
            "rust/consumption.rs",
            "Some(previous) => same_children(previous, children)",
            "Some(previous) => true",
        ),
        (
            "staging_graph_ignores_missing_child",
            "rust/consumption.rs",
            "if !covered(known, additions, children)",
            "if false",
        ),
        (
            "staging_graph_skips_node",
            "rust/consumption.rs",
            "let mut entries = additions.iter();",
            "let mut entries = additions.iter();\n    entries.next();",
        ),
        (
            "staging_graph_ignores_child_length",
            "rust/consumption.rs",
            "if left.len() != right.len() {\n        return false;",
            "if left.len() != right.len() {\n        return true;",
        ),
        (
            "staging_graph_ignores_child_content",
            "rust/consumption.rs",
            "if !String::eq(&left[i], &right[i])",
            "if false",
        ),
        (
            "replay_driver_ignores_sync_error",
            "rust/wal_recovery.rs",
            "        device.sync()?;",
            "        let _ = device.sync();",
        ),
        (
            "replay_driver_ignores_seek_error",
            "rust/wal_recovery.rs",
            "    device.seek(end)?;",
            "    let _ = device.seek(end);",
        ),
        (
            "catalog_binding_ignores_pool",
            "rust/identities.rs",
            "expected_root == actual_root && expected_run == actual_run",
            "expected_run == actual_run",
        ),
        (
            "catalog_binding_ignores_run",
            "rust/identities.rs",
            "expected_root == actual_root && expected_run == actual_run",
            "expected_root == actual_root",
        ),
        (
            "catalog_binding_ignores_inode",
            "rust/identities.rs",
            "expected.0 == observed.0 && expected.1 == observed.1",
            "expected.0 == observed.0",
        ),
        (
            "catalog_binding_ignores_device",
            "rust/identities.rs",
            "expected.0 == observed.0 && expected.1 == observed.1",
            "expected.1 == observed.1",
        ),
        ("catalog_ignores_owner", "rust/catalog_state.rs", "self.owners.insert(key.clone(), paths.clone());", ""),
        ("catalog_ignores_release", "rust/catalog_state.rs", "self.owners.remove(key);", ""),
        (
            "catalog_reopens_sealed",
            "rust/catalog_state.rs",
            "self.packs.insert(path.clone(), *sealed);",
            "self.packs.insert(path.clone(), false);",
        ),
        (
            "catalog_ignores_mark",
            "rust/catalog_state.rs",
            "crate::reclamation::mark(\n                    &mut self.deleted,\n                    &mut self.pending_deletions,\n                    path.clone(),\n                );",
            "",
        ),
        ("catalog_keeps_reclaimed_pack", "rust/catalog_state.rs", "self.packs.remove(path);", ""),
        (
            "catalog_ignores_edges",
            "rust/catalog_state.rs",
            "self.staging_edges.insert(key.clone(), children.clone());",
            "",
        ),
        (
            "catalog_replay_skips_operation",
            "rust/catalog_state.rs",
            "self.apply_one(&ops[i])",
            "self.apply_one(&ops[0])",
        ),
        (
            "catalog_replay_swallows_error",
            "rust/catalog_state.rs",
            "if !self.apply_one(&ops[i]) {",
            "if { self.apply_one(&ops[i]); false } {",
        ),
        ("catalog_omits_owner_roots", "rust/catalog_state.rs", "extend_roots(&mut result, paths);", ""),
        (
            "catalog_omits_root_member",
            "rust/catalog_state.rs",
            "result.insert(path.clone());\n        #[cfg(straw_verus)]",
            "// missing root\n        #[cfg(straw_verus)]",
        ),
        ("catalog_snapshot_ignores_owner", "rust/catalog_state.rs", "owned: live.contains(path),", "owned: false,"),
        (
            "catalog_snapshot_ignores_tombstone",
            "rust/catalog_state.rs",
            "tombstoned: self.deleted.contains(path),",
            "tombstoned: false,",
        ),
        ("replay_accepts_uncertain_state", "rust/replay.rs", "    !state.rebuild", "    true"),
        (
            "environment_replay_hash_unbound",
            "rust/wal_device.rs",
            "ensures result@ == digest(first@ + second@), result.len() == 32,",
            "ensures result.len() == 32,",
        ),
        (
            "environment_replay_apply_drops_record",
            "rust/wal_device.rs",
            "final(self)@ == old(self)@.push(payload@)",
            "final(self)@ == old(self)@",
        ),
        (
            "environment_replay_wrong_read_bytes",
            "rust/wal_device.rs",
            "read_matches(old(self)@.bytes, old(self)@.cursor, length as nat, result->Ok_0@)",
            "result->Ok_0.len() == length",
        ),
        (
            "environment_replay_wrong_seek_identity",
            "rust/wal_device.rs",
            "ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,\n            final(self)@.buffered == old(self)@.buffered,",
            "ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.buffered == old(self)@.buffered,",
        ),
        (
            "environment_replay_seek_lies",
            "rust/wal_device.rs",
            "result.is_ok() ==> final(self)@.cursor == offset,",
            "true,",
        ),
        (
            "environment_replay_sync_lies",
            "rust/wal_device.rs",
            "result.is_ok() ==> final(self)@.durable == final(self)@.bytes.len(),",
            "true,",
        ),
        ("replay_driver_skips_sync", "rust/wal_recovery.rs", "        device.sync()?;", "        // sync removed"),
        ("replay_driver_skips_seek", "rust/wal_recovery.rs", "    device.seek(end)?;", "    // seek removed"),
        (
            "replay_driver_skips_truncate",
            "rust/wal_recovery.rs",
            "            device.truncate(end)?;",
            "            // truncate removed",
        ),
        (
            "replay_driver_keeps_buffer",
            "rust/wal_recovery.rs",
            "    device.discard_buffer();",
            "    // discard removed",
        ),
        ("replay_driver_skips_floor", "rust/wal_recovery.rs", "if position < durable", "if false"),
        ("replay_driver_skips_apply", "rust/wal_recovery.rs", "target.apply(&payload)", "Ok::<(), T::Error>(())"),
        (
            "replay_driver_wrong_sequence",
            "rust/wal_recovery.rs",
            "let expected_sequence = sequence;",
            "let expected_sequence = 0;",
        ),
        (
            "replay_driver_wrong_header_hash_span",
            "rust/wal_recovery.rs",
            "copy_range(&header, 0, 24)",
            "copy_range(&header, 0, 23)",
        ),
        (
            "replay_driver_omits_header_from_frame_hash",
            "rust/wal_recovery.rs",
            "device.hash(&header, &payload)",
            "device.hash(&[], &payload)",
        ),
        (
            "replay_driver_truncates_corrupt_frame",
            "rust/wal_recovery.rs",
            "return Err(Failure::Malformed(Malformed::Trailer));",
            "return Ok(Frame::Incomplete);",
        ),
        (
            "replay_driver_accepts_bad_partial_magic",
            "rust/wal_recovery.rs",
            "if !crate::framing::partial_header_valid(kind, &partial)",
            "if false",
        ),
        (
            "replay_driver_skips_trailer_validation",
            "rust/wal_recovery.rs",
            "if !crate::framing::trailer_valid(kind, &trailer, &frame_digest)",
            "if false",
        ),
        (
            "control_retry_does_not_restart",
            "verification/protocol/verus/retry_liveness.rs",
            "RetryEvent::Retry => retry_enabled(s) && n == initial()",
            "RetryEvent::Retry => retry_enabled(s) && n == s",
        ),
        (
            "control_effect_skips_step",
            "verification/protocol/verus/retry_liveness.rs",
            "n == advance(s, success, limit)",
            "n == initial()",
        ),
        (
            "environment_control_no_retry_fairness",
            "verification/common/verus/environment.rs",
            "exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), RetryEvent::Retry)",
            "true",
        ),
        (
            "environment_control_no_io_fairness",
            "verification/common/verus/environment.rs",
            "exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), RetryEvent::Io { .. })",
            "true",
        ),
        (
            "environment_control_errors_forever",
            "verification/common/verus/environment.rs",
            "(matches!(events(i), RetryEvent::Io { .. }) ==> events(i)->success)",
            "true",
        ),
        (
            "environment_control_crashes_forever",
            "verification/common/verus/environment.rs",
            "!matches!(events(i), RetryEvent::Crash)",
            "true",
        ),
        (
            "wal_sync_before_trailer",
            "rust/wal.rs",
            "2 => Some(AppendEffect::Trailer)",
            "2 => Some(AppendEffect::Sync)",
        ),
        (
            "framing_ignores_magic",
            "rust/framing.rs",
            "if !matches_at(data, 0, &magic(kind)) || !matches_at(data, 24, digest)",
            "if !matches_at(data, 24, digest)",
        ),
        (
            "framing_ignores_header_digest",
            "rust/framing.rs",
            "if !matches_at(data, 0, &magic(kind)) || !matches_at(data, 24, digest)",
            "if !matches_at(data, 0, &magic(kind))",
        ),
        ("framing_ignores_sequence", "rust/framing.rs", "if decode_u64(data, 8) != sequence", "if false"),
        ("framing_wrong_length_field", "rust/framing.rs", "Some(decode_u64(data, 16))", "Some(decode_u64(data, 8))"),
        (
            "framing_ignores_trailer_marker",
            "rust/framing.rs",
            "&& matches_at(data, 32, &end_marker(kind))",
            "&& true",
        ),
        ("framing_ignores_trailer_digest", "rust/framing.rs", "&& matches_at(data, 0, digest)", "&& true"),
        ("framing_accepts_bad_partial_magic", "rust/framing.rs", "if partial[i] != expected[i]", "if false"),
        (
            "framing_wrong_decode_endian",
            "rust/framing.rs",
            "| ((data[start + 1] as u64) << 8)",
            "| ((data[start + 1] as u64) << 16)",
        ),
        ("framing_loses_high_byte", "rust/framing.rs", "        (value >> 56) as u8,", "        0,"),
        (
            "framing_encodes_sequence_as_length",
            "rust/framing.rs",
            "result.extend_from_slice(&encode_u64(length));",
            "result.extend_from_slice(&encode_u64(sequence));",
        ),
        ("wal_skip_payload", "rust/wal.rs", "1 => Some(AppendEffect::Payload)", "1 => Some(AppendEffect::Trailer)"),
        ("wal_ignore_failure", "rust/wal.rs", "    if success {", "    if true {"),
        (
            "wal_ack_before_sync",
            "rust/wal.rs",
            "    !state.failed && state.completed == 4",
            "    !state.failed && state.completed >= 3",
        ),
        ("wal_wrong_frame_overhead", "rust/wal.rs", "    let frame = payload + 96;", "    let frame = payload + 95;"),
        ("wal_frame_ignores_size", "rust/wal.rs", "    if frame > size - offset {", "    if false {"),
        ("wal_frame_wrong_origin", "rust/wal.rs", "        end: offset + frame,", "        end: frame,"),
        (
            "wal_reject_complete_frame",
            "rust/wal.rs",
            "    if frame > size - offset {",
            "    if frame >= size - offset {",
        ),
        ("frame_position_ignores_extent", "rust/bounds.rs", "    if end > extent {", "    if false {"),
        (
            "frame_position_ignores_base_overflow",
            "rust/bounds.rs",
            "    add(base, extent)?;",
            "    // extent address check removed",
        ),
        ("frame_position_wrong_origin", "rust/bounds.rs", "    add(base, relative)\n}", "    add(0, relative)\n}"),
        (
            "frame_position_skips_header",
            "rust/bounds.rs",
            "    let end = frame_end(relative, metadata, payload)?;",
            "    let end = add(add(relative, metadata)?, payload)?;",
        ),
        ("typed_ignore_length", "rust/typed.rs", "actual.length != expected.length", "false"),
        ("typed_ignore_dtype", "rust/typed.rs", "!String::eq(&actual.dtype, &expected.dtype)", "false"),
        ("typed_ignore_rank", "rust/typed.rs", "actual.shape.len() != expected.shape.len()", "false"),
        ("typed_ignore_dimensions", "rust/typed.rs", "while i < actual.shape.len()", "while i < 0usize"),
        ("typed_reject_match", "rust/typed.rs", "    true\n}", "    false\n}"),
        ("range_span_skips_prefix", "rust/bounds.rs", "let first = start - start % chunk;", "let first = start;"),
        (
            "range_span_includes_next_chunk",
            "rust/bounds.rs",
            "let last = (stop - 1) - (stop - 1) % chunk;",
            "let last = stop - stop % chunk;",
        ),
        (
            "range_span_reads_tail",
            "rust/bounds.rs",
            "    Some((first, end))",
            "    Some((first, length))",
        ),
        (
            "range_span_reads_empty",
            "rust/bounds.rs",
            "        return Some((start, stop));",
            "        return Some((0, length));",
        ),
        ("committed_cleanup_forgets_error", "rust/consumption.rs", "    if confirmed {", "    if true {"),
        ("committed_cleanup_never_finishes", "rust/consumption.rs", "    if confirmed {", "    if false {"),
        (
            "committed_receipt_not_recorded",
            "rust/consumption.rs",
            "        known.insert(receipts[i].clone());",
            "        // receipt omitted",
        ),
        (
            "committed_receipt_forgets_history",
            "rust/consumption.rs",
            "        known.insert(receipts[i].clone());",
            "        known.clear();\n        known.insert(receipts[i].clone());",
        ),
        ("writer_access_ignores_poison", "rust/access.rs", "    if poisoned {", "    if false {"),
        (
            "writer_access_denies_valid_token",
            "rust/access.rs",
            "\n        Some(value) => Access::Granted(value),",
            "\n        Some(_value) => Access::Busy,",
        ),
        (
            "writer_access_misclassifies_busy",
            "rust/access.rs",
            "\n        None => Access::Busy,",
            "\n        None => Access::Poisoned,",
        ),
        ("preparation_owner_forgotten_after_error", "rust/holding.rs", "    if safe {", "    if true {"),
        ("preparation_owner_never_cleared", "rust/holding.rs", "    if safe {", "    if false {"),
        (
            "consumption_accepts_missing_node",
            "rust/consumption.rs",
            "if !known.contains_key(&children[i]) && !additions.contains_key(&children[i]) {\n            return false;",
            "if !known.contains_key(&children[i]) && !additions.contains_key(&children[i]) {\n            return true;",
        ),
        (
            "consumption_ignores_additions",
            "rust/consumption.rs",
            "!known.contains_key(&children[i]) && !additions.contains_key(&children[i])",
            "!known.contains_key(&children[i])",
        ),
        ("consumption_skips_children", "rust/consumption.rs", "while i < children.len()", "while i < 0usize"),
        (
            "handoff_accepts_another_request",
            "rust/finalization.rs",
            "    !state.active || (same_api(state.api, api) && same_id)\n",
            "true",
        ),
        (
            "handoff_ignores_api",
            "rust/finalization.rs",
            "    !state.active || (same_api(state.api, api) && same_id)\n",
            "!state.active || same_id",
        ),
        ("handoff_starts_inactive", "rust/finalization.rs", "active: true,", "active: false,"),
        ("handoff_never_acknowledges", "rust/finalization.rs", "state.active = false;", "state.active = true;"),
        (
            "preparation_truncates_unknown_prefix",
            "rust/incomplete.rs",
            "start((value, 0), true)",
            "start((value, 0), false)",
        ),
        ("preparation_loses_captured_prefix", "rust/incomplete.rs", "state.value.1 = prefix;", "state.value.1 = 0;"),
        (
            "preparation_removes_existing_pack",
            "rust/incomplete.rs",
            "state.standalone = standalone;",
            "state.standalone = true;",
        ),
        (
            "parent_missing_never_recovers",
            "rust/directory.rs",
            "        ParentAction::RecreateAndSync\n",
            "        ParentAction::Fail\n",
        ),
        (
            "parent_skips_existing_sync",
            "rust/directory.rs",
            "        ParentAction::Sync\n",
            "        ParentAction::Fail\n",
        ),
        (
            "parent_ignores_other_errors",
            "rust/directory.rs",
            "        ParentAction::Fail\n",
            "        ParentAction::RecreateAndSync\n",
        ),
        ("incomplete_accepts_inherited_writer", "rust/incomplete.rs", "    owner == current\n}", "    true\n}"),
        (
            "incomplete_seals_before_barrier",
            "rust/incomplete.rs",
            "1 if state.standalone => Some(Effect::SyncParent),",
            "1 if state.standalone => Some(Effect::SealFinal),",
        ),
        ("incomplete_skips_file_sync", "rust/incomplete.rs", "1 => Some(Effect::SyncFile),", "1 => None,"),
        ("incomplete_forgets_restart", "rust/incomplete.rs", "state.completed = 0;", "state.completed = 1;"),
        ("incomplete_ignores_failure", "rust/incomplete.rs", "state.failed = !success;", "state.failed = false;"),
        (
            "incomplete_finishes_early",
            "rust/incomplete.rs",
            "!state.failed && state.completed == if state.standalone { 3 } else { 2 }",
            "!state.failed && state.completed >= 1",
        ),
        (
            "identity_trusts_legacy_encoding",
            "rust/identities.rs",
            "None => Encoding::Legacy,",
            "None => Encoding::Resolved,",
        ),
        (
            "identity_accepts_unknown_encoding",
            "rust/identities.rs",
            "Some(_) => Encoding::Unsupported,",
            "Some(_) => Encoding::Resolved,",
        ),
        (
            "identity_resolves_certified_encoding",
            "rust/identities.rs",
            "Some(1) => Encoding::Resolved,",
            "Some(1) => Encoding::Legacy,",
        ),
        (
            "identity_returns_alias",
            "rust/identities.rs",
            "cache.get(path).cloned()",
            "Some(path.clone())",
        ),
        (
            "identity_reverses_binding",
            "rust/identities.rs",
            "cache.insert(path, identity);",
            "cache.insert(identity, path);",
        ),
        ("identity_ignores_conflict", "rust/identities.rs", "return String::eq(previous, &identity);", "return true;"),
        ("pending_starts_synced", "rust/pending.rs", "synced: false", "synced: true"),
        (
            "pending_trusts_failed_sync",
            "rust/pending.rs",
            "state.synced = state.synced || success;",
            "state.synced = true;",
        ),
        (
            "pending_forgets_confirmed_sync",
            "rust/pending.rs",
            "state.synced = state.synced || success;",
            "state.synced = success;",
        ),
        ("pending_skips_required_sync", "rust/pending.rs", "    !state.synced\n}", "    false\n}"),
        (
            "finalization_seals_before_stage",
            "rust/finalization.rs",
            "0 => Some(Effect::Stage),",
            "0 => Some(Effect::Seal),",
        ),
        (
            "finalization_skips_seal",
            "rust/finalization.rs",
            "1 if state.standalone => Some(Effect::Seal),",
            "1 if state.standalone => None,",
        ),
        ("finalization_ignores_failure", "rust/finalization.rs", "failed: !success,", "failed: false,"),
        (
            "finalization_accepts_unsealed",
            "rust/finalization.rs",
            "!state.failed && state.completed == if state.standalone { 2 } else { 1 }",
            "!state.failed && state.completed >= 1",
        ),
        (
            "holding_forgets_previous",
            "rust/holding.rs",
            "let mut i = 0usize;",
            "*kept = BTreeSet::new();\n    let mut i = 0usize;",
        ),
        (
            "holding_ignores_additions",
            "rust/holding.rs",
            "kept.insert(additions[i].clone());",
            "let _ = &additions[i];",
        ),
        (
            "environment_holding_order_not_guaranteed",
            "verification/common/verus/environment.rs",
            "vstd::laws_cmp::obeys_cmp::<String>()",
            "true",
        ),
        ("directory_starts_complete", "rust/directory.rs", "completed: 0,", "completed: total,"),
        ("directory_ignores_failure", "rust/directory.rs", "failed: !success,", "failed: false,"),
        (
            "directory_accepts_incomplete",
            "rust/directory.rs",
            "!state.failed && state.completed == state.total\n}",
            "!state.failed\n}",
        ),
        (
            "directory_repeats_barrier",
            "rust/directory.rs",
            "state.completed + 1",
            "state.completed",
        ),
        ("discovery_forgets_required", "rust/discovery.rs", "if explicit || remembered {", "if explicit {"),
        (
            "discovery_ignores_present",
            "rust/discovery.rs",
            "Observation::Present => Decision::Required,",
            "Observation::Present => Decision::Legacy,",
        ),
        (
            "discovery_error_is_absence",
            "rust/discovery.rs",
            "Observation::Failed => Decision::Unavailable,",
            "Observation::Failed => Decision::Legacy,",
        ),
        (
            "discovery_skips_probe",
            "rust/discovery.rs",
            "Observation::Unqueried => Decision::Probe,",
            "Observation::Unqueried => Decision::Legacy,",
        ),
        (
            "replay_unbounded_buffer",
            "rust/replay.rs",
            "    if remaining < 65536 { remaining } else { 65536 }\n",
            "    remaining\n",
        ),
        (
            "replay_keeps_tentative_cache",
            "rust/replay.rs",
            "state.rebuild = state.rebuild || changes;",
            "let _ = changes;",
        ),
        (
            "replay_forgets_pending_rebuild",
            "rust/replay.rs",
            "state.rebuild = state.rebuild || changes;",
            "state.rebuild = changes;",
        ),
        ("replay_accepts_lost_prefix", "rust/replay.rs", "    position >= state.durable\n", "    true\n"),
        ("replay_regresses_durable", "rust/replay.rs", "if position < state.durable {", "if false {"),
        ("replay_forgets_barrier", "rust/replay.rs", "state.durable = position;", "let _ = position;"),
        (
            "reactivation_without_verification",
            "rust/protocol.rs",
            "if !verified || statuses.is_empty() {",
            "if statuses.is_empty() {",
        ),
        (
            "reactivation_without_source",
            "rust/protocol.rs",
            "if !statuses[i].owned || statuses[i].tombstoned {",
            "if statuses[i].tombstoned {",
        ),
        (
            "reactivation_ignores_tombstone",
            "rust/protocol.rs",
            "if !statuses[i].owned || statuses[i].tombstoned {",
            "if !statuses[i].owned {",
        ),
        (
            "unbounded_verification_buffer",
            "rust/bounds.rs",
            "if length < 1048576 { length } else { 1048576 }",
            "length",
        ),
        (
            "frontier_repeats_node",
            "rust/traversal.rs",
            "if !state.seen.insert(key.clone()) {",
            "state.seen.insert(key.clone()); if false {",
        ),
        (
            "frontier_drops_node",
            "rust/traversal.rs",
            "state.pending.push((key, value));",
            "let _ = (key, value);",
        ),
        (
            "stream_accepts_oversize",
            "rust/stream.rs",
            "count > state.expected - state.seen",
            "count > u64::MAX - state.seen",
        ),
        (
            "stream_forgets_failure",
            "rust/stream.rs",
            "if state.failed || state.seen > state.expected",
            "if state.seen > state.expected",
        ),
        (
            "stream_accepts_short_end",
            "rust/stream.rs",
            "\n    !state.failed && state.seen == state.expected\n",
            "\n    !state.failed\n",
        ),
        (
            "receipt_before_directory_sync",
            "rust/protocol.rs",
            "0 => Some(ReclaimEffect::SyncParents)",
            "0 => Some(ReclaimEffect::RecordReceipt)",
        ),
        (
            "retry_completed_tombstone",
            "rust/reclamation.rs",
            "if marked.insert(path.clone()) {",
            "marked.insert(path.clone()); if true {",
        ),
        (
            "receipt_for_unmarked_pack",
            "rust/reclamation.rs",
            "if !marked.contains(path) {",
            "if false {",
        ),
        (
            "receipt_forgets_pending_removal",
            "rust/reclamation.rs",
            "pending.remove(path);",
            "let _ = path;",
        ),
        (
            "batch_ignores_failed_sync",
            "rust/protocol.rs",
            "if !synced {",
            "if false {",
        ),
        (
            "batch_accepts_wrong_stage",
            "rust/protocol.rs",
            "if !matches!(delete_next(&states[i]), Some(DeleteEffect::SyncParent)) {",
            "if false {",
        ),
        (
            "gc_retry_does_not_restart",
            "verification/gc/verus/gc_liveness.rs",
            "GcEvent::Retry => retry_enabled(s) && n.controller == initial()",
            "GcEvent::Retry => retry_enabled(s) && n.controller == s.controller",
        ),
        (
            "environment_gc_sync_lies",
            "verification/common/verus/environment.rs",
            "n.present == s.present && n.durable_present == s.present",
            "n.present == s.present && n.durable_present == true",
        ),
        (
            "environment_gc_no_retry_fairness",
            "verification/common/verus/environment.rs",
            "exists|i: nat| #![trigger events(i)] i >= start && matches!(events(i), GcEvent::Retry)",
            "true",
        ),
        (
            "environment_gc_errors_forever",
            "verification/common/verus/environment.rs",
            "(matches!(events(i), GcEvent::Io { .. }) ==> events(i)->success)",
            "true",
        ),
        (
            "duplicate_lease",
            "rust/selection.rs",
            "result.push(seen.insert(keys[i]));",
            "seen.insert(keys[i]); result.push(true);",
        ),
        (
            "skip_first_lease",
            "rust/selection.rs",
            "result.push(seen.insert(keys[i]));",
            "result.push(!seen.insert(keys[i]));",
        ),
        ("unchecked_add", "rust/bounds.rs", "if b <= u64::MAX - a {", "if true {"),
        ("missing_payload", "rust/bounds.rs", "add(body, payload)", "Some(body)"),
        ("wrong_slice", "rust/bounds.rs", "edge - position\n", "count\n"),
        ("buffer_overrun", "rust/bounds.rs", "Some((count, consumed + count))", "Some((count, consumed + count + 1))"),
        (
            "wal_before_retain",
            "rust/protocol.rs",
            "0 => Some(CommitEffect::Retain)",
            "0 => Some(CommitEffect::AppendWal)",
        ),
        (
            "consume_before_apply",
            "rust/protocol.rs",
            "2 => Some(CommitEffect::Apply)",
            "2 => Some(CommitEffect::ConsumeStaging)",
        ),
        ("ignore_commit_failure", "rust/protocol.rs", "state.failed || state.completed >= 4", "state.completed >= 4"),
        ("unlink_without_tombstone", "rust/protocol.rs", "state.failed || !state.tombstoned", "state.failed"),
        ("skip_directory_sync", "rust/protocol.rs", "1 => Some(DeleteEffect::SyncParent)", "1 => None"),
        (
            "select_owned_pack",
            "rust/protocol.rs",
            "status.sealed && !status.owned && !status.tombstoned\n",
            "status.sealed && !status.tombstoned\n",
        ),
        ("skip_candidate", "rust/catalog_state.rs", "if crate::protocol::candidate(status) {", "if false {"),
        (
            "model_mark_owned",
            "verification/protocol/verus/control_plane.rs",
            "!live(c, p) && !c.tombstones.contains(p)",
            "!c.tombstones.contains(p)",
        ),
    ):
        source = sources[path]
        if source.count(old) != 1:
            raise ValueError(f"Mutant {name} must change exactly one site")
        yield name, {**sources, path: source.replace(old, new)}, True


def select_cases(configured, requested=None, shard_index=0, shard_count=1):
    """Partition every negative exactly once; every shard checks all positive code."""
    if shard_count <= 0 or not 0 <= shard_index < shard_count:
        raise ValueError("Require 0 <= shard-index < shard-count and shard-count > 0")
    if requested is not None and (shard_count != 1 or shard_index != 0):
        raise ValueError("Named cases cannot be combined with sharding")
    if requested and not set(requested) <= {case[0] for case in configured}:
        raise ValueError("Unknown verification case")
    selected = []
    negative_index = 0
    for case in configured:
        name, _inputs, negative = case
        if not negative:
            selected.append(case)
        elif requested is not None:
            if name in requested:
                selected.append(case)
        else:
            if negative_index % shard_count == shard_index:
                selected.append(case)
            negative_index += 1
    if not selected or not any(not case[2] for case in selected):
        raise ValueError("Every verification run must include the production proof")
    return selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verus", type=Path)
    parser.add_argument("--download", action="store_true")
    parser.add_argument("--tools-dir", type=Path, default=ROOT / "target/verus-tools")
    parser.add_argument("--output", type=Path, default=ROOT / "target/verus")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument(
        "--rlimit",
        type=float,
        default=10,
        help="SMT resource budget; resource exhaustion never counts as a negative success",
    )
    parser.add_argument("--case", action="append", help="Select named cases; the full positive proof always runs")
    parser.add_argument("--shard-index", type=int, default=0, help="Zero-based negative-control shard")
    parser.add_argument(
        "--shard-count", type=int, default=1, help="Partition negatives; every shard runs the full positive proof"
    )
    args = parser.parse_args()
    if args.shard_count <= 0 or not 0 <= args.shard_index < args.shard_count:
        parser.error("Require 0 <= shard-index < shard-count and shard-count > 0")
    if args.case and args.shard_count != 1:
        parser.error("Named cases cannot be combined with sharding")
    executable = download(args.tools_dir) if args.download else args.verus or shutil.which("verus")
    if not executable:
        parser.error("Provide --verus, put verus on PATH, or pass --download")
    executable = Path(executable).resolve()
    version = subprocess.run([str(executable), "--version"], text=True, capture_output=True, check=True, timeout=30)
    version_text = version.stdout + version.stderr
    if VERSION not in version_text:
        raise ValueError(f"Expected Verus {VERSION}, got {version_text}")
    sources = {path: (ROOT / path).read_text() for path in SOURCES}
    if re.search(r"\b(assume|assume_specification|admit|external_body)\b", "\n".join(sources.values())):
        raise ValueError("Production proof module must contain no assumptions or omitted bodies")
    sources[JSON_SPECS] = (ROOT / JSON_SPECS).read_text()
    audit_json_specs(sources[JSON_SPECS])
    sources[COLLECTION_SPECS] = (ROOT / COLLECTION_SPECS).read_text()
    audit_collection_specs(sources[COLLECTION_SPECS])
    sources[FILE_SPECS] = (ROOT / FILE_SPECS).read_text()
    audit_file_specs(sources[FILE_SPECS])
    sources[CATALOG_SPECS] = (ROOT / CATALOG_SPECS).read_text()
    audit_catalog_specs(sources[CATALOG_SPECS])
    sources[JOURNAL_SPECS] = (ROOT / JOURNAL_SPECS).read_text()
    audit_journal_specs(sources[JOURNAL_SPECS])
    registry = load_environment()
    environment = environment_metadata("Verus", registry)
    report = {
        **environment,
        "version": version_text.strip(),
        "journal_os_primitive": {
            "specification": JOURNAL_SPECS,
            "sha256": JOURNAL_SPEC_SHA256,
            "functions": ["open"],
            "opaque_types": ["Path", "PathBuf"],
            "state": "one opened-file permission and path/mode binding",
        },
        "catalog_os_primitives": {
            "specification": CATALOG_SPECS,
            "sha256": CATALOG_SPEC_SHA256,
            "functions": sorted(CATALOG_PRIMITIVES),
            "state": "initial file permission and cloned-lock flag; exclusion requires acquisition",
        },
        "native_io_primitives": {
            "specification": FILE_SPECS,
            "sha256": FILE_SPEC_SHA256,
            "functions": sorted(FILE_PRIMITIVES),
            "opaque_types": 4,
            "state": "non-copy tracked permission, distinct from File identity",
        },
        "collection_library": {
            "specification": COLLECTION_SPECS,
            "methods": sorted(COLLECTION_METHODS),
            "opaque_types": 2,
            "trusted_laws": ["btree_bulk"],
        },
        "archive_sha256": SHA256,
        "source_sha256": {path: hashlib.sha256(code.encode()).hexdigest() for path, code in sources.items()},
        "scope": "actual Rust 2024 catalog JSON wire decoding under audited serde_json primitives, joint catalog indexes/cursor/recovery state, typed frame histories and ordered apply, owner/GC snapshots, staging-graph validation, bounds/controllers and lease filtering; unbounded catalog safety and conditional progress models; shared byte replay with exact format errors and valid-log completeness under Device/Target and shared append/catalog-confirmation composition under Writer contracts; catalog Target raw-history projection, actual typed apply and rebuild/replay/barrier/confirmation composition; actual commit effect dispatch and application quarantine under explicit CommitEffects contracts; actual multi-pack unlink/unique-parent barrier/receipt dispatch under GC Effects contracts; actual queue JSON validation, replay reset, transaction reconstruction and State recovery/append/quarantine under parser and Device/Writer contracts; actual JSON-field/namespace-to-operation construction under Resolver and KeySet contracts; actual raw catalog parsing/run binding and Target business-history interpretation under serde/Resolver/KeySet contracts; actual request receipt lookup/update with exact cached-null/conflict semantics and arbitrary finite admitted receipt-history induction; actual catalog input-field/run serialization and append-to-history composition under the audited Value serialization relation; actual consumer/batch/checkpoint/retention JSON edits and final request receipts, exact successful prefixes on error and no-mutation invalid paths; actual queue event validation/serialization-to-durable-frame composition and finite-history observation equivalence under Value library laws; unbounded multi-queue/request/holder/staging/GC composition model and receipt-preserving history induction at explicit durable-effect boundaries; native request admission/key/hash and full Coordinator composition, path/bulk-set/I/O adapters and complete transaction/queue/GC refinement remain excluded",
        "cases": [],
        "passed": False,
    }
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    dependency, serde_dependency, report["serde_json_dependency"] = json_dependency(args.output)
    shutil.copy2(MANIFEST, args.output / MANIFEST.name)
    configured = list(cases(sources))
    selected = select_cases(configured, args.case, args.shard_index, args.shard_count)
    report["requested_cases"] = args.case
    report["shard"] = {"index": args.shard_index, "count": args.shard_count}
    report["configured_cases"] = [case[0] for case in configured]
    report["selected_cases"] = [case[0] for case in selected]
    for name, inputs, negative in selected:
        directory = args.output / name
        for path, code in inputs.items():
            (directory / path).parent.mkdir(parents=True, exist_ok=True)
            (directory / path).write_text(code)
        command = [
            str(executable),
            "--crate-type=lib",
            "--rlimit",
            str(args.rlimit),
            "--edition=2024",
            "--cfg",
            "straw_verus",
            "--extern",
            f"serde_json={dependency}",
            "--extern",
            f"serde={serde_dependency}",
            "-L",
            f"dependency={dependency.parent}",
            "verification/common/verus/verus.rs",
        ]
        if negative:
            # One semantic failure suffices; do not search for extra consequences
            # after a deliberately broken invariant has already been rejected.
            command.extend(["--multiple-errors", "0"])
        verification_filter = NEGATIVE_FUNCTIONS.get(name)
        verification_module = None
        if verification_filter:
            verification_module = NEGATIVE_MODULES.get(name, "retry_liveness" if "control_" in name else "gc_liveness")
            command.extend(["--verify-only-module", verification_module, "--verify-function", verification_filter])
        started = time.monotonic()
        timeout = False
        try:
            result = subprocess.run(command, cwd=directory, text=True, capture_output=True, timeout=args.timeout)
            output, status = result.stdout + result.stderr, result.returncode
        except subprocess.TimeoutExpired as error:
            timeout = True
            output, status = str(error), None
        (directory / "verus.log").write_text(output)
        summary = re.search(r"verification results:: (\d+) verified, (\d+) errors", output)
        verified, errors = map(int, summary.groups()) if summary else (0, 0)
        semantic_error = bool(
            re.search(
                r"(?:pre|post)condition not satisfied|arithmetic underflow/overflow|invariant not satisfied|assertion failed",
                output,
            )
        )
        resource_failure = bool(re.search(r"Resource limit|rlimit.*exceeded|out of memory", output, re.I))
        passed = (
            not timeout
            and not resource_failure
            and bool(summary)
            and (
                status != 0 and errors > 0 and semantic_error
                if negative
                else status == 0 and verified >= 69 and errors == 0
            )
        )
        report["cases"].append(
            {
                "name": name,
                "verification_filter": verification_filter,
                "verification_module": verification_module,
                "command": command,
                "case_kind": (
                    "environment_negative"
                    if name.startswith("environment_")
                    else "algorithm_negative" if negative else "positive"
                ),
                "expected_failure": negative,
                "verified": verified,
                "errors": errors,
                "resource_failure": resource_failure,
                "exit_status": status,
                "seconds": time.monotonic() - started,
                "source_sha256": {path: hashlib.sha256(code.encode()).hexdigest() for path, code in inputs.items()},
                "environment_sha256": {
                    path: hashlib.sha256(inputs[path].encode()).hexdigest()
                    for path in environment["environment_sha256"]
                },
                "passed": passed,
            }
        )
        print(f"{'PASS' if passed else 'FAIL'} {name}: {verified} verified, {errors} errors", flush=True)
        if not passed:
            print(output)
    report["passed"] = all(case["passed"] for case in report["cases"])
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
