//! Single native open, metadata, clone, lock and unlock operations.
//! The caller holds the stable lock before reading or mutating fresh contents.
use std::fs::{File, OpenOptions};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    pub uninterp spec fn inode_for(identity:int) -> crate::catalog_transaction::Identity;
    pub open spec fn inode(file: File) -> crate::catalog_transaction::Identity {
        inode_for(crate::file_primitives::identity(file))
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Opened {
    pub(crate) file: File,
    #[cfg(straw_verus)]
    pub(crate) permission: Tracked<crate::file_primitives::Permission>,
}
#[cfg(straw_verus)]
verus! {
    impl Opened {
        pub closed spec fn state(&self)->crate::wal_source::FileView { self.permission@@ }
        pub closed spec fn valid(&self)->bool {
            self.permission@@.identity == crate::file_primitives::identity(self.file)
        }
    }
    impl View for Opened {
        type V = crate::catalog_transaction::Identity;
        closed spec fn view(&self)->Self::V { inode(self.file) }
    }
    impl crate::catalog_transaction::HandleModel for Opened { open spec fn valid(&self)->bool { Opened::valid(self) } }
}
#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() ==> result->Ok_0.valid() && result->Ok_0.state().cursor == 0,
))]
pub fn open(root: &str, create: bool) -> std::io::Result<Opened> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .open(std::path::Path::new(root).join("storage.log"))?;
    Ok(Opened {
        file,
        #[cfg(straw_verus)]
        permission: Tracked::assume_new(),
    })
}
#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result => ensures result.is_ok() ==> result->Ok_0 == inode(*file)))]
pub fn identity(file: &File) -> std::io::Result<crate::catalog_transaction::Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Catalog locking requires POSIX file identities",
        ))
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl crate::catalog_transaction::Handle for Opened {
    type Error = std::io::Error;
    fn identity(&self) -> std::io::Result<crate::catalog_transaction::Identity> {
        identity(&self.file)
    }
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Lock {
    file: File,
    #[cfg(straw_verus)]
    held: Ghost<bool>,
}
#[cfg(straw_verus)]
verus! {
    impl View for Lock {
        type V=crate::catalog_transaction::LockView;
        closed spec fn view(&self)->Self::V {
            crate::catalog_transaction::LockView { identity:inode(self.file), held:self.held@ }
        }
    }
    impl crate::catalog_transaction::LockModel for Lock {}
}
#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result.is_ok() ==> result->Ok_0@.identity == inode(*file) && !result->Ok_0@.held,
))]
pub fn clone_lock(file: &File) -> std::io::Result<Lock> {
    Ok(Lock {
        file: file.try_clone()?,
        #[cfg(straw_verus)]
        held: Ghost::assume_new(),
    })
}
#[cfg_attr(straw_verus, verus_verify)]
impl crate::catalog_transaction::Lock for Lock {
    type Error = std::io::Error;
    #[cfg_attr(straw_verus, verifier::external_body)]
    fn acquire(&mut self) -> std::io::Result<()> {
        self.file.lock()
    }
    #[cfg_attr(straw_verus, verifier::external_body)]
    fn release(&mut self) -> std::io::Result<()> {
        self.file.unlock()
    }
}
#[cfg(straw_verus)]
verus! {
    impl Opened {
        pub(crate) proof fn facts(&self)
            ensures self@ == inode(self.file), self.valid() ==
                (self.permission@@.identity == crate::file_primitives::identity(self.file)),
        {}
    }
}
