use async_trait::async_trait;

use super::error::StoreError;
use super::types::{Network, NetworkId, Node, NodeId, PreAuthKey, PreAuthKeyId, User};

/// Durable persistence port. The control plane owns a `dyn Store`; the host
/// picks the backend (or implements this trait itself).
#[async_trait]
pub trait Store: Send + Sync + 'static {
    // --- Networks (one default row in phase 1; the network_id seam) ---
    async fn list_networks(&self) -> Result<Vec<Network>, StoreError>;
    async fn upsert_network(&self, net: &Network) -> Result<(), StoreError>;

    // --- Users ---
    async fn list_users(&self, network: NetworkId) -> Result<Vec<User>, StoreError>;
    async fn upsert_user(&self, user: &User) -> Result<(), StoreError>;

    // --- Nodes ---
    async fn list_nodes(&self, network: NetworkId) -> Result<Vec<Node>, StoreError>;
    async fn get_node(&self, id: NodeId) -> Result<Option<Node>, StoreError>;
    async fn upsert_node(&self, node: &Node) -> Result<(), StoreError>;
    async fn delete_node(&self, id: NodeId) -> Result<(), StoreError>;

    // --- Pre-auth keys ---
    async fn list_preauth_keys(&self, network: NetworkId) -> Result<Vec<PreAuthKey>, StoreError>;
    async fn upsert_preauth_key(&self, key: &PreAuthKey) -> Result<(), StoreError>;

    /// Atomic single-use consumption. Returns `true` iff this call won the race
    /// and flipped an unused key to used.
    async fn consume_preauth_key(&self, id: PreAuthKeyId) -> Result<bool, StoreError>;

    // --- Policy ---
    async fn get_policy(&self, network: NetworkId) -> Result<Option<String>, StoreError>;
    async fn set_policy(&self, network: NetworkId, policy: &str) -> Result<(), StoreError>;
}
