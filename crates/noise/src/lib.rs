//! TS2021 Noise IK handshake for the control server (responder role).
//!
//! The IK crypto and controlbase framing are reused from
//! `_refs/tailscale-rs` (`ts_noise`, `ts_control_noise`); this crate mirrors
//! the initiator logic onto the server side: parse the `X-Tailscale-Handshake`
//! initiation, emit the response, generate the EarlyNoise challenge, and expose
//! the encrypted channel as [`AsyncRead`](tokio::io::AsyncRead) +
//! [`AsyncWrite`](tokio::io::AsyncWrite).
//!
//! The `GET /key` discovery endpoint and the `POST /ts2021` HTTP upgrade live
//! in the server crate; this crate provides the transport [`accept`] runs on
//! the hijacked socket.

#![forbid(unsafe_code)]

pub mod error;
mod framed_io;
mod responder;

pub use error::NoiseError;
pub use framed_io::FramedIo;
pub use responder::{accept, write_early_noise, Accepted, NoiseStream};
