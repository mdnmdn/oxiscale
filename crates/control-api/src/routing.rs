//! Tenant routing strategies (see `_docs/02-target-architecture.md` §4).
//!
//! The server can apply subdomain, path-prefix, and optional mTLS routing
//! simultaneously; the first match wins.

/// How incoming requests are mapped to a [`super::server::Network`].
#[derive(Debug, Clone, Default)]
pub struct TenantRouting {
    pub subdomain: bool,
    pub path_prefix: bool,
    pub optional_mtls: bool,
}

impl TenantRouting {
    /// Subdomain tenancy (`tenant1.ctl.example.com`) — works with all clients.
    pub fn subdomain() -> Self {
        Self {
            subdomain: true,
            path_prefix: false,
            optional_mtls: false,
        }
    }

    /// Path-prefix tenancy (`/tenant1/key`) — requires a patched Rust client.
    pub fn path_prefix() -> Self {
        Self {
            subdomain: false,
            path_prefix: true,
            optional_mtls: false,
        }
    }

    /// All three strategies enabled; first match wins.
    pub fn all() -> Self {
        Self {
            subdomain: true,
            path_prefix: true,
            optional_mtls: true,
        }
    }
}
