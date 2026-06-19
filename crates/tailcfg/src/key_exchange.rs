//! Server-emitted types for the Noise key-discovery and handshake phase.
//!
//! These two structs are *not* part of [`ts_control_serde`] (which models the
//! client's view); the control server emits them, so we define them here with
//! the exact JSON casing the real Go server produces.
//!
//! Byte-fidelity: the casing below follows [05-protocol-reference.md] §3.1/§4.3
//! — Go marshals these structs with no explicit JSON tags, yielding PascalCase
//! field names. Confirmed at the phase-1 gate: a real `tailscaled` (capver 138)
//! fetched `/key` and completed the handshake against these exact bytes. The
//! unit tests below pin the JSON to guard against regressions.

use serde::{Deserialize, Serialize};
use tskey::{ChallengePublicKey, MachinePublicKey};

/// Response body of `GET /key` — `tailcfg.OverTLSPublicKeyResponse`.
///
/// The client fetches the server's long-lived Noise static key before starting
/// the TS2021 handshake. A modern client uses [`Self::public_key`];
/// [`Self::legacy_public_key`] is a deprecated pre-Noise control key (usually
/// the zero key) retained for compatibility — it must still be present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct OverTLSPublicKeyResponse {
    /// Deprecated pre-Noise control key. Emitted as the zero `mkey:` when unset.
    pub legacy_public_key: MachinePublicKey,
    /// The server's current Noise static public key (the `MachineKey`).
    pub public_key: MachinePublicKey,
}

impl OverTLSPublicKeyResponse {
    /// Build a response advertising `public_key`, with the legacy key zeroed.
    pub fn new(public_key: MachinePublicKey) -> Self {
        Self {
            legacy_public_key: MachinePublicKey::default(),
            public_key,
        }
    }
}

/// Optional frame the server writes into the encrypted channel immediately
/// after the handshake — `tailcfg.EarlyNoise`.
///
/// Carries a per-handshake challenge public key the client must prove
/// ownership of its node key against, preventing node-key impersonation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EarlyNoise {
    /// Temporary Curve25519 public key (`chalpub:`) the node signs against.
    pub node_key_challenge: ChallengePublicKey,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn over_tls_response_uses_pascal_case_and_mkey_encoding() {
        let pk = MachinePublicKey::from([0x11; 32]);
        let resp = OverTLSPublicKeyResponse::new(pk);
        let json = serde_json::to_string(&resp).unwrap();
        // Field order = declaration order; both keys present, PascalCase, mkey:.
        assert_eq!(
            json,
            format!(
                "{{\"LegacyPublicKey\":\"mkey:{}\",\"PublicKey\":\"mkey:{}\"}}",
                "00".repeat(32),
                "11".repeat(32),
            )
        );
    }

    #[test]
    fn over_tls_response_round_trips() {
        let resp = OverTLSPublicKeyResponse::new(MachinePublicKey::from([0xab; 32]));
        let back: OverTLSPublicKeyResponse =
            serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn early_noise_uses_pascal_case_and_chalpub_encoding() {
        let challenge = ChallengePublicKey::from([0x22; 32]);
        let en = EarlyNoise {
            node_key_challenge: challenge,
        };
        let json = serde_json::to_string(&en).unwrap();
        assert_eq!(
            json,
            format!("{{\"NodeKeyChallenge\":\"chalpub:{}\"}}", "22".repeat(32))
        );
        // And it decodes back.
        let back: EarlyNoise = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.node_key_challenge,
            ChallengePublicKey::from_str(&format!("chalpub:{}", "22".repeat(32))).unwrap()
        );
    }
}
