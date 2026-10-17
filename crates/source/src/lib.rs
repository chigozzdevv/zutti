#[cfg(feature = "blocks")]
pub mod auth;
#[cfg(feature = "blocks")]
pub mod block;
#[cfg(feature = "blocks")]
pub mod bootstrap;
#[cfg(feature = "blocks")]
pub mod chain_history;
pub mod consensus;
#[cfg(feature = "blocks")]
pub mod difficulty;
pub mod history;
pub mod index;
pub mod query;
#[cfg(feature = "blocks")]
pub mod state;
#[cfg(feature = "blocks")]
pub mod work;

pub use index::{CandidateIndex, IndexContext, IndexProof, IndexSnapshot, Observation};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] zutti_protocol::Error),
    #[error("blocks must be appended consecutively")]
    Height,
    #[error("invalid or oversized block transaction list")]
    Transactions,
    #[error("history path does not match its context and root")]
    Path,
    #[error("source consensus verifier and genesis bootstrap are not implemented")]
    ConsensusUnavailable,
    #[error("history cannot be used with this payment window")]
    Window,
    #[error("malformed block or invalid transaction Merkle root")]
    Block,
    #[error("block does not extend the candidate tip")]
    Parent,
    #[error("unsupported source network")]
    Network,
    #[error("block does not match the network genesis identity")]
    Genesis,
    #[error("invalid proof-of-work target or insufficient work")]
    Work,
    #[error("invalid Equihash solution")]
    Equihash,
    #[error("invalid authorizing-data block commitment")]
    AuthorizingCommitment,
}
pub type Result<T> = core::result::Result<T, Error>;
