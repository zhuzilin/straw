//! The single native open operation and its erased descriptor permission.
use crate::journal_owner::Opened;
use std::{fs::OpenOptions, path::PathBuf};

pub(crate) fn open(path: &PathBuf, recover: bool, read_only: bool) -> std::io::Result<Opened> {
    let mut options = OpenOptions::new();
    options.read(true);
    if !read_only {
        options.write(true);
        if !recover {
            options.create_new(true);
        }
    }
    let file = options.open(path)?;
    Ok(Opened { file })
}
