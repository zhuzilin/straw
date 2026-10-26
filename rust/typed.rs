//! Bind the caller's typed descriptor before allocating or reading its payload.
//! JSON decoding and the caller's shape/product validation are adapter duties.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
pub struct Layout {
    pub shape: Vec<u64>,
    pub dtype: String,
    pub length: u64,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures result == (actual.shape@ == expected.shape@ &&
        actual.dtype@ == expected.dtype@ && actual.length == expected.length),
))]
pub fn matches(actual: &Layout, expected: &Layout) -> bool {
    if actual.length != expected.length
        || !String::eq(&actual.dtype, &expected.dtype)
        || actual.shape.len() != expected.shape.len()
    {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(straw_verus, verus_spec(
        invariant i <= actual.shape.len(), actual.shape.len() == expected.shape.len(),
            actual.dtype@ == expected.dtype@, actual.length == expected.length,
            forall|j: int| 0 <= j < i ==> actual.shape[j] == expected.shape[j],
        decreases actual.shape.len() - i,
    ))]
    while i < actual.shape.len() {
        if actual.shape[i] != expected.shape[i] {
            return false;
        }
        i += 1;
    }
    #[cfg(straw_verus)]
    proof! { assert(actual.shape@ =~= expected.shape@); }
    true
}
