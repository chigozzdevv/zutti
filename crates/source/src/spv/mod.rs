mod block;
mod chain;
mod header;
mod policy;
mod window;
mod witness;

pub use block::parse_block;
pub use chain::HeaderClaim;
pub use header::{HEADER_BYTES, parse_header};
pub use policy::{MAX_HEADERS, MAX_WINDOW_BLOCKS, Policy};
pub use window::{Branch, WindowClaim, WindowTerms, WindowWitness};
pub use witness::{Evaluation, MAX_WITNESS, Witness};

use crate::Error as SourceError;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Encoding(#[from] zutti_protocol::Error),
    #[error("SPV policy or predecessor does not match")]
    Policy,
    #[error("SPV witness exceeds its permitted bounds")]
    Bounds,
    #[error("SPV history does not cover the exact payment window")]
    Coverage,
    #[error("SPV history contradicts the requested outcome")]
    Outcome,
    #[error("SPV history lacks the required confirmation descendants")]
    Confirmations,
    #[error("legacy transaction encoding is ambiguous with a Merkle internal node")]
    AmbiguousMerkleLeaf,
}

pub type Result<T> = core::result::Result<T, Error>;
