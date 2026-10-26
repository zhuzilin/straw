//! Ordered retirement after acquiring one incarnation's exclusive gate.
//! State flags record confirmations in this attempt, not the absence of prior
//! durable effects. A retry reconfirms existing fence/seal/receipt effects.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[cfg(straw_verus)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Acquired,
    Fenced,
    Drained,
    Reconciled,
    Sealed,
    Recorded,
}

#[cfg(straw_verus)]
verus! {
    pub struct State {
        pub identity: Seq<u8>,
        pub gate_held: bool,
        pub fenced: bool,
        pub drained: bool,
        pub reconciled: bool,
        pub sealed: bool,
        pub recorded: bool,
    }
    pub trait Model: View<V = State> {}
    pub open spec fn safe(s: State) -> bool {
        s.gate_held && (s.drained ==> s.fenced) && (s.reconciled ==> s.drained)
            && (s.sealed ==> s.reconciled) && (s.recorded ==> s.sealed)
    }
    pub open spec fn stage(s: State, phase: Phase) -> bool {
        safe(s) && s.fenced == (phase != Phase::Acquired)
            && s.drained == (phase == Phase::Drained || phase == Phase::Reconciled || phase == Phase::Sealed || phase == Phase::Recorded)
            && s.reconciled == (phase == Phase::Reconciled || phase == Phase::Sealed || phase == Phase::Recorded)
            && s.sealed == (phase == Phase::Sealed || phase == Phase::Recorded)
            && s.recorded == (phase == Phase::Recorded)
    }
}
#[cfg(not(straw_verus))]
pub trait Model {}
#[cfg(not(straw_verus))]
impl<T> Model for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Effects: Model {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires stage(old(self)@, Phase::Acquired),
        ensures final(self)@ == (State { fenced: final(self)@.fenced, ..old(self)@ }),
            result.is_ok() ==> final(self)@.fenced,
    ))]
    fn fence(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires stage(old(self)@, Phase::Fenced),
        ensures final(self)@ == (State { drained: final(self)@.drained, ..old(self)@ }),
            result.is_ok() ==> final(self)@.drained,
    ))]
    fn drain(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires stage(old(self)@, Phase::Drained),
        ensures final(self)@ == (State { reconciled: final(self)@.reconciled, ..old(self)@ }),
            result.is_ok() ==> final(self)@.reconciled,
    ))]
    fn reconcile(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires stage(old(self)@, Phase::Reconciled),
        ensures final(self)@ == (State { sealed: final(self)@.sealed, ..old(self)@ }),
            result.is_ok() ==> final(self)@.sealed,
    ))]
    fn seal(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires stage(old(self)@, Phase::Sealed),
        ensures final(self)@ == (State { recorded: final(self)@.recorded, ..old(self)@ }),
            result.is_ok() ==> final(self)@.recorded,
    ))]
    fn record(&mut self) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires stage(old(effects)@, Phase::Acquired),
    ensures final(effects)@.identity == old(effects)@.identity, safe(final(effects)@),
        result.is_ok() ==> stage(final(effects)@, Phase::Recorded),
))]
pub fn execute<E: Effects>(effects: &mut E) -> Result<(), E::Error> {
    effects.fence()?;
    effects.drain()?;
    effects.reconcile()?;
    effects.seal()?;
    effects.record()
}

#[cfg(straw_verus)]
verus! {
    pub open spec fn writer_name(bytes: Seq<u8>) -> bool {
        bytes.len() == 32 && forall|i: int| 0 <= i < bytes.len()
            ==> (b'0' <= #[trigger] bytes[i] <= b'9') || (b'a' <= bytes[i] <= b'f')
    }
}
#[cfg_attr(straw_verus, verus_spec(result => ensures result == writer_name(bytes@)))]
#[allow(
    clippy::manual_range_contains,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn valid_writer_name(bytes: &[u8]) -> bool {
    if bytes.len() != 32 {
        return false;
    }
    let mut index = 0;
    #[cfg_attr(straw_verus, verus_spec(
        invariant bytes.len() == 32, index <= bytes.len(),
            forall|i: int| 0 <= i < index ==> (b'0' <= #[trigger] bytes@[i] <= b'9') || (b'a' <= bytes@[i] <= b'f'),
        decreases bytes.len() - index,
    ))]
    while index < bytes.len() {
        let byte = bytes[index];
        if !((b'0' <= byte && byte <= b'9') || (b'a' <= byte && byte <= b'f')) {
            return false;
        }
        index += 1;
    }
    true
}

