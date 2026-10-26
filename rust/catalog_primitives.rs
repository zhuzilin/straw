//! Single native open, metadata, clone, lock and unlock operations.
//! The caller holds the stable lock before reading or mutating fresh contents.
use std::fs::{File, OpenOptions};

pub struct Opened {
    pub(crate) file: File,
}

pub fn open(root: &str, create: bool) -> std::io::Result<Opened> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .open(std::path::Path::new(root).join("storage.log"))?;
    Ok(Opened { file })
}

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

impl crate::catalog_transaction::Handle for Opened {
    type Error = std::io::Error;
    fn identity(&self) -> std::io::Result<crate::catalog_transaction::Identity> {
        identity(&self.file)
    }
}

pub struct Lock {
    file: File,
}

pub fn clone_lock(file: &File) -> std::io::Result<Lock> {
    Ok(Lock {
        file: file.try_clone()?,
    })
}

impl crate::catalog_transaction::Lock for Lock {
    type Error = std::io::Error;

    fn acquire(&mut self) -> std::io::Result<()> {
        self.file.lock()
    }

    fn release(&mut self) -> std::io::Result<()> {
        self.file.unlock()
    }
}
