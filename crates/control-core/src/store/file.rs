use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::document::StoreDocument;
use super::error::StoreError;
use super::memory::MemoryStore;
use super::traits::Store;
use super::types::{Network, NetworkId, Node, NodeId, PreAuthKey, PreAuthKeyId, User};

/// Snapshot file format for [`FileStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Yaml,
    Toml,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "json" => Some(Self::Json),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            _ => None,
        }
    }
}

/// File-backed store: whole-document snapshots with write-temp-then-rename.
pub struct FileStore {
    #[allow(dead_code)]
    path: PathBuf,
    format: Format,
    memory: MemoryStore,
}

impl FileStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let format = Format::from_path(&path).ok_or(StoreError::Serde(
            "unsupported file extension; use .json, .yaml, .yml, or .toml".into(),
        ))?;
        let memory = MemoryStore::new();
        Ok(Self {
            path,
            format,
            memory,
        })
    }

    pub fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    pub async fn load(&self) -> Result<(), StoreError> {
        Err(StoreError::NotImplemented)
    }

    pub async fn flush(&self) -> Result<(), StoreError> {
        Err(StoreError::NotImplemented)
    }

    #[allow(dead_code)]
    fn serialize(&self, doc: &StoreDocument) -> Result<Vec<u8>, StoreError> {
        match self.format {
            Format::Json => {
                serde_json::to_vec_pretty(doc).map_err(|e| StoreError::Serde(e.to_string()))
            }
            Format::Yaml => serde_yaml::to_string(doc)
                .map(|s| s.into_bytes())
                .map_err(|e| StoreError::Serde(e.to_string())),
            Format::Toml => toml::to_string_pretty(doc)
                .map(|s| s.into_bytes())
                .map_err(|e| StoreError::Serde(e.to_string())),
        }
    }
}

#[async_trait]
impl Store for FileStore {
    async fn list_networks(&self) -> Result<Vec<Network>, StoreError> {
        self.memory.list_networks().await
    }

    async fn upsert_network(&self, net: &Network) -> Result<(), StoreError> {
        self.memory.upsert_network(net).await?;
        self.flush().await
    }

    async fn list_users(&self, network: NetworkId) -> Result<Vec<User>, StoreError> {
        self.memory.list_users(network).await
    }

    async fn upsert_user(&self, user: &User) -> Result<(), StoreError> {
        self.memory.upsert_user(user).await?;
        self.flush().await
    }

    async fn list_nodes(&self, network: NetworkId) -> Result<Vec<Node>, StoreError> {
        self.memory.list_nodes(network).await
    }

    async fn get_node(&self, id: NodeId) -> Result<Option<Node>, StoreError> {
        self.memory.get_node(id).await
    }

    async fn upsert_node(&self, node: &Node) -> Result<(), StoreError> {
        self.memory.upsert_node(node).await?;
        self.flush().await
    }

    async fn delete_node(&self, id: NodeId) -> Result<(), StoreError> {
        self.memory.delete_node(id).await?;
        self.flush().await
    }

    async fn list_preauth_keys(&self, network: NetworkId) -> Result<Vec<PreAuthKey>, StoreError> {
        self.memory.list_preauth_keys(network).await
    }

    async fn upsert_preauth_key(&self, key: &PreAuthKey) -> Result<(), StoreError> {
        self.memory.upsert_preauth_key(key).await?;
        self.flush().await
    }

    async fn consume_preauth_key(&self, id: PreAuthKeyId) -> Result<bool, StoreError> {
        let consumed = self.memory.consume_preauth_key(id).await?;
        if consumed {
            self.flush().await?;
        }
        Ok(consumed)
    }

    async fn get_policy(&self, network: NetworkId) -> Result<Option<String>, StoreError> {
        self.memory.get_policy(network).await
    }

    async fn set_policy(&self, network: NetworkId, policy: &str) -> Result<(), StoreError> {
        self.memory.set_policy(network, policy).await?;
        self.flush().await
    }
}
