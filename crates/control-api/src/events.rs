use control_core::store::{NetworkId, NodeId};

/// Control-plane events surfaced to the host application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    NodeOnline { network: NetworkId, node: NodeId },
    NodeOffline { network: NetworkId, node: NodeId },
    NodeRegistered { network: NetworkId, node: NodeId },
}
