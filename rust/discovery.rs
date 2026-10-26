//! GC discovery decisions. File-open responses and shared atomic storage are
//! adapter contracts; an I/O error is not evidence that a pool is unmanaged.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Unqueried,
    Present,
    Absent,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Probe,
    Required,
    Legacy,
    Unavailable,
}

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
