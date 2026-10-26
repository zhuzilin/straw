//! Actual root selection, restricted preflight, durable retirement and owner names.
use crate::closure_node::Digest;
use crate::file_adapter::{Library, NativeFile};

use crate::journal_state::State;
use crate::wal_sink::Hooks;
use serde_json::Value;
use std::fs::File;

/// No successful-effect assumption: this callback cannot borrow queue or WAL state.
/// Its native implementation owns only catalog/preflight bookkeeping and fault hooks.
pub trait Preflight<E> {
    fn prepare(&mut self) -> Result<(), E>;
}

pub enum Error<D, E> {
    Roots(crate::queue_roots::Error<D>),
    Progress(crate::queue_progress::Error<E>),
    Preflight(E),
    Closed,
    Retire(crate::queue_retirement::Error<E>),
}

pub(crate) fn prepare<
    'a,
    'p,
    L: Library,
    D: Digest,
    H: Hooks<NativeFile<'a, L>> + Preflight<L::Error>,
>(
    file: &'a mut Option<File>,
    journal: &mut State,
    business: &mut Value,
    progress: &Value,
    digester: &mut D,
    prefix: &'p str,
    mut hooks: H,
) -> Result<crate::queue_gc::Prepared<'p>, Error<D::Error, L::Error>> {
    let roots = match crate::queue_roots::collect(digester, business, progress) {
        Ok(roots) => roots,
        Err(error) => return Err(Error::Roots(error)),
    };
    if let Err(error) = hooks.prepare() {
        return Err(Error::Preflight(error));
    }
    let file = match file.as_mut() {
        Some(file) => file,
        None => return Err(Error::Closed),
    };
    let retired = match crate::journal_io::retire::<L, H>(file, journal, business, roots, hooks) {
        Ok(retired) => retired,
        Err(error) => return Err(Error::Retire(error)),
    };
    Ok(crate::queue_gc::prepare(prefix, retired))
}

/// Select progress for the actual training consumer before selecting its GC roots.
pub(crate) fn for_queue<
    'a,
    'p,
    L: Library,
    D: Digest,
    H: Hooks<NativeFile<'a, L>>
        + Preflight<L::Error>
        + crate::queue_progress::Reader<Error = L::Error>,
>(
    file: &'a mut Option<File>,
    journal: &mut State,
    business: &mut Value,
    cache: &mut crate::queue_progress::Cache,
    digester: &mut D,
    prefix: &'p str,
    mut hooks: H,
) -> Result<crate::queue_gc::Prepared<'p>, Error<D::Error, L::Error>> {
    let progress = match crate::queue_progress::training(cache, &mut hooks, business) {
        Ok(progress) => progress,
        Err(error) => return Err(Error::Progress(error)),
    };
    prepare::<L, D, H>(file, journal, business, &progress, digester, prefix, hooks)
}
