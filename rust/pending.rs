//! Keep the exact completed-file description across an uncertain sync response.

pub struct Pending<T> {
    pub value: T,
    pub synced: bool,
}

pub fn start<T>(value: T) -> Pending<T> {
    Pending {
        value,
        synced: false,
    }
}

pub fn record_sync<T>(state: &mut Pending<T>, success: bool) {
    state.synced = state.synced || success;
}

pub fn needs_sync<T>(state: &Pending<T>) -> bool {
    !state.synced
}
