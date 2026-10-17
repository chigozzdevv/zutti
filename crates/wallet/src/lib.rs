#[cfg(feature = "native")]
pub mod backup;
pub mod preparation;
pub mod prepare;
#[cfg(feature = "native")]
pub mod private_note;

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
