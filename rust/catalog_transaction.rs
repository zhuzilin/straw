//! Actual catalog critical-section entry, recovery, callback and exit dispatch.
//! Lock primitives, refresh/callback adapters and namespace binding are separate.
#[cfg(straw_verus)]
use vstd::prelude::*;

pub type Identity = (u64, u64);

#[cfg(straw_verus)]
verus! {
    pub struct State {
        pub bound: bool,
        pub active: bool,
        pub ready: bool,
        pub expected: Identity,
        pub current: Identity,
        pub refreshed: bool,
        pub calls: nat,
    }
    pub struct LockView { pub identity: Identity, pub held: bool }
    pub trait HandleModel: View<V = Identity> { spec fn valid(&self) -> bool; }
    pub trait LockModel: View<V = LockView> {}
    pub trait Model: View<V = State> {
        spec fn resource_valid(&self) -> bool;
        type Loan;
        #[verifier::prophetic]
        spec fn loan(&self) -> Self::Loan;
    }
    pub open spec fn valid(s: State) -> bool { s.current == s.expected }
    pub open spec fn protected(s: State, lock: LockView) -> bool {
        s.bound && valid(s) && lock.held && lock.identity == s.current
    }
}

#[cfg(not(straw_verus))]
pub trait HandleModel {}
#[cfg(not(straw_verus))]
impl<T> HandleModel for T {}
#[cfg(not(straw_verus))]
pub trait LockModel {}
#[cfg(not(straw_verus))]
impl<T> LockModel for T {}
#[cfg(not(straw_verus))]
pub trait Model {}
#[cfg(not(straw_verus))]
impl<T> Model for T {}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Handle: HandleModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.valid(),
        ensures result.is_ok() ==> result->Ok_0 == self@,
    ))]
    fn identity(&self) -> Result<Identity, Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Lock: LockModel {
    type Error;
    #[cfg_attr(straw_verus, verus_spec(result =>
        ensures final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> final(self)@.held,
    ))]
    fn acquire(&mut self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self)@.held,
        ensures final(self)@.identity == old(self)@.identity,
            result.is_ok() ==> !final(self)@.held,
    ))]
    fn release(&mut self) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Effects: Model {
    type Error;
    type Output;
    type Handle: Handle<Error = Self::Error>;
    type Lock: Lock<Error = Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result => requires self.resource_valid(), ensures result.is_ok() == self@.bound))]
    fn check_binding(&self) -> Result<(), Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result => requires self.resource_valid(), ensures result == self@.active))]
    fn active(&self) -> bool;
    #[cfg_attr(straw_verus, verus_spec(result => requires self.resource_valid(), ensures result == self@.expected))]
    fn expected(&self) -> Identity;
    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.resource_valid(),
        ensures result.is_ok() ==> result->Ok_0@.identity == self@.current,
    ))]
    fn clone_lock(&self) -> Result<Self::Lock, Self::Error>;
    #[cfg_attr(straw_verus, verus_spec(result => requires self.resource_valid(), ensures result.is_ok() ==> result->Ok_0.valid()))]
    fn open_fresh(&self) -> Result<Self::Handle, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(
        requires old(self).resource_valid(), fresh.valid(), fresh@ == old(self)@.expected,
        ensures final(self).loan() == old(self).loan(), final(self).resource_valid(), final(self)@ == (State { current: fresh@, refreshed: false, ..old(self)@ }),
    ))]
    fn install(&mut self, fresh: Self::Handle);

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(), protected(old(self)@, lock@), !old(self)@.active,
        ensures final(self).loan() == old(self).loan(), final(self).resource_valid(), final(self)@ == (State { ready: final(self)@.ready,
            refreshed: result.is_ok(), ..old(self)@ }),
            result.is_ok() ==> final(self)@.ready,
    ))]
    fn refresh(&mut self, lock: &Self::Lock) -> Result<(), Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(
        requires old(self).resource_valid(), active ==> old(self)@.bound && old(self)@.ready && old(self)@.refreshed,
        ensures final(self).loan() == old(self).loan(), final(self).resource_valid(), final(self)@ == (State { active, ..old(self)@ }),
    ))]
    fn set_active(&mut self, active: bool);

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires old(self).resource_valid(), protected(old(self)@, lock@), old(self)@.active, old(self)@.ready,
            old(self)@.refreshed, old(self)@.calls == 0,
        ensures final(self).loan() == old(self).loan(), final(self).resource_valid(), valid(final(self)@), final(self)@.calls == old(self)@.calls + 1,
            final(self)@.active ==> final(self)@.expected == old(self)@.expected
                && final(self)@.refreshed,
    ))]
    fn invoke(&mut self, lock: &Self::Lock) -> Result<Self::Output, Self::Error>;

    #[cfg_attr(straw_verus, verus_spec(result =>
        requires self.resource_valid(),
        ensures result.is_ok() == (self@.bound && self@.active && self@.ready),
    ))]
    fn check_context(&self) -> Result<(), Self::Error>;
}

