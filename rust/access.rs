//! Admit a local operation only with a non-poisoned exclusive access token.
//! The adapter must obtain that token with a nonblocking mutex attempt.

pub enum Access<T> {
    Granted(T),
    Busy,
    Poisoned,
}

pub fn decide<T>(token: Option<T>, poisoned: bool) -> Access<T> {
    if poisoned {
        return Access::Poisoned;
    }
    match token {
        Some(value) => Access::Granted(value),
        None => Access::Busy,
    }
}
