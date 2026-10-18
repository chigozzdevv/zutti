#[cfg(feature = "blocks")]
mod access_index;
#[cfg(feature = "blocks")]
pub mod auth;
#[cfg(feature = "authenticated")]
pub mod authenticated;
#[cfg(feature = "blocks")]
pub mod authorization;
#[cfg(feature = "blocks")]
pub mod block;
#[cfg(feature = "blocks")]
pub mod bootstrap;
#[cfg(feature = "blocks")]
pub mod chain_history;
#[cfg(feature = "authenticated")]
pub mod checkpoint;
pub mod consensus;
#[cfg(feature = "blocks")]
pub mod difficulty;
pub mod history;
pub mod index;
#[cfg(feature = "blocks")]
mod phgr;
#[cfg(feature = "blocks")]
pub mod proof;
pub mod query;
#[cfg(feature = "blocks")]
pub mod range;
mod sparse;
#[cfg(feature = "blocks")]
pub mod spends;
#[cfg(feature = "blocks")]
pub mod state;
#[cfg(feature = "storage")]
pub mod storage;
#[cfg(feature = "blocks")]
pub mod subsidy;
#[cfg(feature = "blocks")]
pub mod transaction;
#[cfg(feature = "blocks")]
pub mod transition;
#[cfg(feature = "blocks")]
pub mod trees;
#[cfg(feature = "blocks")]
pub mod update;
#[cfg(feature = "blocks")]
pub mod upgrade;
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
    #[error("block exceeds a shielded action limit")]
    ShieldedActions,
    #[error("history path does not match its context and root")]
    Path,
    #[error("history root lacks admitted consensus evidence")]
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
    #[error("the source profile does not support the active network upgrade")]
    Upgrade,
    #[error("invalid proof-of-work target or insufficient work")]
    Work,
    #[error("invalid Equihash solution")]
    Equihash,
    #[error("invalid authorizing-data block commitment")]
    AuthorizingCommitment,
}
pub type Result<T> = core::result::Result<T, Error>;
