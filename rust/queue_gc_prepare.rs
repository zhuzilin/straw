//! Actual root selection, restricted preflight, durable retirement and owner names.
use crate::closure_node::Digest;
use crate::file_adapter::{Library, NativeFile};
#[cfg(straw_verus)]
use crate::file_primitives::{Permission, identity};
use crate::journal_state::State;
use crate::wal_sink::Hooks;
use serde_json::Value;
use std::fs::File;
#[cfg(straw_verus)]
use vstd::prelude::*;

/// No successful-effect assumption: this callback cannot borrow queue or WAL state.
/// Its native implementation owns only catalog/preflight bookkeeping and fault hooks.
#[cfg_attr(straw_verus, verus_verify)]
pub trait Preflight<E> {
    fn prepare(&mut self) -> Result<(), E>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<D, E> {
    Roots(crate::queue_roots::Error<D>),
    Progress(crate::queue_progress::Error<E>),
    Preflight(E),
    Closed,
    Retire(crate::queue_retirement::Error<E>),
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(file).is_some() ==> old(permission)@@.identity == identity(old(file)->Some_0),
        crate::environment::string_hash_contract(), crate::environment::string_order_contract(), crate::environment::string_content_contract(),
    ensures match result {Err(Error::Progress(_))=>false,_=>true}, final(digester)@ == old(digester)@,
        final(file).is_some() == old(file).is_some(),
        final(file).is_some() ==> identity(final(file)->Some_0) == identity(old(file)->Some_0)
            && final(permission)@@.identity == identity(final(file)->Some_0),
        match result { Err(Error::Roots(_)) | Err(Error::Preflight(_)) | Err(Error::Closed) =>
            *final(business) == *old(business) && *final(journal) == *old(journal)
                && *final(file) == *old(file) && final(permission)@@ == old(permission)@@, _ => true },
        result.is_ok() ==> result->Ok_0@.0 == prefix@
            && result->Ok_0@.2.origin@ == (*progress,old(digester)@)
            && result->Ok_0@.1.to_iset() == crate::queue_gc::owner_names(prefix@,crate::queue_roots::live_keys(*old(business),*progress,old(digester)@))
            && (result->Ok_0@.2).selected_for(*old(business)) && (result->Ok_0@.2).selected_for(*final(business))
            && final(file).is_some() && !final(journal).poisoned && final(permission)@@.durable == final(journal).offset
            && exists|events:Value| crate::queue_retirement::transaction(events,result->Ok_0@.2.prefix,result->Ok_0@.2.positions@)
                && crate::journal_append::accepted(*old(journal),*final(journal),events,final(permission)@@.bytes)
                && crate::queue_metadata::outcome(*old(business),*final(business),crate::json_specs::json(events)->Array_0[0],Ok(())),
        match result { Err(Error::Retire(crate::queue_retirement::Error::Before(_)))
            | Err(Error::Retire(crate::queue_retirement::Error::Apply(_)))
            | Err(Error::Retire(crate::queue_retirement::Error::After(_))) => final(journal).poisoned, _ => true },
))]
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
    #[cfg(straw_verus)] permission: &'a mut Tracked<Permission>,
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
    let retired = match crate::journal_io::retire::<L, H>(
        file,
        journal,
        business,
        roots,
        hooks,
        #[cfg(straw_verus)]
        permission,
    ) {
        Ok(retired) => retired,
        Err(error) => return Err(Error::Retire(error)),
    };
    Ok(crate::queue_gc::prepare(prefix, retired))
}

/// Select progress for the actual training consumer before selecting its GC roots.
#[cfg_attr(straw_verus,verus_spec(result =>
    requires old(cache).valid(), old(file).is_some() ==> old(permission)@@.identity == identity(old(file)->Some_0),
        crate::environment::string_hash_contract(),crate::environment::string_order_contract(),crate::environment::string_content_contract(),
    ensures final(cache).valid(), final(digester)@ == old(digester)@,
        final(file).is_some() == old(file).is_some(),
        final(file).is_some() ==> identity(final(file)->Some_0) == identity(old(file)->Some_0)
            && final(permission)@@.identity == identity(final(file)->Some_0),
        match result {Err(Error::Progress(_)) | Err(Error::Roots(_)) | Err(Error::Preflight(_)) | Err(Error::Closed) =>
            *final(business)==*old(business) && *final(journal)==*old(journal)
                && *final(file)==*old(file) && final(permission)@@==old(permission)@@,_=>true},
        result.is_ok() ==> result->Ok_0@.0==prefix@
            && crate::queue_progress::selected(crate::queue_progress::training_reference(*old(business)),result->Ok_0@.2.origin@.0)
            && result->Ok_0@.2.origin@.1==old(digester)@
            && result->Ok_0@.1.to_iset()==crate::queue_gc::owner_names(prefix@,crate::queue_roots::live_keys(*old(business),result->Ok_0@.2.origin@.0,old(digester)@))
            && (result->Ok_0@.2).selected_for(*old(business)) && (result->Ok_0@.2).selected_for(*final(business))
            && final(file).is_some() && !final(journal).poisoned && final(permission)@@.durable==final(journal).offset
            && exists|events:Value| crate::queue_retirement::transaction(events,result->Ok_0@.2.prefix,result->Ok_0@.2.positions@)
                && crate::journal_append::accepted(*old(journal),*final(journal),events,final(permission)@@.bytes)
                && crate::queue_metadata::outcome(*old(business),*final(business),crate::json_specs::json(events)->Array_0[0],Ok(())),
        match result {Err(Error::Retire(crate::queue_retirement::Error::Before(_)))
            |Err(Error::Retire(crate::queue_retirement::Error::Apply(_)))
            |Err(Error::Retire(crate::queue_retirement::Error::After(_)))=>final(journal).poisoned,_=>true},
))]
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
    #[cfg(straw_verus)] permission: &'a mut Tracked<Permission>,
) -> Result<crate::queue_gc::Prepared<'p>, Error<D::Error, L::Error>> {
    let progress = match crate::queue_progress::training(cache, &mut hooks, business) {
        Ok(progress) => progress,
        Err(error) => return Err(Error::Progress(error)),
    };
    prepare::<L, D, H>(
        file,
        journal,
        business,
        &progress,
        digester,
        prefix,
        hooks,
        #[cfg(straw_verus)]
        permission,
    )
}
