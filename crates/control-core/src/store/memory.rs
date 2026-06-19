use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::document::StoreDocument;
use super::error::StoreError;
use super::traits::Store;
use super::types::{Network, NetworkId, Node, NodeId, PreAuthKey, PreAuthKeyId, User};

/// Volatile in-process backend — default for tests and ephemeral deployments.
#[derive(Debug)]
pub struct MemoryStore {
    inner: Arc<RwLock<StoreDocument>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreDocument {
                version: 1,
                networks: Vec::new(),
                users: Vec::new(),
                nodes: Vec::new(),
                preauth_keys: Vec::new(),
                policies: Default::default(),
            })),
        }
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn list_networks(&self) -> Result<Vec<Network>, StoreError> {
        Ok(self.inner.read().await.networks.clone())
    }

    async fn upsert_network(&self, net: &Network) -> Result<(), StoreError> {
        let mut doc = self.inner.write().await;
        if let Some(existing) = doc.networks.iter_mut().find(|n| n.id == net.id) {
            *existing = net.clone();
        } else {
            doc.networks.push(net.clone());
        }
        Ok(())
    }

    async fn list_users(&self, network: NetworkId) -> Result<Vec<User>, StoreError> {
        Ok(self
            .inner
            .read()
            .await
            .users
            .iter()
            .filter(|u| u.network_id == network)
            .cloned()
            .collect())
    }

    async fn upsert_user(&self, user: &User) -> Result<(), StoreError> {
        let mut doc = self.inner.write().await;
        if let Some(existing) = doc.users.iter_mut().find(|u| u.id == user.id) {
            *existing = user.clone();
        } else {
            doc.users.push(user.clone());
        }
        Ok(())
    }

    async fn list_nodes(&self, network: NetworkId) -> Result<Vec<Node>, StoreError> {
        Ok(self
            .inner
            .read()
            .await
            .nodes
            .iter()
            .filter(|n| n.network_id == network)
            .cloned()
            .collect())
    }

    async fn get_node(&self, id: NodeId) -> Result<Option<Node>, StoreError> {
        Ok(self
            .inner
            .read()
            .await
            .nodes
            .iter()
            .find(|n| n.id == id)
            .cloned())
    }

    async fn upsert_node(&self, node: &Node) -> Result<(), StoreError> {
        let mut doc = self.inner.write().await;
        if let Some(existing) = doc.nodes.iter_mut().find(|n| n.id == node.id) {
            *existing = node.clone();
        } else {
            doc.nodes.push(node.clone());
        }
        Ok(())
    }

    async fn delete_node(&self, id: NodeId) -> Result<(), StoreError> {
        let mut doc = self.inner.write().await;
        doc.nodes.retain(|n| n.id != id);
        Ok(())
    }

    async fn list_preauth_keys(&self, network: NetworkId) -> Result<Vec<PreAuthKey>, StoreError> {
        Ok(self
            .inner
            .read()
            .await
            .preauth_keys
            .iter()
            .filter(|k| k.network_id == network)
            .cloned()
            .collect())
    }

    async fn upsert_preauth_key(&self, key: &PreAuthKey) -> Result<(), StoreError> {
        let mut doc = self.inner.write().await;
        if let Some(existing) = doc.preauth_keys.iter_mut().find(|k| k.id == key.id) {
            *existing = key.clone();
        } else {
            doc.preauth_keys.push(key.clone());
        }
        Ok(())
    }

    async fn consume_preauth_key(&self, id: PreAuthKeyId) -> Result<bool, StoreError> {
        let mut doc = self.inner.write().await;
        let Some(key) = doc.preauth_keys.iter_mut().find(|k| k.id == id) else {
            return Ok(false);
        };
        if key.used || key.reusable {
            return Ok(false);
        }
        key.used = true;
        Ok(true)
    }

    async fn get_policy(&self, network: NetworkId) -> Result<Option<String>, StoreError> {
        Ok(self.inner.read().await.policies.get(&network).cloned())
    }

    async fn set_policy(&self, network: NetworkId, policy: &str) -> Result<(), StoreError> {
        self.inner
            .write()
            .await
            .policies
            .insert(network, policy.to_owned());
        Ok(())
    }
}
