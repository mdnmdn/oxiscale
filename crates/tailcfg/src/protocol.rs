//! Wire-protocol constants from `_docs/05-protocol-reference.md`.

use std::time::Duration;

/// HTTP route: Noise public key discovery.
pub const PATH_KEY: &str = "/key";

/// HTTP route: TS2021 Noise handshake upgrade.
pub const PATH_TS2021: &str = "/ts2021";

/// HTTP route: node registration (inside Noise HTTP/2 tunnel).
pub const PATH_MACHINE_REGISTER: &str = "/machine/register";

/// HTTP route: long-poll network map stream (inside Noise HTTP/2 tunnel).
pub const PATH_MACHINE_MAP: &str = "/machine/map";

/// `Upgrade` header token for the TS2021 handshake.
///
/// Note: "TS2021" is only the informal protocol name and URL path; it never
/// appears on the wire. The real `Upgrade` value sent by clients is
/// `tailscale-control-protocol` (see [05-protocol-reference.md] §3.2). The
/// server only requires the header to be present and non-empty.
pub const UPGRADE_TS2021: &str = "tailscale-control-protocol";

/// EarlyNoise frame magic: the 5 bytes `\xff\xff\xffTS` (`0xFF 0xFF 0xFF 'T'
/// 'S'`). Deliberately not a valid HTTP/2 frame header prefix so the client
/// can read 9 bytes and disambiguate EarlyNoise from the HTTP/2 preface.
/// Matches Headscale's `earlyPayloadMagic` (see [05-protocol-reference.md] §4.3).
pub const EARLY_NOISE_MAGIC: [u8; 5] = [0xFF, 0xFF, 0xFF, b'T', b'S'];

/// Long-poll keepalive interval sent to connected clients.
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(50);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_noise_magic_is_the_five_byte_ts_marker() {
        assert_eq!(&EARLY_NOISE_MAGIC, b"\xff\xff\xffTS");
        assert_eq!(EARLY_NOISE_MAGIC.len(), 5);
    }

    #[test]
    fn upgrade_token_is_the_wire_value_not_the_protocol_nickname() {
        assert_eq!(UPGRADE_TS2021, "tailscale-control-protocol");
        assert_ne!(UPGRADE_TS2021, "TS2021");
    }
}
