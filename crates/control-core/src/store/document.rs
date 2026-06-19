use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::types::{Network, NetworkId, Node, PreAuthKey, User};

/// On-disk envelope for [`super::FileStore`]; format chosen by extension or builder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreDocument {
    pub version: u32,
    pub networks: Vec<Network>,
    pub users: Vec<User>,
    pub nodes: Vec<Node>,
    pub preauth_keys: Vec<PreAuthKey>,
    pub policies: HashMap<NetworkId, String>,
}
