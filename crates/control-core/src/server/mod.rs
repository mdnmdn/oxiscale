//! HTTP edge for the control plane.
//!
//! Two layers:
//! 1. **HTTP/1.1 (axum)** — `GET /key` (Noise key discovery) and `POST /ts2021`
//!    (the Noise handshake upgrade). `/ts2021` performs an HTTP Upgrade, hijacks
//!    the connection, and runs [`noise::accept`] on the raw socket.
//! 2. **HTTP/2 inside Noise (hyper)** — after the handshake + EarlyNoise frame,
//!    the encrypted [`noise::NoiseStream`] is handed to a per-connection HTTP/2
//!    server that dispatches `POST /machine/*`.
//!
//! Phase 1 only needs the handshake + HTTP/2 framing to succeed; the
//! `/machine/*` handlers return a placeholder `200 {}` (real register/map land
//! in phases 2–3).

use std::convert::Infallible;

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tailcfg::{EarlyNoise, OverTLSPublicKeyResponse, UPGRADE_TS2021};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tskey::MachineKeyPair;

/// Errors raised while binding/serving or driving a Noise connection.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Listener bind or accept failure.
    #[error("server I/O error")]
    Io(#[from] std::io::Error),
    /// Noise handshake failure on an upgraded connection.
    #[error(transparent)]
    Noise(#[from] noise::NoiseError),
    /// HTTP/2-over-Noise connection error.
    #[error("http/2-over-noise connection error: {0}")]
    Http2(String),
}

/// Shared server state injected into the axum handlers.
#[derive(Clone)]
pub struct ServerState {
    /// The server's long-lived Noise static identity (the `MachineKey`).
    server_key: MachineKeyPair,
}

impl ServerState {
    /// Build server state around the given Noise static keypair.
    pub fn new(server_key: MachineKeyPair) -> Self {
        Self { server_key }
    }
}

/// Build the HTTP/1.1 router (`GET /key`, `POST /ts2021`).
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/key", get(get_key))
        .route("/ts2021", post(ts2021_upgrade))
        .with_state(state)
}

/// Serve on an already-bound listener until shutdown.
pub async fn serve(listener: TcpListener, state: ServerState) -> Result<(), ServerError> {
    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// `GET /key` — advertise the server's Noise static public key.
async fn get_key(State(state): State<ServerState>) -> Json<OverTLSPublicKeyResponse> {
    Json(OverTLSPublicKeyResponse::new(state.server_key.public))
}

/// `POST /ts2021` — HTTP Upgrade into the Noise handshake.
///
/// Validates the headers, replies `101 Switching Protocols`, and hands the
/// hijacked connection to [`run_noise_connection`] on a background task.
async fn ts2021_upgrade(
    State(state): State<ServerState>,
    mut req: axum::extract::Request,
) -> axum::response::Response {
    // The server only requires the Upgrade header to be present and non-empty
    // (the token itself is not inspected — see protocol reference §3.2).
    let has_upgrade = req
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| !v.is_empty());
    if !has_upgrade {
        return (StatusCode::INTERNAL_SERVER_ERROR, "missing upgrade header").into_response();
    }

    let init_b64 = match req
        .headers()
        .get("X-Tailscale-Handshake")
        .and_then(|v| v.to_str().ok())
    {
        Some(v) => v.to_owned(),
        None => return (StatusCode::BAD_REQUEST, "missing handshake header").into_response(),
    };

    let on_upgrade = hyper::upgrade::on(&mut req);
    let server_key = state.server_key;

    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(error = %e, "ts2021 connection upgrade failed");
                return;
            }
        };
        if let Err(e) = run_noise_connection(TokioIo::new(upgraded), server_key, init_b64).await {
            tracing::warn!(error = %e, "ts2021 noise connection ended with error");
        }
    });

    axum::response::Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, UPGRADE_TS2021)
        .body(Body::empty())
        .expect("static 101 response is valid")
}

/// Run the Noise handshake on a hijacked connection, emit EarlyNoise, then serve
/// HTTP/2 inside the encrypted tunnel.
async fn run_noise_connection<I>(
    io: I,
    server_key: MachineKeyPair,
    init_b64: String,
) -> Result<(), ServerError>
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let accepted = noise::accept(io, &server_key.private, &init_b64).await?;
    let mut stream = accepted.stream;

    let early = EarlyNoise {
        node_key_challenge: accepted.node_key_challenge,
    };
    noise::write_early_noise(&mut stream, &early).await?;

    hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(stream), service_fn(handle_machine))
        .await
        .map_err(|e| ServerError::Http2(e.to_string()))?;
    Ok(())
}

/// HTTP/2 handler for `/machine/*` requests inside the Noise tunnel.
///
/// Phase 1 placeholder: acknowledge the request so the client sees working
/// HTTP/2 framing. Registration and map streaming arrive in phases 2–3.
async fn handle_machine(
    req: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    tracing::info!(
        method = %req.method(),
        path = %req.uri().path(),
        "machine request received inside noise tunnel (phase-1 placeholder)"
    );
    let response = hyper::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_static(b"{}")))
        .expect("static response is valid");
    Ok(response)
}