/// Exact durable-operation evidence is immutable after registration. The
/// evidence parameter contains its request key, owner identity and dependency
/// descriptors; the result parameter holds complete publication descriptors.
#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(not(straw_verus), derive(Clone))]
pub struct Operation<D, R> {
    pub sequence: u64,
    pub evidence: D,
    pub results: Option<R>,
    pub done: bool,
}
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_some() ==> result.unwrap() < operations.len(),
        result.is_some() ==> operations[result.unwrap() as int].sequence == sequence,
        result.is_some() ==> forall|i:int| 0 <= i < result.unwrap() ==> (#[trigger] operations[i]).sequence != sequence,
        result.is_none() ==> forall|i:int| 0 <= i < operations.len() ==> (#[trigger] operations[i]).sequence != sequence,
))]
pub fn operation_position<D, R>(operations: &[Operation<D, R>], sequence: u64) -> Option<usize> {
    let mut index = 0;
    #[cfg_attr(straw_verus, verus_spec(
        invariant index <= operations.len(),
            forall|i:int| 0 <= i < index ==> (#[trigger] operations[i]).sequence != sequence,
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if operations[index].sequence == sequence {
            return Some(index);
        }
        index += 1;
    }
    None
}
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (old(operations).len()==0 || old(operations)@[old(operations).len()-1].sequence < sequence),
        result ==> final(operations)@ == old(operations)@.push(Operation {sequence,evidence,results:None,done:false}),
        !result ==> final(operations)@ == old(operations)@,
))]
pub fn register_operation<D, R>(
    operations: &mut Vec<Operation<D, R>>,
    sequence: u64,
    evidence: D,
) -> bool {
    if !operations.is_empty() && operations[operations.len() - 1].sequence >= sequence {
        return false;
    }
    operations.push(Operation {
        sequence,
        evidence,
        results: None,
        done: false,
    });
    true
}
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures final(operation).sequence == old(operation).sequence,
        final(operation).evidence == old(operation).evidence,
        final(operation).done == old(operation).done,
        result == (!old(operation).done && matches),
        result && old(operation).results.is_none() ==> final(operation).results == Some(results),
        !result ==> final(operation).results == old(operation).results,
        old(operation).results.is_some() ==> final(operation).results == old(operation).results,
))]
pub fn remember_results<D, R>(operation: &mut Operation<D, R>, results: R, matches: bool) -> bool {
    if operation.done || !matches {
        return false;
    }
    if operation.results.is_none() {
        operation.results = Some(results);
    }
    true
}
#[cfg_attr(straw_verus, verus_spec(
    ensures final(operation).sequence == old(operation).sequence,
        final(operation).evidence == old(operation).evidence,
        final(operation).results == old(operation).results,
        final(operation).done,
))]
pub fn settle_operation<D, R>(operation: &mut Operation<D, R>) {
    operation.done = true;
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(not(straw_verus), derive(Clone))]
pub struct Evidence<D> {
    pub key: String,
    pub owner: Option<String>,
    pub dependencies: D,
}
#[cfg(straw_verus)]
verus! {
    pub open spec fn named<D>(evidence:Evidence<D>,name:Seq<char>,by_owner:bool)->bool {
        if by_owner {match evidence.owner {Some(owner)=>owner@==name,None=>false}}
        else {evidence.key@==name}
    }
}
#[cfg_attr(straw_verus,verus_spec(result=> ensures result==named(*evidence,name@,by_owner)))]
pub fn named_operation<D>(evidence: &Evidence<D>, name: &str, by_owner: bool) -> bool {
    if by_owner {
        match &evidence.owner {
            Some(owner) => owner.as_str() == name,
            None => false,
        }
    } else {
        evidence.key.as_str() == name
    }
}
#[cfg_attr(straw_verus,verus_spec(
    ensures final(operations).len()==old(operations).len(),
        forall|i:int| 0 <= i < old(operations).len() ==> final(operations)@[i] == (Operation {
            done:old(operations)@[i].done || named(old(operations)@[i].evidence,name@,by_owner),..old(operations)@[i]
        }),
))]
#[allow(
    clippy::ptr_arg,
    reason = "Keep the verified branches, owned-state contracts and mutation-control anchors explicit."
)]
pub fn settle_named<D, R>(
    operations: &mut Vec<Operation<Evidence<D>, R>>,
    name: &str,
    by_owner: bool,
) {
    let mut index = 0;
    #[cfg_attr(straw_verus,verus_spec(
        invariant index <= operations.len(),operations.len()==old(operations).len(),
            forall|i:int| 0 <= i < operations.len() ==> operations@[i] == (Operation {
                done:old(operations)@[i].done || (i<index && named(old(operations)@[i].evidence,name@,by_owner)),..old(operations)@[i]
            }),
        decreases operations.len()-index,
    ))]
    while index < operations.len() {
        if named_operation(&operations[index].evidence, name, by_owner) {
            settle_operation(&mut operations[index]);
        }
        index += 1;
    }
}
