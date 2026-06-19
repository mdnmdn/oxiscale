//! Capability version handling.
//!
//! The [`CapabilityVersion`](crate::CapabilityVersion) type itself is reused
//! from `ts_capabilityversion`; this module pins the server's minimum
//! supported client version.

/// Minimum Tailscale client capability version accepted by this server.
///
/// 113 corresponds to Tailscale v1.80 (see [05-protocol-reference.md] §7). The
/// `v` query parameter of `GET /key` and the initiation message's capability
/// version are compared against this; older clients may be rejected.
pub const MIN_SUPPORTED_CAPABILITY_VERSION: u16 = 113;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_supported_is_v1_80_baseline() {
        assert_eq!(MIN_SUPPORTED_CAPABILITY_VERSION, 113);
    }
}
