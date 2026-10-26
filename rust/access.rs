//! Admit a local operation only with a non-poisoned exclusive access token.
//! The adapter must obtain that token with a nonblocking mutex attempt.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub enum Access<T> {
    Granted(T),
    Busy,
    Poisoned,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (if poisoned { Access::Poisoned }
        else { match token { Some(value) => Access::Granted(value), None => Access::Busy } }),
))]
pub fn decide<T>(token: Option<T>, poisoned: bool) -> Access<T> {
    if poisoned {
        return Access::Poisoned;
    }
    match token {
        Some(value) => Access::Granted(value),
        None => Access::Busy,
    }
}
