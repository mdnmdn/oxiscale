use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store not implemented")]
    NotImplemented,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(String),
}
