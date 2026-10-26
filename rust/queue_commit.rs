//! The actual owned WAL append and guarded business-application chain.
use crate::file_adapter::{Library, NativeFile};
use crate::journal_owner::{Borrowed, Journal};
use crate::{queue_application, queue_history};
use serde_json::Value;
use std::collections::HashMap;

pub(crate) enum Error<E> {
    Append(crate::journal_owner::Error<E>),
    Apply(queue_application::Error<E>),
}

pub(crate) struct Requests {
    pub roots: Vec<(String, Vec<Value>)>,
    pub references: Vec<Value>,
}

pub(crate) fn reactivation_requests(
    prefix: &str,
    input: HashMap<String, Value>,
) -> Vec<(String, Value)> {
    let mut it = input.into_iter();

    let mut output = Vec::new();

    loop {
        let Some((key, reference)) = it.next() else {
            return output;
        };

        let mut owner = prefix.to_owned();
        owner.push_str(&key);
        output.push((owner, reference));
    }
}

pub(crate) fn restore_event(events: &[Value]) -> Option<usize> {
    let mut index = 0;

    while index < events.len() {
        let event = &events[index];
        let kind = crate::queue_roots::field_value(event, "type").as_str();
        if (kind == Some("ConsumerState") || kind == Some("BatchReady"))
            && crate::queue_roots::field_value(event, "consumer_id").as_str() == Some("training")
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

pub(crate) fn retention_needed(
    retired_prefix: u64,
    retired: &[Value],
    prefix: u64,
    positions: &[Value],
) -> bool {
    if prefix < retired_prefix {
        return true;
    }
    let mut index = 0;

    while index < retired.len() {
        if let Some(position) = retired[index].as_u64()
            && position >= prefix
            && !crate::queue_roots::contains(positions, &retired[index])
        {
            return true;
        }
        index += 1;
    }

    false
}

/// Build each retained owner and its eventual staging root from one traversal.
pub(crate) fn owner_requests(prefix: &str, references: HashMap<String, Value>) -> Requests {
    let mut it = references.into_iter();

    let mut roots = Vec::new();
    let mut refs = Vec::new();

    loop {
        let Some((key, reference)) = it.next() else {
            return Requests {
                roots,
                references: refs,
            };
        };

        let mut owner = prefix.to_owned();
        owner.push_str(&key);

        let one = vec![reference.clone()];
        roots.push((owner, one));
        refs.push(reference);
    }
}

/// Retention and the post-application staging receipt surround this function.
/// This function owns every mutable queue field while its callbacks run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_and_apply<P, L: Library, C: queue_history::Caches, H>(
    journal: &mut Journal<P>,
    state: &mut Value,
    epoch: &mut String,
    order: &mut usize,
    sealed: &mut bool,
    pending: &mut Vec<(String, Vec<Value>)>,
    handoff: Option<(String, Vec<Value>)>,
    events: &Value,
    caches: &mut C,
    hooks: &mut H,
) -> Result<(), Error<L::Error>>
where
    H: queue_application::Hooks<Error = L::Error>,
    for<'a> H: crate::wal_sink::Hooks<NativeFile<'a, L>>,
{
    if let Err(error) = journal.append_with::<L, _>(events, Borrowed(hooks)) {
        return Err(Error::Append(error));
    }
    if let Some(value) = handoff {
        pending.push(value);
    }

    queue_application::arm(&mut journal.state);
    let confirmation = match queue_application::apply(
        &journal.state,
        state,
        epoch,
        order,
        sealed,
        events,
        caches,
        hooks,
    ) {
        Ok(confirmation) => confirmation,
        Err(error) => {
            return Err(Error::Apply(error));
        }
    };
    queue_application::confirm(&mut journal.state, confirmation);

    Ok(())
}
