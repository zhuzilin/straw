//! Independently reviewed replay I/O and application interfaces.
//! Implementations must establish these contracts; trait declarations alone
//! do not verify concrete file adapters or business-state reconstruction.

pub trait Device {
    type Error;

    fn length(&self) -> Result<u64, Self::Error>;

    fn read(&mut self, length: usize) -> Result<Vec<u8>, Self::Error>;

    fn hash(&self, first: &[u8], second: &[u8]) -> Vec<u8>;

    fn discard_buffer(&mut self);

    fn truncate(&mut self, end: u64) -> Result<(), Self::Error>;

    fn sync(&mut self) -> Result<(), Self::Error>;

    fn seek(&mut self, offset: u64) -> Result<(), Self::Error>;
}

pub trait Target {
    type Error;

    fn apply(&mut self, payload: &[u8]) -> Result<(), Self::Error>;
}

pub struct Position {
    pub size: u64,
    pub cursor: u64,
}

/// Byte effects used by the shared append implementation. Error replies can
/// follow partial effects; only an Ok sync establishes a durable complete frame.
pub trait Writer {
    type Error;

    fn inspect(&mut self) -> Result<Position, Self::Error>;

    fn hash(&self, first_span: &[u8], second_span: &[u8]) -> Vec<u8>;

    fn write(&mut self, part: &[u8]) -> Result<(), Self::Error>;

    fn sync(&mut self) -> Result<(), Self::Error>;
}
