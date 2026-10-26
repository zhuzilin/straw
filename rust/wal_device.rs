//! Independently reviewed replay I/O and application interfaces.
//! Implementations must establish these contracts; trait declarations alone
//! do not verify concrete file adapters or business-state reconstruction.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub struct DeviceView {
        pub identity: int,
        pub bytes: Seq<u8>,
        pub cursor: nat,
        pub durable: nat,
        pub buffered: bool,
    }
    pub trait DeviceModel: View<V = DeviceView> {
        type Loan;
        #[verifier::prophetic]
        spec fn loan(&self) -> Self::Loan;
        open spec fn valid(&self) -> bool { true }
    }
    pub trait TargetModel: View<V = Seq<Seq<u8>>> {
        type Context;
        type Loan;
        #[verifier::prophetic]
        spec fn loan(&self) -> Self::Loan;
        spec fn valid(&self) -> bool;
        /// No partially applied record remains at a successful boundary.
        spec fn complete(&self) -> bool;
        /// Application context that replay must not advance (catalog floor).
        spec fn context(&self) -> Self::Context;
    }
    pub uninterp spec fn digest(bytes: Seq<u8>) -> Seq<u8>;
    /// An empty read succeeds without touching the descriptor, including past EOF.
    pub open spec fn read_matches(bytes: Seq<u8>, cursor: nat, length: nat, result: Seq<u8>) -> bool {
        result.len() == length && (length == 0 || (cursor + length <= bytes.len()
            && result == bytes.subrange(cursor as int, (cursor + length) as int)))
    }
    pub proof fn empty_read_matches_any_cursor(bytes: Seq<u8>, cursor: nat)
        ensures read_matches(bytes, cursor, 0, Seq::<u8>::empty()),
    {}
    pub proof fn nonempty_read_is_bounded(bytes: Seq<u8>, cursor: nat, length: nat, result: Seq<u8>)
        requires length > 0, read_matches(bytes, cursor, length, result),
        ensures cursor + length <= bytes.len(),
            result == bytes.subrange(cursor as int, (cursor + length) as int),
    {}
}

#[cfg(not(straw_verus))]
pub trait DeviceModel {}
#[cfg(not(straw_verus))]
impl<T> DeviceModel for T {}
#[cfg(not(straw_verus))]
pub trait TargetModel {}
#[cfg(not(straw_verus))]
impl<T> TargetModel for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Device: DeviceModel {
    type Error;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.is_ok() ==> result->Ok_0 as int == self@.bytes.len(),
    ))]
    fn length(&self) -> Result<u64, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes,
            result.is_ok() ==> final(self)@.durable == old(self)@.durable,
            final(self)@.buffered == old(self)@.buffered,
            result.is_ok() ==> read_matches(old(self)@.bytes, old(self)@.cursor, length as nat, result->Ok_0@),
            result.is_ok() ==> final(self)@.cursor == old(self)@.cursor + length,
    ))]
    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result@ == digest(first@ + second@), result.len() == 32,
    ))]
    fn hash(&self, first: &[u8], second: &[u8]) -> Vec<u8>;

    #[cfg_attr(straw_verus, verus_spec(
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            final(self)@.bytes == old(self)@.bytes,
            final(self)@.durable == old(self)@.durable,
            !final(self)@.buffered,
    ))]
    fn discard_buffer(&mut self);

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered, end <= old(self)@.bytes.len(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes.take(end as int),
    ))]
    fn truncate(&mut self, end: u64) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered,
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes,
            result.is_ok() ==> final(self)@.durable == final(self)@.bytes.len(),
    ))]
    fn sync(&mut self) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity,
            final(self)@.buffered == old(self)@.buffered,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes,
            result.is_ok() ==> final(self)@.durable == old(self)@.durable,
            result.is_ok() ==> final(self)@.cursor == offset,
    ))]
    fn seek(&mut self, offset: u64) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Target: TargetModel {
    type Error;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), old(self).complete(),
        ensures final(self).valid(), final(self).context() == old(self).context(),
            final(self).loan() == old(self).loan(),
            result.is_ok() ==> final(self).complete(),
            result.is_ok() ==> final(self)@ == old(self)@.push(payload@),
    ))]
    fn apply(&mut self, payload: &[u8]) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub struct Position {
    pub size: u64,
    pub cursor: u64,
}

/// Byte effects used by the shared append implementation. Error replies can
/// follow partial effects; only an Ok sync establishes a durable complete frame.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Writer: DeviceModel {
    type Error;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered,
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@ == old(self)@,
            result.is_ok() ==> result->Ok_0.size == old(self)@.bytes.len()
                && result->Ok_0.cursor == old(self)@.cursor,
    ))]
    fn inspect(&mut self) -> Result<Position, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures result@ == digest(first_span@ + second_span@), result.len() == 32,
    ))]
    fn hash(&self, first_span: &[u8], second_span: &[u8]) -> Vec<u8>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered, old(self)@.cursor == old(self)@.bytes.len(),
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes + part@,
            result.is_ok() ==> final(self)@.cursor == final(self)@.bytes.len(),
            result.is_ok() ==> final(self)@.durable == old(self)@.durable,
    ))]
    fn write(&mut self, part: &[u8]) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).valid(), !old(self)@.buffered,
        ensures final(self).loan() == old(self).loan(), final(self).valid(), final(self)@.identity == old(self)@.identity, !final(self)@.buffered,
            result.is_ok() ==> final(self)@.bytes == old(self)@.bytes
                && final(self)@.cursor == old(self)@.cursor
                && final(self)@.durable == old(self)@.bytes.len(),
    ))]
    fn sync(&mut self) -> Result<(), Self::Error>;
}
