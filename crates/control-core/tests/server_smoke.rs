//! In-process phase-1 gate for the HTTP edge.
//!
//! Drives the real server with a low-level `hyper` client + the reused
//! `ts_control_noise` initiator handshake: proves `GET /key`, the `POST /ts2021`
//! HTTP upgrade + connection hijack, the Noise handshake, and the EarlyNoise
//! frame end-to-end without a container. The authoritative `tailscaled` gate
//! runs separately.

use std::net::SocketAddr;

use std::sync::Arc;

use bytes::Bytes;
use control_core::server::{self, ServerState};
use control_core::store::MemoryStore;
use http_body_util::{BodyExt, Empty};
use hyper::client::conn::http1;
use hyper::{header, Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use tailcfg::{CapabilityVersion, EarlyNoise, OverTLSPublicKeyResponse, EARLY_NOISE_MAGIC};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use ts_control_noise::Handshake;
use tskey::MachineKeyPair;

/// Bind an ephemeral port and serve in the background; return the address.
async fn start_server(server_key: MachineKeyPair) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = ServerState::new(server_key, Arc::new(MemoryStore::new()));
    tokio::spawn(async move {
        let _ = server::serve(listener, state).await;
    });
    addr
}

/// Open a low-level HTTP/1.1 client connection (with upgrade support) to `addr`.
async fn connect(addr: SocketAddr) -> http1::SendRequest<Empty<Bytes>> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (sender, conn) = http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });
    sender
}

#[tokio::test]
async fn get_key_advertises_the_server_public_key() {
    let server_key = MachineKeyPair::new();
    let addr = start_server(server_key).await;

    let mut sender = connect(addr).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/key?v=113")
        .header(header::HOST, "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: OverTLSPublicKeyResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed.public_key, server_key.public);
}

#[tokio::test]
async fn ts2021_upgrade_completes_handshake_and_emits_early_noise() {
    let server_key = MachineKeyPair::new();
    let client = MachineKeyPair::new();
    let addr = start_server(server_key).await;

    let capver = CapabilityVersion::V113;
    let prologue = format!("Tailscale Control Protocol v{}", u16::from(capver));
    let (handshake, init_b64) =
        Handshake::initialize(&prologue, &client.private, &server_key.public, capver);

    let mut sender = connect(addr).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/ts2021")
        .header(header::HOST, "localhost")
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "tailscale-control-protocol")
        .header("X-Tailscale-Handshake", &init_b64)
        .body(Empty::<Bytes>::new())
        .unwrap();

    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);

    // Hijack the upgraded connection and finish the Noise handshake over it.
    let upgraded = hyper::upgrade::on(resp).await.unwrap();
    let mut client_stream = handshake
        .complete(TokioIo::new(upgraded), &client.private)
        .await
        .expect("initiator completes handshake against the server");

    // The server emits the EarlyNoise frame: 5-byte magic + u32 BE len + JSON.
    let mut head = [0u8; 9];
    client_stream.read_exact(&mut head).await.unwrap();
    assert_eq!(&head[..5], &EARLY_NOISE_MAGIC[..], "EarlyNoise magic");

    let json_len = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) as usize;
    let mut json = vec![0u8; json_len];
    client_stream.read_exact(&mut json).await.unwrap();
    let early: EarlyNoise = serde_json::from_slice(&json).unwrap();
    assert!(
        early.node_key_challenge.to_string().starts_with("chalpub:"),
        "challenge key uses the chalpub: codec"
    );
}
