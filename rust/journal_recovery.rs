//! The queue journal's actual JSON validation and replay application.
use crate::wal_device::{Device, Target};
use crate::wal_recovery::{Boundary, Failure};
use serde_json::Value;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub open spec fn event(value: Value) -> bool {
        crate::json_specs::json(value) is Object
            && match crate::json_specs::field(value, "type"@) {
                Some(kind) => crate::json_specs::json(kind) is Text,
                None => false,
            }
    }
    pub open spec fn events(value: Value) -> bool {
        match crate::json_specs::json(value) {
            crate::json_specs::Json::Array(items) => items.len() > 0
                && forall|i: int| 0 <= i < items.len() ==> event(items[i]),
            _ => false,
        }
    }
    pub open spec fn decoded(raw: Seq<Seq<u8>>, transactions: Seq<Value>) -> bool {
        raw.len() == transactions.len() && forall|i: int| 0 <= i < raw.len()
            ==> crate::json_specs::parses(raw[i], transactions[i]) && events(transactions[i])
    }
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == event(*value)))]
fn valid_event(value: &Value) -> bool {
    let Some(kind) = crate::catalog_json::get(value, "type") else {
        return false;
    };
    kind.as_str().is_some()
}

#[cfg_attr(straw_verus, verus_spec(result => ensures result == events(*value)))]
pub fn valid_events(value: &Value) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut index = 0;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= items.len(), items.len() > 0,
            crate::json_specs::json(*value) == crate::json_specs::Json::Array(items@),
            forall|j: int| 0 <= j < index ==> event(items@[j]),
        decreases items.len() - index,
    ))]
    while index < items.len() {
        if !valid_event(&items[index]) {
            #[cfg(straw_verus)]
            proof! { assert(!events(*value)); }
            return false;
        }
        index += 1;
    }
    true
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error {
    Json(serde_json::Error),
    Events,
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Apply {
    transactions: Vec<Value>,
    #[cfg(straw_verus)]
    raw: Ghost<Seq<Seq<u8>>>,
}

#[cfg(straw_verus)]
verus! {
    impl View for Apply {
        type V = Seq<Seq<u8>>;
        closed spec fn view(&self) -> Self::V { self.raw@ }
    }
    impl crate::wal_device::TargetModel for Apply {
        type Loan = ();
        #[verifier::prophetic]
        open spec fn loan(&self) -> () { () }
        type Context = int;
        closed spec fn valid(&self) -> bool { decoded(self@, self.transactions@) }
        closed spec fn complete(&self) -> bool { true }
        closed spec fn context(&self) -> int { 0 }
    }
}

#[cfg_attr(straw_verus, verus_verify)]
impl Target for Apply {
    type Error = Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result.is_err() ==> final(self)@ == old(self)@,
            result == Err(Error::Events) ==> exists|value: Value|
                crate::json_specs::parses(payload@, value) && !events(value),
    ))]
    fn apply(&mut self, payload: &[u8]) -> Result<(), Error> {
        let value = match serde_json::from_slice::<Value>(payload) {
            Ok(value) => value,
            Err(error) => return Err(Error::Json(error)),
        };
        if !valid_events(&value) {
            return Err(Error::Events);
        }
        self.transactions.push(value);
        #[cfg(straw_verus)]
        proof! { self.raw = Ghost(self@.push(payload@)); }
        Ok(())
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(device).valid(),
    ensures final(device).loan() == old(device).loan(), final(device).valid(), final(device)@.identity == old(device)@.identity,
        forall|i: int| 0 <= i < final(transactions)@.len() ==> events(final(transactions)@[i]),
        result.is_ok() ==> result->Ok_0.sequence == final(transactions)@.len()
            && final(device)@.cursor == result->Ok_0.end && !final(device)@.buffered,
        result.is_ok() && writable ==> final(device)@.bytes.len() == result->Ok_0.end
            && final(device)@.durable == result->Ok_0.end,
        result.is_ok() ==> exists|raw: Seq<Seq<u8>>| decoded(raw, final(transactions)@)
            && crate::wal_recovery::replayed(crate::framing::Kind::Queue, old(device)@.bytes,
                0, 0, Seq::empty(), raw)
            && result->Ok_0.end == crate::wal_recovery::run_end(0, raw),
))]
// Use the audited replace contract without adding a generic Default contract.
#[allow(clippy::mem_replace_with_default)]
pub fn recover<D: Device>(
    device: &mut D,
    transactions: &mut Vec<Value>,
    writable: bool,
) -> Result<Boundary, Failure<D::Error, Error>> {
    // Restart from the physical prefix, including after an earlier partial apply.
    transactions.clear();
    let mut target = Apply {
        transactions: std::mem::replace(transactions, Vec::new()),
        #[cfg(straw_verus)]
        raw: verus_exec_expr!(Ghost(Seq::empty())),
    };
    let result = crate::wal_recovery::recover(
        device,
        &mut target,
        crate::framing::Kind::Queue,
        0,
        0,
        0,
        writable,
    );
    #[cfg(straw_verus)]
    proof! {
        assert(decoded(target@, target.transactions@));
        assert forall|i: int| 0 <= i < target.transactions@.len()
            implies events(target.transactions@[i]) by { }
        if result.is_ok() {
            assert(target@.subrange(0, target@.len() as int) =~= target@);
            assert(exists|raw: Seq<Seq<u8>>| decoded(raw, target.transactions@)
                && crate::wal_recovery::replayed(crate::framing::Kind::Queue, old(device)@.bytes,
                    0, 0, Seq::empty(), raw)
                && result->Ok_0.end == crate::wal_recovery::run_end(0, raw));
        }
    }
    *transactions = target.transactions;
    result
}
