//! The production ancestor-barrier loop, including failure-prefix semantics.
use crate::directory;
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg(straw_verus)]
verus! {
    pub struct State<P> { pub attempted: Seq<P>, pub synced: Set<P> }
    pub trait Model<P>: View<V = State<P>> { type Context; type SuccessContext; spec fn success_context(&self) -> Self::SuccessContext; #[verifier::prophetic] spec fn context(&self) -> Self::Context; open spec fn context_valid(&self) -> bool { true } }
}
#[cfg(not(straw_verus))]
pub trait Model<P> {}
#[cfg(not(straw_verus))]
impl<P, T> Model<P> for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Barrier<P>: Model<P> {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).context_valid(),
        ensures final(self).context_valid(), final(self).context() == old(self).context(), final(self)@.attempted == old(self)@.attempted.push(*path),
            old(self)@.synced.subset_of(final(self)@.synced),
            result.is_ok() ==> final(self).success_context() == old(self).success_context() && final(self)@.synced.contains(*path),
    ))]
    fn sync(&mut self, path: &P) -> Result<(), Self::Error>;
    fn incomplete() -> Self::Error;
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).context_valid(), directories.len() <= u64::MAX,
    ensures final(effects).context_valid(), final(effects).context() == old(effects).context(), old(effects)@.synced.subset_of(final(effects)@.synced),
        result.is_ok() ==> final(effects).success_context() == old(effects).success_context() && final(effects)@.attempted == old(effects)@.attempted + directories@
            && directories@.to_set().subset_of(final(effects)@.synced),
        result.is_err() ==> exists|n: int| 1 <= n <= directories.len()
            && final(effects)@.attempted == old(effects)@.attempted + directories@.subrange(0, n)
            && directories@.subrange(0, n - 1).to_set().subset_of(final(effects)@.synced),
))]
pub fn sync_all<P, E: Barrier<P>>(directories: &[P], effects: &mut E) -> Result<(), E::Error> {
    let mut state = directory::start(directories.len() as u64);
    #[cfg_attr(straw_verus, verus_spec(
        invariant effects.success_context() == old(effects).success_context(), effects.context_valid(), effects.context() == old(effects).context(), state.total == directories.len(), state.completed <= directories.len(), !state.failed,
            old(effects)@.synced.subset_of(effects@.synced),
            effects@.attempted == old(effects)@.attempted + directories@.subrange(0, state.completed as int),
            directories@.subrange(0, state.completed as int).to_set().subset_of(effects@.synced),
        decreases directories.len() - state.completed,
    ))]
    loop {
        let Some(index) = directory::next(&state) else {
            if !directory::finished(&state) {
                return Err(E::incomplete());
            }
            #[cfg(straw_verus)]
            proof! { assert(directories@.subrange(0, state.completed as int) =~= directories@); }
            return Ok(());
        };
        let outcome = effects.sync(&directories[index as usize]);
        state = directory::advance(state, outcome.is_ok());
        #[cfg(straw_verus)]
        proof! {
            assert(directories@.subrange(0, index as int + 1)
                =~= directories@.subrange(0, index as int).push(directories[index as int]));
            let prefix = directories@.subrange(0, index as int);
            prefix.lemma_push_to_set_commute(directories[index as int]);
            assert(directories@.subrange(0, index as int + 1).to_set()
                =~= prefix.to_set().insert(directories[index as int]));
            if outcome.is_err() { assert(exists|n: int| 1 <= n <= directories.len()
                && effects@.attempted == old(effects)@.attempted + directories@.subrange(0, n)
                && directories@.subrange(0, n - 1).to_set().subset_of(effects@.synced)) by {
                    assert(effects@.attempted == old(effects)@.attempted
                        + directories@.subrange(0, index as int + 1));
                }
            }
        }
        outcome?;
    }
}
