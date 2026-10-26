//! The single native open operation and its erased descriptor permission.
use crate::journal_owner::Opened;
use std::{fs::OpenOptions, path::PathBuf};
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg(straw_verus)]
verus! {
    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExPathBuf(PathBuf);
    #[verifier::external_type_specification]
    #[verifier::external_body]
    pub struct ExPath(std::path::Path);
}

#[cfg_attr(straw_verus, verus_verify)]
#[cfg_attr(straw_verus, verifier::external_body)]
#[cfg_attr(straw_verus,verus_spec(result =>
    requires !read_only || recover,
    ensures result.is_ok() ==> result->Ok_0.valid()
        && result->Ok_0.location@==crate::journal_owner::destination(*path)
        && result->Ok_0.read_only@==read_only && result->Ok_0.recover@==recover
        && !result->Ok_0.parent_synced@ && !result->Ok_0.file_synced@
        && result->Ok_0.permission@@.cursor==0
        && (!recover ==> result->Ok_0.permission@@.bytes.len()==0 && result->Ok_0.permission@@.durable==0),
))]
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
    Ok(Opened {
        file,
        #[cfg(straw_verus)]
        permission: Tracked::assume_new(),
        #[cfg(straw_verus)]
        location: Ghost::assume_new(),
        #[cfg(straw_verus)]
        read_only: Ghost::new(read_only),
        #[cfg(straw_verus)]
        recover: Ghost::new(recover),
        #[cfg(straw_verus)]
        parent_synced: Ghost::new(false),
        #[cfg(straw_verus)]
        file_synced: Ghost::new(false),
    })
}
