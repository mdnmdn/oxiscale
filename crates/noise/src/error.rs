use thiserror::Error;

/// Errors raised while accepting a TS2021 Noise handshake (responder side).
#[derive(Debug, Error)]
pub enum NoiseError {
    /// The `X-Tailscale-Handshake` value was not valid base64.
    #[error("invalid base64 in handshake initiation")]
    Base64(#[from] base64::DecodeError),

    /// The initiation message was malformed: wrong length, wrong message type,
    /// or an unexpected controlbase header.
    #[error("malformed handshake initiation: {0}")]
    BadInitiation(&'static str),

    /// The Noise IK handshake failed to authenticate (bad keys, tampered
    /// message, or a prologue / capability-version mismatch).
    #[error("noise handshake authentication failed")]
    HandshakeFailed,

    /// The client advertised a capability version below the server's minimum.
    #[error("unsupported capability version {got} (minimum {min})")]
    UnsupportedVersion { got: u16, min: u16 },

    /// Failed to serialize a frame (e.g. EarlyNoise) before sending.
    #[error("frame serialization failed")]
    Serde(#[from] serde_json::Error),

    /// Underlying socket I/O error while writing the response or framing.
    #[error("noise transport I/O error")]
    Io(#[from] std::io::Error),
}
