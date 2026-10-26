//! Bind the caller's typed descriptor before allocating or reading its payload.
//! JSON decoding and the caller's shape/product validation are adapter duties.

pub struct Layout {
    pub shape: Vec<u64>,
    pub dtype: String,
    pub length: u64,
}

pub fn matches(actual: &Layout, expected: &Layout) -> bool {
    if actual.length != expected.length
        || !String::eq(&actual.dtype, &expected.dtype)
        || actual.shape.len() != expected.shape.len()
    {
        return false;
    }
    let mut i = 0usize;

    while i < actual.shape.len() {
        if actual.shape[i] != expected.shape[i] {
            return false;
        }
        i += 1;
    }

    true
}