#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Effect(E),
    Nested,
    Identity {
        expected: Identity,
        observed: Identity,
    },
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures match value {
        Ok(value) => result == Ok(value),
        Err(error) => result == Err(Failure::Effect(error)),
    },
))]
fn effect<T, E>(value: Result<T, E>) -> Result<T, Failure<E>> {
    match value {
        Ok(value) => Ok(value),
        Err(error) => Err(Failure::Effect(error)),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == match operation {
        Err(error) => Err(error),
        Ok(value) => match unlocked {
            Ok(()) => Ok(value),
            Err(error) => Err(Failure::Effect(error)),
        },
    },
))]
fn finish<T, E>(
    operation: Result<T, Failure<E>>,
    unlocked: Result<(), E>,
) -> Result<T, Failure<E>> {
    match operation {
        Ok(value) => {
            effect(unlocked)?;
            Ok(value)
        }
        Err(error) => Err(error),
    }
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).resource_valid(), protected(old(effects)@, lock@), !old(effects)@.active,
        old(effects)@.calls == 0,
    ensures final(effects).loan() == old(effects).loan(), final(effects).resource_valid(), valid(final(effects)@), final(effects)@.calls <= 1,
        result.is_ok() ==> final(effects)@.calls == 1 && final(effects)@.active
            && final(effects)@.bound && final(effects)@.ready && final(effects)@.refreshed
            && final(effects)@.expected == old(effects)@.expected,
        match result {
            Err(Failure::Nested) => false,
            Err(Failure::Identity { expected, observed }) => expected == old(effects)@.expected
                && expected != observed && final(effects)@ == old(effects)@,
            _ => true,
        },
))]
fn locked<E: Effects>(effects: &mut E, lock: &E::Lock) -> Result<E::Output, Failure<E::Error>> {
    let fresh = effect(effects.open_fresh())?;
    let expected = effects.expected();
    let observed = effect(fresh.identity())?;
    if !crate::identities::same_log(expected, observed) {
        return Err(Failure::Identity { expected, observed });
    }
    effects.install(fresh);
    effect(effects.refresh(lock))?;
    effects.set_active(true);
    let value = effect(effects.invoke(lock))?;
    effect(effects.check_context())?;
    Ok(value)
}

#[cfg_attr(straw_verus, verus_spec(result =>
    requires old(effects).resource_valid(), valid(old(effects)@), old(effects)@.calls == 0, !old(effects)@.refreshed,
    ensures final(effects).loan() == old(effects).loan(), final(effects).resource_valid(), valid(final(effects)@), final(effects)@.calls <= 1,
        !old(effects)@.active ==> !final(effects)@.active,
        !old(effects)@.bound || old(effects)@.active ==> final(effects)@ == old(effects)@,
        result.is_ok() ==> final(effects)@.calls == 1 && final(effects)@.bound
            && final(effects)@.ready && final(effects)@.refreshed
            && final(effects)@.expected == old(effects)@.expected,
        match result {
            Err(Failure::Nested) => old(effects)@.bound && old(effects)@.active,
            Err(Failure::Identity { expected, observed }) => expected == old(effects)@.expected
                && expected != observed && final(effects)@ == old(effects)@,
            _ => true,
        },
))]
pub fn execute<E: Effects>(effects: &mut E) -> Result<E::Output, Failure<E::Error>> {
    effect(effects.check_binding())?;
    if effects.active() {
        return Err(Failure::Nested);
    }
    let mut lock = effect(effects.clone_lock())?;
    effect(lock.acquire())?;
    let result = locked(effects, &lock);
    effects.set_active(false);
    let unlocked = lock.release();
    finish(result, unlocked)
}
