//! Actual catalog critical-section entry, recovery, callback and exit dispatch.
//! Lock primitives, refresh/callback adapters and namespace binding are separate.

pub type Identity = (u64, u64);

pub trait Handle {
    type Error;

    fn identity(&self) -> Result<Identity, Self::Error>;
}

pub trait Lock {
    type Error;

    fn acquire(&mut self) -> Result<(), Self::Error>;

    fn release(&mut self) -> Result<(), Self::Error>;
}

pub trait Effects {
    type Error;
    type Output;
    type Handle: Handle<Error = Self::Error>;
    type Lock: Lock<Error = Self::Error>;

    fn check_binding(&self) -> Result<(), Self::Error>;

    fn active(&self) -> bool;

    fn expected(&self) -> Identity;

    fn clone_lock(&self) -> Result<Self::Lock, Self::Error>;

    fn open_fresh(&self) -> Result<Self::Handle, Self::Error>;

    fn install(&mut self, fresh: Self::Handle);

    fn refresh(&mut self, lock: &Self::Lock) -> Result<(), Self::Error>;

    fn set_active(&mut self, active: bool);

    fn invoke(&mut self, lock: &Self::Lock) -> Result<Self::Output, Self::Error>;

    fn check_context(&self) -> Result<(), Self::Error>;
}

pub enum Failure<E> {
    Effect(E),
    Nested,
    Identity {
        expected: Identity,
        observed: Identity,
    },
}

fn effect<T, E>(value: Result<T, E>) -> Result<T, Failure<E>> {
    match value {
        Ok(value) => Ok(value),
        Err(error) => Err(Failure::Effect(error)),
    }
}

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
