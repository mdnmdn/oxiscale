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
    path: PathBuf,
    format: Format,
    memory: MemoryStore,
}

impl FileStore {
    /// Open a snapshot file, loading its contents if it already exists.
    ///
    /// A missing file is not an error — it yields an empty store that
    /// [`flush`](Self::flush) will create on first write.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let format = Format::from_path(&path).ok_or(StoreError::Serde(
            "unsupported file extension; use .json, .yaml, .yml, or .toml".into(),
        ))?;
        let store = Self {
            path,
            format,
            memory: MemoryStore::new(),
        };
        store.load().await?;
        Ok(store)
    }

    pub fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Load the snapshot from disk into memory, replacing current state.
    /// A missing file is treated as an empty (fresh) store.
    pub async fn load(&self) -> Result<(), StoreError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(StoreError::Io(e)),
        };
        let doc = self.deserialize(&bytes)?;
        self.memory.replace(doc).await;
        Ok(())
    }

    /// Persist the current in-memory state to disk atomically
    /// (write to a temp file in the same directory, then rename over the target).
    pub async fn flush(&self) -> Result<(), StoreError> {
        let doc = self.memory.snapshot().await;
        let bytes = self.serialize(&doc)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

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

    fn deserialize(&self, bytes: &[u8]) -> Result<StoreDocument, StoreError> {
        match self.format {
            Format::Json => {
                serde_json::from_slice(bytes).map_err(|e| StoreError::Serde(e.to_string()))
            }
            Format::Yaml => {
                serde_yaml::from_slice(bytes).map_err(|e| StoreError::Serde(e.to_string()))
            }
            Format::Toml => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|e| StoreError::Serde(format!("snapshot is not valid UTF-8: {e}")))?;
                toml::from_str(text).map_err(|e| StoreError::Serde(e.to_string()))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_json_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!(
            "oxiscale-filestore-{}-{}.json",
            std::process::id(),
            nanos
        ));
        p
    }

    fn sample_network() -> Network {
        Network {
            id: super::super::types::NetworkId(1),
            name: "default".into(),
            server_host: "ctl.local".into(),
            ipv4_prefix: "100.64.0.0/10".into(),
            ipv6_prefix: "fd7a:115c:a1e0::/48".into(),
            noise_key_id: "k1".into(),
            policy: None,
            created_at: "2026-06-19T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn mutations_succeed_and_persist_across_reopen() {
        let path = temp_json_path();

        {
            let store = FileStore::open(path.clone()).await.unwrap();
            // The flush-after-mutate path must succeed (regression: it used to
            // return NotImplemented).
            store.upsert_network(&sample_network()).await.unwrap();
        }

        // A fresh instance loads the snapshot written above.
        let reloaded = FileStore::open(path.clone()).await.unwrap();
        let networks = reloaded.list_networks().await.unwrap();
        assert_eq!(networks.len(), 1);
        assert_eq!(networks[0].id, super::super::types::NetworkId(1));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn open_missing_file_is_empty_not_an_error() {
        let path = temp_json_path();
        let store = FileStore::open(path).await.unwrap();
        assert!(store.list_networks().await.unwrap().is_empty());
    }
}
