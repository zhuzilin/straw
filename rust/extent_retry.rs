//! Actual finite visibility retry loop, preserving failures and the shared budget.

pub enum Failure<E> {
    Visibility(E),
    Permanent(E),
}

pub enum Error<E> {
    Exhausted(E),
    Permanent(E),
}

pub trait Attempt {
    type Output;
    type Error;
    fn read(&mut self) -> Result<Self::Output, Failure<Self::Error>>;
}

pub trait Observer<E> {
    fn recovered(&mut self, retries: usize);

    fn wait(&mut self, retry: usize, limit: usize, delay: u64, error: &E);
}

pub fn run<A: Attempt, O: Observer<A::Error>>(
    source: &mut A,
    observer: &mut O,
    delays: &[u64],
) -> Result<A::Output, Error<A::Error>> {
    let mut retry = 0usize;

    loop {
        let outcome = source.read();

        match outcome {
            Ok(value) => {
                if retry > 0 {
                    observer.recovered(retry);
                }
                return Ok(value);
            }
            Err(Failure::Permanent(error)) => return Err(Error::Permanent(error)),
            Err(Failure::Visibility(error)) => {
                let Some(delay) = delays.get(retry) else {
                    return Err(Error::Exhausted(error));
                };
                observer.wait(retry + 1, delays.len(), *delay, &error);

                retry += 1;
            }
        }
    }
}

pub fn visibility(kind: &str) -> bool {
    kind == "ExtentVisibility"
}

pub fn exhausted(kind: &mut String) {
    *kind = "CorruptData".to_owned();
}

pub trait Log<E> {
    fn recovered(&mut self, retries: usize);
    fn retry(&mut self, retry: usize, limit: usize, delay: u64, error: &E);
}

pub trait Wait {
    fn wait(&mut self, delay: u64);
}

pub struct Observe<L, W: Wait> {
    log: L,
    wait: W,
}

impl<L, W: Wait> Observe<L, W> {
    pub fn new(log: L, wait: W) -> Self {
        Self { log, wait }
    }
}

impl<E, L: Log<E>, W: Wait> Observer<E> for Observe<L, W> {
    fn recovered(&mut self, retries: usize) {
        self.log.recovered(retries);
    }
    fn wait(&mut self, retry: usize, limit: usize, delay: u64, error: &E) {
        self.log.retry(retry, limit, delay, error);
        self.wait.wait(delay);
    }
}
