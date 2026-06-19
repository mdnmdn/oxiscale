//! TS2021 Noise IK handshake, responder (server) side.
//!
//! Builds on the reused crypto: [`ts_noise::ik::ReceivedHandshake`] does the IK
//! responder maths, and [`ts_control_noise::BiCodec`] provides the post-
//! handshake `Record` framing. This module parses the controlbase initiation,
//! emits the response, and exposes the encrypted channel as
//! [`AsyncRead`] + [`AsyncWrite`] for the HTTP/2 server to drive.

use base64::Engine as _;
use bytes::BytesMut;
use tailcfg::EARLY_NOISE_MAGIC;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::Framed;
use ts_control_noise::{BiCodec, MessageType};
use ts_noise::ik::{ReceivedHandshake, SentHandshake};
use tskey::{ChallengePublicKey, MachinePrivateKey, MachinePublicKey};

use crate::error::NoiseError;
use crate::framed_io::FramedIo;

/// Length of the Noise IK initiation payload — the ciphertext carrying
/// `e, es, s, ss` + payload tag (96 bytes; same as [`SentHandshake::INIT_SIZE`]).
const INIT_PAYLOAD_LEN: usize = SentHandshake::INIT_SIZE;
/// controlbase message header: 1-byte type + 2-byte BE length.
const CONTROLBASE_HEADER_LEN: usize = 3;
/// The initiation message (alone) is prefixed by a 2-byte capability version.
const CAPVER_PREFIX_LEN: usize = 2;

/// The encrypted, `Record`-framed Noise transport, surfaced as a byte stream.
pub type NoiseStream<T> = FramedIo<Framed<T, BiCodec>, BytesMut>;

/// Outcome of a successful responder handshake.
pub struct Accepted<T> {
    /// The encrypted byte stream; hand this to the HTTP/2 server.
    pub stream: NoiseStream<T>,
    /// Capability version the client advertised in the initiation.
    pub capability_version: u16,
    /// The client's long-lived machine (Noise static) public key.
    pub peer_machine_key: MachinePublicKey,
    /// Server-generated per-handshake challenge public key for the EarlyNoise
    /// frame. Phase 1 only emits it; node-key proof verification (phase 2) will
    /// require retaining the matching secret here.
    pub node_key_challenge: ChallengePublicKey,
}

/// Accept a TS2021 Noise IK handshake as the responder.
///
/// `initiation_b64` is the base64 value of the client's `X-Tailscale-Handshake`
/// header — controlbase msg1: `[capver u16 BE][type+len header][96-byte payload]`.
/// `socket` is the hijacked connection; msg2 is written to it, after which the
/// returned [`NoiseStream`] carries encrypted application data.
pub async fn accept<T>(
    mut socket: T,
    server_private: &MachinePrivateKey,
    initiation_b64: &str,
) -> Result<Accepted<T>, NoiseError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let raw = base64::engine::general_purpose::STANDARD.decode(initiation_b64.trim())?;

    // Parse the controlbase initiation: [capver(2)][type(1)][len(2 BE)][payload].
    if raw.len() < CAPVER_PREFIX_LEN + CONTROLBASE_HEADER_LEN + INIT_PAYLOAD_LEN {
        return Err(NoiseError::BadInitiation("initiation too short"));
    }
    let capability_version = u16::from_be_bytes([raw[0], raw[1]]);
    let min = tailcfg::capver::MIN_SUPPORTED_CAPABILITY_VERSION;
    if capability_version < min {
        return Err(NoiseError::UnsupportedVersion {
            got: capability_version,
            min,
        });
    }
    if raw[2] != MessageType::Initiation as u8 {
        return Err(NoiseError::BadInitiation("not an initiation message"));
    }
    let body_len = u16::from_be_bytes([raw[3], raw[4]]) as usize;
    if body_len != INIT_PAYLOAD_LEN {
        return Err(NoiseError::BadInitiation(
            "unexpected initiation payload length",
        ));
    }
    let payload_start = CAPVER_PREFIX_LEN + CONTROLBASE_HEADER_LEN;
    let mut payload = raw[payload_start..payload_start + INIT_PAYLOAD_LEN].to_vec();

    // Run the Noise IK responder. The prologue binds the capability version, so
    // it must match exactly what the client mixed in (§4.1).
    let prologue = format!("Tailscale Control Protocol v{capability_version}");
    let my_static: x25519_dalek::StaticSecret = server_private.into();
    let received = ReceivedHandshake::new(&mut payload, prologue.as_bytes(), &my_static)
        .ok_or(NoiseError::HandshakeFailed)?;
    let peer_machine_key = MachinePublicKey::from(received.peer_static_pub.to_bytes());

    // Generate msg2 and derive the transport session.
    let mut resp = [0u8; ReceivedHandshake::RESP_SIZE];
    let session = received.finish(&mut resp);

    // Write msg2 framed as a controlbase Response: [type=0x02][len BE][resp].
    let len_be = (ReceivedHandshake::RESP_SIZE as u16).to_be_bytes();
    let header = [MessageType::Response as u8, len_be[0], len_be[1]];
    socket.write_all(&header).await?;
    socket.write_all(&resp).await?;
    socket.flush().await?;

    // Per-handshake challenge keypair. The secret is unused in phase 1 (no
    // node-key proof verification yet), so only the public half is retained.
    let challenge_secret = x25519_dalek::StaticSecret::random();
    let node_key_challenge =
        ChallengePublicKey::from(x25519_dalek::PublicKey::from(&challenge_secret).to_bytes());

    tracing::debug!(%peer_machine_key, capability_version, "noise handshake accepted");

    let stream = FramedIo::new(Framed::new(socket, BiCodec::from(session)));
    Ok(Accepted {
        stream,
        capability_version,
        peer_machine_key,
        node_key_challenge,
    })
}

