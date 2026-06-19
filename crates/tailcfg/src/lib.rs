//! `tailcfg` wire types for the Tailscale control protocol.
//!
//! The bulk of the wire structs (register, map, node, hostinfo, DNS, DERP) are
//! re-exported from [`ts_control_serde`] (`_refs/tailscale-rs`) — the phase-1
//! spike confirmed reuse is viable. This crate adds the server-emitted types
//! ([`OverTLSPublicKeyResponse`], [`EarlyNoise`]) and the protocol constants
//! that the reused crate does not model.

#![forbid(unsafe_code)]

pub mod capver;
mod key_exchange;
pub mod protocol;

pub use key_exchange::{EarlyNoise, OverTLSPublicKeyResponse};
pub use protocol::{
    EARLY_NOISE_MAGIC, KEEPALIVE_INTERVAL, PATH_KEY, PATH_MACHINE_MAP, PATH_MACHINE_REGISTER,
    PATH_TS2021, UPGRADE_TS2021,
};

// Capability version type, reused from the Tailscale Rust client.
pub use ts_capabilityversion::CapabilityVersion;

// Wire types reused from `ts_control_serde`. These are the JSON shapes spoken
// inside the Noise/HTTP-2 tunnel for register + map (phases 2–3).
pub use ts_control_serde::{
    DerpMap, DnsConfig, Endpoint, EndpointType, HostInfo, MapRequest, MapResponse, NetInfo, Node,
    NodeId, PeerChange, RegisterAuth, RegisterRequest, RegisterResponse, StableNodeId, User,
    UserId, UserProfile,
};
