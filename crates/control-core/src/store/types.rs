//! Domain aggregates persisted through [`super::Store`].
//!
//! Every aggregate carries `network_id` from phase 2 onward — a single default
//! network in the hub-and-spoke case, with the seam kept open for multi-network.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PreAuthKeyId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub id: NetworkId,
    pub name: String,
    /// Routing key: canonical host and optional path prefix (see tenancy §4).
    pub server_host: String,
    pub ipv4_prefix: String,
    pub ipv6_prefix: String,
    pub noise_key_id: String,
    pub policy: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub network_id: NetworkId,
    pub login_name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub network_id: NetworkId,
    pub user_id: UserId,
    pub machine_key: String,
    pub node_key: String,
    pub disco_key: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreAuthKey {
    pub id: PreAuthKeyId,
    pub network_id: NetworkId,
    pub prefix: String,
    pub secret_hash: String,
    pub reusable: bool,
    pub ephemeral: bool,
    pub used: bool,
    pub expires_at: Option<String>,
    pub tags: Vec<String>,
}