/// Write a `tailcfg.EarlyNoise` frame into the encrypted stream:
/// `[5-byte magic][4-byte BE length][JSON]` (§4.3). Optional on the wire, but
/// Headscale always sends it, so we do too.
pub async fn write_early_noise<S>(
    stream: &mut S,
    frame: &tailcfg::EarlyNoise,
) -> Result<(), NoiseError>
where
    S: AsyncWrite + Unpin,
{
    let json = serde_json::to_vec(frame)?;
    let mut buf = Vec::with_capacity(EARLY_NOISE_MAGIC.len() + 4 + json.len());
    buf.extend_from_slice(&EARLY_NOISE_MAGIC);
    buf.extend_from_slice(&(json.len() as u32).to_be_bytes());
    buf.extend_from_slice(&json);
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tailcfg::{CapabilityVersion, EarlyNoise};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use ts_control_noise::Handshake;
    use tskey::MachineKeyPair;

    fn prologue_for(capver: CapabilityVersion) -> String {
        format!("Tailscale Control Protocol v{}", u16::from(capver))
    }

    #[tokio::test]
    async fn responder_completes_handshake_and_round_trips_encrypted_data() {
        let server = MachineKeyPair::new();
        let client = MachineKeyPair::new();
        let capver = CapabilityVersion::V113;
        let prologue = prologue_for(capver);

        // The in-process tailscale-rs initiator produces msg1 (base64).
        let (handshake, init_b64) =
            Handshake::initialize(&prologue, &client.private, &server.public, capver);

        let (client_io, server_io) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            let mut accepted = accept(server_io, &server.private, &init_b64)
                .await
                .expect("responder accepts handshake");
            assert_eq!(accepted.capability_version, 113);
            // The responder learns the client's static machine key from msg1.
            assert_eq!(accepted.peer_machine_key, client.public);

            accepted
                .stream
                .write_all(b"hello-from-server")
                .await
                .unwrap();
            accepted.stream.flush().await.unwrap();

            let mut reply = [0u8; 11];
            accepted.stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"hi-from-cli");
        });

        // The initiator completes the handshake over the same socket.
        let mut client_stream = handshake
            .complete(client_io, &client.private)
            .await
            .expect("initiator completes handshake");

        let mut greeting = [0u8; 17];
        client_stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(&greeting, b"hello-from-server");

        client_stream.write_all(b"hi-from-cli").await.unwrap();
        client_stream.flush().await.unwrap();

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn early_noise_frame_is_magic_then_be_len_then_json() {
        let server = MachineKeyPair::new();
        let client = MachineKeyPair::new();
        let capver = CapabilityVersion::V113;
        let prologue = prologue_for(capver);
        let (handshake, init_b64) =
            Handshake::initialize(&prologue, &client.private, &server.public, capver);

        let (client_io, server_io) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            let mut accepted = accept(server_io, &server.private, &init_b64).await.unwrap();
            let frame = EarlyNoise {
                node_key_challenge: accepted.node_key_challenge,
            };
            write_early_noise(&mut accepted.stream, &frame)
                .await
                .unwrap();
            accepted.node_key_challenge
        });

        let mut client_stream = handshake
            .complete(client_io, &client.private)
            .await
            .unwrap();

        // Read the 9-byte header (5 magic + 4 BE length) from the decrypted stream.
        let mut head = [0u8; 9];
        client_stream.read_exact(&mut head).await.unwrap();
        assert_eq!(&head[..5], &EARLY_NOISE_MAGIC, "EarlyNoise magic");
        let json_len = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) as usize;

        let mut json = vec![0u8; json_len];
        client_stream.read_exact(&mut json).await.unwrap();
        let parsed: EarlyNoise = serde_json::from_slice(&json).unwrap();

        let server_challenge = server_task.await.unwrap();
        assert_eq!(parsed.node_key_challenge, server_challenge);
    }

    #[tokio::test]
    async fn capability_version_below_minimum_is_rejected() {
        let server = MachineKeyPair::new();
        let client = MachineKeyPair::new();
        // V112 is one below the supported floor (113); the initiation carries it
        // in cleartext, so the server rejects before running the crypto.
        let capver = CapabilityVersion::V112;
        let prologue = prologue_for(capver);
        let (_handshake, init_b64) =
            Handshake::initialize(&prologue, &client.private, &server.public, capver);

        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let _keep = client_io;
        let result = accept(server_io, &server.private, &init_b64).await;
        assert!(matches!(
            result,
            Err(NoiseError::UnsupportedVersion { got: 112, min: 113 })
        ));
    }

    #[tokio::test]
    async fn garbage_initiation_is_rejected() {
        let server = MachineKeyPair::new();
        let (client_io, server_io) = tokio::io::duplex(1024);
        drop(client_io);
        let result = accept(server_io, &server.private, "not-base64-at-all!!").await;
        assert!(matches!(result, Err(NoiseError::Base64(_))));
    }
}
