//! GC discovery decisions. File-open responses and shared atomic storage are
//! adapter contracts; an I/O error is not evidence that a pool is unmanaged.
#[cfg(straw_verus)]
use vstd::prelude::*;

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Unqueried,
    Present,
    Absent,
    Failed,
}

#[cfg_attr(straw_verus, verus_verify)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Probe,
    Required,
    Legacy,
    Unavailable,
}

#[cfg_attr(straw_verus, verus_spec(result =>
    ensures
        (result == Decision::Required) ==
            (explicit || remembered || observation == Observation::Present),
        (result == Decision::Probe) ==
            (!explicit && !remembered && observation == Observation::Unqueried),
        (result == Decision::Legacy) ==
            (!explicit && !remembered && observation == Observation::Absent),
        (result == Decision::Unavailable) ==
            (!explicit && !remembered && observation == Observation::Failed),
))]
pub fn decide(explicit: bool, remembered: bool, observation: Observation) -> Decision {
    if explicit || remembered {
        return Decision::Required;
    }
    match observation {
        Observation::Unqueried => Decision::Probe,
        Observation::Present => Decision::Required,
        Observation::Absent => Decision::Legacy,
        Observation::Failed => Decision::Unavailable,
    }
}
