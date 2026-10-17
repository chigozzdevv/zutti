#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("unsupported encoding version")]
    Version,
    #[error("truncated canonical encoding")]
    Truncated,
    #[error("trailing bytes in canonical encoding")]
    Trailing,
    #[error("length exceeds the supported bound")]
    Length,
    #[error("amount arithmetic overflow")]
    Overflow,
    #[error("asset mismatch")]
    Asset,
    #[error("operating-system entropy unavailable")]
    Entropy,
}

pub type Result<T> = core::result::Result<T, Error>;
