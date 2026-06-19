use std::sync::Arc;

use control_core::server::{self, ServerState};
use control_core::store::{MemoryStore, Store};
use thiserror::Error;
use tokio::net::TcpListener;
use tskey::{MachineKeyPair, MachinePublicKey};

use super::routing::TenantRouting;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("no persistence backend configured")]
    MissingStore,

    #[error("failed to bind listener on {addr}")]
    Bind {
        addr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("control server stopped with an error")]
    Serve(#[from] server::ServerError),
}

/// Default listen address when none is configured.
const DEFAULT_LISTEN: &str = "0.0.0.0:8080";

/// Builder for a [`ControlServer`] instance.
pub struct ControlServerBuilder {
    store: Option<Arc<dyn Store>>,
    listen: Option<String>,
    tenant_routing: TenantRouting,
    server_key: Option<MachineKeyPair>,
}

impl Default for ControlServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlServerBuilder {
    pub fn new() -> Self {
        Self {
            store: None,
            listen: None,
            tenant_routing: TenantRouting::default(),
            server_key: None,
        }
    }

    /// Inject the persistence backend (`MemoryStore`, `FileStore`, or a host `impl Store`).
    pub fn store(mut self, store: impl Store + 'static) -> Self {
        self.store = Some(Arc::new(store));
        self
    }

    /// Default volatile backend for tests and ephemeral deployments.
    pub fn memory_store(mut self) -> Self {
        self.store = Some(Arc::new(MemoryStore::new()));
        self
    }

    pub fn listen(mut self, addr: impl Into<String>) -> Self {
        self.listen = Some(addr.into());
        self
    }

    pub fn tenant_routing(mut self, routing: TenantRouting) -> Self {
        self.tenant_routing = routing;
        self
    }

    /// Supply the server's long-lived Noise static identity. If unset, a fresh
    /// ephemeral keypair is generated at [`build`](Self::build) time.
    ///
    /// Persisting and reloading this key is a phase-2 concern (it lives in the
    /// `Store`); an ephemeral key is sufficient for the phase-1 handshake gate.
    pub fn server_key(mut self, key: MachineKeyPair) -> Self {
        self.server_key = Some(key);
        self
    }

    pub fn build(self) -> Result<ControlServer, ApiError> {
        let store = self.store.ok_or(ApiError::MissingStore)?;
        let _tenant_routing = self.tenant_routing;
        Ok(ControlServer {
            store,
            listen: self.listen.unwrap_or_else(|| DEFAULT_LISTEN.to_owned()),
            server_key: self.server_key.unwrap_or_default(),
        })
    }
}

/// Embeddable Tailscale control server handle.
pub struct ControlServer {
    store: Arc<dyn Store>,
    listen: String,
    server_key: MachineKeyPair,
}

impl ControlServer {
    pub fn builder() -> ControlServerBuilder {
        ControlServerBuilder::new()
    }

    pub fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    /// The server's Noise static public key — what `GET /key` advertises.
    pub fn server_public_key(&self) -> MachinePublicKey {
        self.server_key.public
    }

    /// The configured listen address.
    pub fn listen_addr(&self) -> &str {
        &self.listen
    }

    /// Bind the configured address and serve until shutdown.
    pub async fn serve(&self) -> Result<(), ApiError> {
        let listener = TcpListener::bind(&self.listen)
            .await
            .map_err(|source| ApiError::Bind {
                addr: self.listen.clone(),
                source,
            })?;
        tracing::info!(addr = %self.listen, server_key = %self.server_key.public, "control server listening");
        server::serve(listener, ServerState::new(self.server_key)).await?;
        Ok(())
    }
}
