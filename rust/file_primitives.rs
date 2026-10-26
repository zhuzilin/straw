//! Narrow standard I/O contracts. Mutable filesystem state belongs to a tracked
//! permission, never to the immutable logical value of a std File handle.
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Take, Write};
#[cfg(straw_verus)]
use vstd::prelude::*;

pub type Buffered<'a> = BufReader<Take<&'a mut File>>;
#[cfg(straw_verus)]
verus! {
    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExFile(File);
    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExError(std::io::Error);
    #[verifier::external_type_specification]
    #[verifier::external_body]
    #[verifier::accept_recursive_types(R)]
    pub struct ExBufReader<R: ?Sized>(BufReader<R>);
    #[verifier::external_type_specification]
    #[verifier::external_body]
    #[verifier::accept_recursive_types(R)]
    pub struct ExTake<R>(Take<R>);

    pub tracked struct Permission { state: crate::wal_source::FileView }
    impl Permission {
        pub closed spec fn view(self) -> crate::wal_source::FileView { self.state }
    }
    pub uninterp spec fn identity(file: File) -> int;
    pub uninterp spec fn buffered_identity(file: Buffered) -> int;
    #[verifier::prophetic]
    pub uninterp spec fn buffered_final_identity(file: Buffered) -> int;
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires permission@@.identity == identity(*file),
    ensures result.is_ok() ==> result->Ok_0 == permission@@.bytes.len(),
))]
pub fn length(
    file: &File,
    #[cfg(straw_verus)] permission: &Tracked<Permission>,
) -> std::io::Result<u64> {
    Ok(file.metadata()?.len())
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures final(permission)@@ == old(permission)@@,
        buffered_identity(result) == old(permission)@@.identity,
        buffered_final_identity(result) == identity(*final(file)),
))]
pub fn buffer<'a>(
    file: &'a mut File,
    limit: u64,
    capacity: usize,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> Buffered<'a> {
    BufReader::with_capacity(capacity, file.take(limit))
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == buffered_identity(buffer),
    ensures identity(*result) == old(permission)@@.identity,
        identity(*final(result)) == buffered_final_identity(buffer),
        final(permission)@@.identity == old(permission)@@.identity,
        final(permission)@@.bytes == old(permission)@@.bytes,
        final(permission)@@.durable == old(permission)@@.durable,
))]
pub fn unbuffer<'a>(
    buffer: Buffered<'a>,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> &'a mut File {
    buffer.into_inner().into_inner()
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == old(permission)@@.identity,
        final(bytes)@.len() == old(bytes)@.len(),
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes
            && final(permission)@@.durable == old(permission)@@.durable
            && crate::wal_device::read_matches(old(permission)@@.bytes, old(permission)@@.cursor, old(bytes)@.len(), final(bytes)@)
            && final(permission)@@.cursor == old(permission)@@.cursor + old(bytes)@.len(),
))]
pub fn read(
    file: &mut File,
    bytes: &mut [u8],
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<()> {
    file.read_exact(bytes)
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == buffered_identity(*file),
    ensures buffered_identity(*final(file)) == buffered_identity(*old(file)),
        buffered_final_identity(*final(file)) == buffered_final_identity(*old(file)),
        final(permission)@@.identity == old(permission)@@.identity,
        final(bytes)@.len() == old(bytes)@.len(),
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes
            && final(permission)@@.durable == old(permission)@@.durable
            && crate::wal_device::read_matches(old(permission)@@.bytes, old(permission)@@.cursor, old(bytes)@.len(), final(bytes)@)
            && final(permission)@@.cursor == old(permission)@@.cursor + old(bytes)@.len(),
))]
pub fn read_buffered(
    file: &mut Buffered<'_>,
    bytes: &mut [u8],
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<()> {
    file.read_exact(bytes)
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file), end <= old(permission)@@.bytes.len(),
    ensures final(permission)@@.identity == old(permission)@@.identity,
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes.take(end as int),
))]
pub fn truncate(
    file: &File,
    end: u64,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<()> {
    file.set_len(end)
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == old(permission)@@.identity,
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes
            && final(permission)@@.durable == old(permission)@@.durable
            && final(permission)@@.cursor == offset,
))]
pub fn seek(
    file: &mut File,
    offset: u64,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<u64> {
    file.seek(SeekFrom::Start(offset))
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == old(permission)@@.identity,
        result.is_ok() ==> final(permission)@@ == old(permission)@@ && result->Ok_0 == old(permission)@@.cursor,
))]
pub fn position(
    file: &mut File,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<u64> {
    file.stream_position()
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file), old(permission)@@.cursor == old(permission)@@.bytes.len(),
    ensures identity(*final(file)) == identity(*old(file)),
        final(permission)@@.identity == old(permission)@@.identity,
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes + bytes@
            && final(permission)@@.cursor == final(permission)@@.bytes.len()
            && final(permission)@@.durable == old(permission)@@.durable,
))]
pub fn write(
    file: &mut File,
    bytes: &[u8],
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<()> {
    file.write_all(bytes)
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(permission)@@.identity == identity(*file),
    ensures final(permission)@@.identity == old(permission)@@.identity,
        result.is_ok() ==> final(permission)@@.bytes == old(permission)@@.bytes
            && final(permission)@@.cursor == old(permission)@@.cursor
            && final(permission)@@.durable == final(permission)@@.bytes.len(),
))]
pub fn sync(
    file: &File,
    #[cfg(straw_verus)] permission: &mut Tracked<Permission>,
) -> std::io::Result<()> {
    file.sync_all()
}
