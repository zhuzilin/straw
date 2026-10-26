//! The production ancestor-barrier loop, including failure-prefix semantics.
use crate::directory;

pub trait Barrier<P> {
    type Error;

    fn sync(&mut self, path: &P) -> Result<(), Self::Error>;
    fn incomplete() -> Self::Error;
}

pub fn sync_all<P, E: Barrier<P>>(directories: &[P], effects: &mut E) -> Result<(), E::Error> {
    let mut state = directory::start(directories.len() as u64);

    loop {
        let Some(index) = directory::next(&state) else {
            if !directory::finished(&state) {
                return Err(E::incomplete());
            }

            return Ok(());
        };
        let outcome = effects.sync(&directories[index as usize]);
        state = directory::advance(state, outcome.is_ok());

        outcome?;
    }
}
