#[cfg(test)]
extern crate self as zutti_wallet;

#[cfg(feature = "native")]
pub mod backup;
#[cfg(feature = "native")]
pub mod payment;
#[cfg(feature = "pool")]
pub mod pool;
pub mod preparation;
pub mod prepare;
#[cfg(feature = "note-crypto")]
pub mod private_note;
#[cfg(feature = "native")]
pub mod swap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] zutti_protocol::Error),
    #[error("invalid or unsupported Zcash PCZT")]
    Pczt,
    #[error("payment does not match the agreed profile or terms")]
    Payment,
    #[error("payment already contains a real spending authorization")]
    AlreadyAuthorized,
    #[error("invalid, incomplete or mismatched payment signatures")]
    Signatures,
    #[error("payment journal is not in the required state")]
    PaymentState,
    #[error("Zcash transaction proofs or authorizations are missing or invalid")]
    Transaction,
    #[error("broadcast response does not match the saved transaction")]
    Broadcast,
    #[error("invalid recovery archive or authentication failure")]
    Archive,
    #[error("recovery file permissions or type are unsafe")]
    Permissions,
    #[error("recovery I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "source authorization requires an admitted Ethereum finality verifier and preparation proof"
    )]
    AuthorizationUnavailable,
}
pub type Result<T> = core::result::Result<T, Error>;
