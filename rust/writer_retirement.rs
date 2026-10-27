//! Ordered retirement after acquiring one incarnation's exclusive gate.
//! State flags record confirmations in this attempt, not the absence of prior
//! durable effects. A retry reconfirms existing fence/seal/receipt effects.

pub trait Effects {
    type Error;

    fn fence(&mut self) -> Result<(), Self::Error>;

    fn drain(&mut self) -> Result<(), Self::Error>;

    fn reconcile(&mut self) -> Result<(), Self::Error>;

    fn seal(&mut self) -> Result<(), Self::Error>;

    fn record(&mut self) -> Result<(), Self::Error>;
}

pub fn execute<E: Effects>(effects: &mut E) -> Result<(), E::Error> {
    effects.fence()?;
    effects.drain()?;
    effects.reconcile()?;
    effects.seal()?;
    effects.record()
}

pub fn valid_writer_name(bytes: &[u8]) -> bool {
    if bytes.len() != 32 {
        return false;
    }
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return false;
        }
        index += 1;
    }
    true
}

/// Exact durable-operation evidence is immutable after registration. The
/// evidence parameter contains its request key, owner identity and dependency
/// descriptors; the result parameter holds complete publication descriptors.
#[derive(Clone)]
pub struct Operation<D, R> {
    pub sequence: u64,
    pub evidence: D,
    pub results: Option<R>,
    pub done: bool,
}

pub fn operation_position<D, R>(operations: &[Operation<D, R>], sequence: u64) -> Option<usize> {
    // Registration admits strictly increasing WAL sequence numbers.
    operations
        .binary_search_by_key(&sequence, |op| op.sequence)
        .ok()
}

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

pub fn remember_results<D, R>(operation: &mut Operation<D, R>, results: R, matches: bool) -> bool {
    if operation.done || !matches {
        return false;
    }
    if operation.results.is_none() {
        operation.results = Some(results);
    }
    true
}

pub fn settle_operation<D, R>(operation: &mut Operation<D, R>) {
    operation.done = true;
}

#[derive(Clone)]
pub struct Evidence<D> {
    pub key: String,
    pub owner: Option<String>,
    pub dependencies: D,
}

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
