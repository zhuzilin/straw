//! Actual append descriptor dispatch over file and fault-hook primitives.
use crate::framing::Kind;
use crate::wal_device::{Position, Writer};

pub trait File: crate::wal_source::File {
    fn position(&mut self) -> Result<u64, Self::Error>;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    fn sync_all(&mut self) -> Result<(), Self::Error>;
}

pub trait Hooks<F: File> {
    fn point(&mut self, file: &mut F, point: &str) -> Result<(), F::Error>;
}

pub struct Appender<F: File, H: Hooks<F>> {
    file: F,
    kind: Kind,
    hooks: H,
    first: bool,
}

impl<F: File, H: Hooks<F>> Appender<F, H> {
    pub fn from_file(file: F, kind: Kind, hooks: H) -> Self {
        Self {
            file,
            kind,
            hooks,
            first: true,
        }
    }

    pub fn finish(self) {
        self.file.finish();
    }

    /// Begin another append admission on the same borrowed descriptor.
    /// This resets only the once-per-append fault hook; it performs no I/O.
    pub fn restart(&mut self) {
        self.first = true;
    }

    pub fn point(&mut self, point: &str) -> Result<(), F::Error> {
        self.hooks.point(&mut self.file, point)
    }
}

impl<F: File, H: Hooks<F>> Writer for Appender<F, H> {
    type Error = F::Error;

    fn inspect(&mut self) -> Result<Position, Self::Error> {
        Ok(Position {
            size: self.file.length()?,
            cursor: self.file.position()?,
        })
    }

    fn hash(&self, first: &[u8], second: &[u8]) -> Vec<u8> {
        F::hash(first, second)
    }

    fn write(&mut self, part: &[u8]) -> Result<(), Self::Error> {
        if self.first {
            self.hooks.point(
                &mut self.file,
                match self.kind {
                    Kind::Queue => "before_journal_write",
                    Kind::Catalog => "before_storage_catalog_write",
                },
            )?;
            self.first = false;
        }
        self.file.write_all(part)?;
        self.hooks.point(
            &mut self.file,
            match self.kind {
                Kind::Queue => "after_journal_part",
                Kind::Catalog => "after_storage_catalog_part",
            },
        )
    }

    fn sync(&mut self) -> Result<(), Self::Error> {
        match self.kind {
            Kind::Queue => {
                self.hooks.point(&mut self.file, "before_journal_sync")?;
                self.hooks.point(&mut self.file, "before_file_sync")?;
                self.file.sync_all()?;
                self.hooks.point(&mut self.file, "after_journal_sync")
            }
            Kind::Catalog => {
                self.hooks
                    .point(&mut self.file, "before_storage_catalog_sync")?;
                self.file.sync_all()?;
                self.hooks
                    .point(&mut self.file, "after_storage_catalog_sync")
            }
        }
    }
}
