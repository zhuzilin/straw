//! Connect actual queue encoding, byte framing and successful JSON replay.
//! External Value observation laws exclude parser acceptance and full floats.
use crate::journal_state::State;
use serde_json::Value;
use vstd::prelude::*;

verus! {
    pub(crate) proof fn restored_prefix(
        bytes: Seq<u8>, journal: State, value: crate::queue_application::Restored,
    )
        requires
            exists|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, journal.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
                && journal.offset == crate::wal_recovery::run_end(0, raw),
            crate::queue_application::rebuilt(value, journal.transactions@),
        ensures exists|before: crate::queue_history::Business, raw: Seq<Seq<u8>>|
            crate::queue_application::fresh(before)
                && crate::journal_recovery::decoded(raw, journal.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
                && journal.offset == crate::wal_recovery::run_end(0, raw)
                && crate::queue_history::folded(before, value.business(),
                    crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))),
    {
        let raw = choose|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, journal.transactions@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
            && journal.offset == crate::wal_recovery::run_end(0, raw);
        let before = crate::queue_application::rebuilt_fold(value, journal.transactions@);
        assert(crate::queue_application::fresh(before)
            && crate::journal_recovery::decoded(raw, journal.transactions@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
            && journal.offset == crate::wal_recovery::run_end(0, raw)
            && crate::queue_history::folded(before, value.business(),
                crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))));
    }

    /// Compose the postconditions of State::recover and State::apply_business.
    /// The Values are the actual parser results, not an independently reparsed
    /// or observationally equivalent history (in particular for JSON numbers).
    pub(crate) proof fn physical_prefix_business<E>(
        bytes: Seq<u8>, journal: State,
        before: crate::queue_history::Business, after: crate::queue_history::Business,
    )
        requires
            exists|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, journal.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
                && journal.offset == crate::wal_recovery::run_end(0, raw),
            crate::queue_history::recovery_result::<E>(before, after, journal.transactions@, Ok(())),
        ensures
            exists|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, journal.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
                && journal.offset == crate::wal_recovery::run_end(0, raw)
                && crate::queue_history::folded(before, after,
                    crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))),
    {
        let raw = choose|raw: Seq<Seq<u8>>| crate::journal_recovery::decoded(raw, journal.transactions@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
            && journal.offset == crate::wal_recovery::run_end(0, raw);
        crate::queue_history::frames_fold(before, after, crate::queue_history::body_frames(journal.transactions@));
        assert(crate::journal_recovery::decoded(raw, journal.transactions@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, bytes, 0, 0, Seq::empty(), raw)
            && journal.offset == crate::wal_recovery::run_end(0, raw)
            && crate::queue_history::folded(before, after,
                crate::queue_history::flatten(crate::queue_history::body_frames(journal.transactions@))));
    }

    pub(crate) proof fn recovered_frame_observations(
        before: State, after: State, original: Value, bytes: Seq<u8>,
        payload: Vec<u8>, boundary: u64, recovered: Value, depth: nat,
    )
        requires crate::journal_append::accepted(before, after, original, bytes),
            bytes.len() <= u64::MAX,
            crate::wal_recovery::format_result::<(), ()>(crate::framing::Kind::Queue,
                bytes, before.offset as int, before.sequence,
                Ok(crate::wal_recovery::Frame::Complete { payload, end: boundary })),
            crate::json_specs::parses(payload@, recovered),
        ensures boundary == after.offset,
            crate::json_specs::observe(original, depth) == crate::json_specs::observe(recovered, depth),
    {
        let raw = choose|raw: Seq<u8>| crate::journal_append::encoded(original, raw)
            && crate::wal_recovery::complete_frame(crate::framing::Kind::Queue,
                bytes, before.offset as int, before.sequence as nat, raw, after.offset as int);
        crate::wal_recovery::complete_decodes::<(), ()>(crate::framing::Kind::Queue,
            bytes, before.offset as int, before.sequence, raw, after.offset as int,
            Ok(crate::wal_recovery::Frame::Complete { payload, end: boundary }));
        crate::json_specs::value_parse_consistency(raw, original, recovered, depth);
    }

    pub open spec fn encoded_history(original: Seq<Value>, raw: Seq<Seq<u8>>) -> bool {
        original.len() == raw.len() && forall|i: int| 0 <= i < raw.len()
            ==> crate::journal_append::encoded(original[i], raw[i])
    }
    pub open spec fn same_observations(left: Seq<Value>, right: Seq<Value>) -> bool {
        left.len() == right.len() && forall|i: int, depth: nat| 0 <= i < left.len()
            ==> crate::json_specs::observe(left[i], depth) == crate::json_specs::observe(right[i], depth)
    }
    pub proof fn encoded_history_observations(original: Seq<Value>, raw: Seq<Seq<u8>>, recovered: Seq<Value>)
        requires encoded_history(original, raw), crate::journal_recovery::decoded(raw, recovered),
        ensures same_observations(original, recovered),
    {
        assert forall|i: int, depth: nat| 0 <= i < original.len() implies
            crate::json_specs::observe(original[i], depth) == crate::json_specs::observe(recovered[i], depth) by {
            crate::json_specs::value_parse_consistency(raw[i], original[i], recovered[i], depth);
        }
    }
    pub proof fn repeated_recovery_observations(raw: Seq<Seq<u8>>, first: Seq<Value>, second: Seq<Value>)
        requires crate::journal_recovery::decoded(raw, first), crate::journal_recovery::decoded(raw, second),
        ensures same_observations(first, second),
    {
        assert forall|i: int, depth: nat| 0 <= i < first.len() implies
            crate::json_specs::observe(first[i], depth) == crate::json_specs::observe(second[i], depth) by {
            crate::json_specs::value_parse_consistency(raw[i], first[i], second[i], depth);
        }
    }

    pub(crate) proof fn recovered_frame_targets(
        before: State, after: State, original: Value, bytes: Seq<u8>,
        payload: Vec<u8>, boundary: u64, recovered: Value,
    )
        requires crate::journal_append::accepted(before, after, original, bytes),
            bytes.len() <= u64::MAX,
            crate::wal_recovery::format_result::<(), ()>(crate::framing::Kind::Queue,
                bytes, before.offset as int, before.sequence,
                Ok(crate::wal_recovery::Frame::Complete { payload, end: boundary })),
            crate::json_specs::parses(payload@, recovered),
        ensures boundary == after.offset,
            crate::queue_event_equivalence::transaction_targets(original)
                == crate::queue_event_equivalence::transaction_targets(recovered),
    {
        recovered_frame_observations(before, after, original, bytes, payload, boundary, recovered, 0);
        assert forall|depth: nat| crate::json_specs::observe(original, depth)
            == crate::json_specs::observe(recovered, depth) by {
            recovered_frame_observations(before, after, original, bytes, payload, boundary, recovered, depth);
        }
        crate::queue_event_equivalence::transaction_targets_equal(original, recovered);
    }

    pub proof fn recovered_history_targets(original: Seq<Value>, raw: Seq<Seq<u8>>, recovered: Seq<Value>)
        requires encoded_history(original, raw), crate::journal_recovery::decoded(raw, recovered),
        ensures original.len() == recovered.len(),
            forall|i: int| 0 <= i < original.len() ==> crate::queue_event_equivalence::transaction_targets(original[i])
                == crate::queue_event_equivalence::transaction_targets(recovered[i]),
    {
        encoded_history_observations(original, raw, recovered);
        assert forall|i: int| 0 <= i < original.len() implies crate::queue_event_equivalence::transaction_targets(original[i])
            == crate::queue_event_equivalence::transaction_targets(recovered[i]) by {
            crate::queue_event_equivalence::transaction_targets_equal(original[i], recovered[i]);
        }
    }
}
