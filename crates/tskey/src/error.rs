use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("invalid key encoding")]
    InvalidEncoding,
}
