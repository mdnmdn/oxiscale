//! Machine, node, disco, and challenge key types with the Tailscale text
//! codecs (`mkey:` / `nodekey:` / `discokey:` / `chalpub:`).
//!
//! These are re-exported from [`ts_keys`] (`_refs/tailscale-rs`), the
//! byte-level protocol oracle: the spike in phase 1 confirmed reuse is viable,
//! so `tskey` is a thin facade rather than a hand-port. Key types implement
//! `Display`/`FromStr` (prefix + 64 lowercase hex) and serde.

#![forbid(unsafe_code)]

// Curve25519 key types + text codecs, reused from the Tailscale Rust client.
//
// Note the challenge key: the *client* never holds the private half, so
// `ts_keys` only exposes [`ChallengePublicKey`]. The server generates the
// challenge keypair itself for the EarlyNoise frame (phase 1, `noise` crate).
pub use ts_keys::{
    ChallengePublicKey, DiscoPrivateKey, DiscoPublicKey, MachineKeyPair, MachinePrivateKey,
    MachinePublicKey, NodePrivateKey, NodePublicKey, ParseError,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn machine_public_key_text_codec_round_trips() {
        let pair = MachineKeyPair::new();
        let encoded = pair.public.to_string();
        assert!(encoded.starts_with("mkey:"), "got {encoded}");
        // prefix + ':' + 64 hex chars
        assert_eq!(encoded.len(), "mkey:".len() + 64);
        let decoded = MachinePublicKey::from_str(&encoded).expect("round-trip");
        assert_eq!(decoded, pair.public);
    }

    #[test]
    fn zero_key_formats_as_all_zero_hex() {
        let key = MachinePublicKey::from([0u8; 32]);
        assert_eq!(format!("{key}"), format!("mkey:{}", "00".repeat(32)));
    }

    #[test]
    fn prefixes_match_the_protocol_reference() {
        assert_eq!(MachinePublicKey::KEY_PREFIX, "mkey");
        assert_eq!(NodePublicKey::KEY_PREFIX, "nodekey");
        assert_eq!(DiscoPublicKey::KEY_PREFIX, "discokey");
    }

    #[test]
    fn wrong_prefix_is_rejected() {
        let pair = MachineKeyPair::new();
        // Same-length prefix so length validation passes and the prefix check
        // is what rejects it (`mkey` and `xkey` are both 4 chars).
        let wrong = pair.public.to_string().replacen("mkey", "xkey", 1);
        assert!(matches!(
            MachinePublicKey::from_str(&wrong),
            Err(ParseError::BadPrefix)
        ));
    }

    #[test]
    fn serde_uses_the_text_codec() {
        let pair = MachineKeyPair::new();
        let json = serde_json::to_string(&pair.public).unwrap();
        assert_eq!(json, format!("\"{}\"", pair.public));
        let back: MachinePublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pair.public);
    }
}
