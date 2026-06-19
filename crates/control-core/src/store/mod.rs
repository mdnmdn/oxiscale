//! Durable persistence port (`Store` trait) and bundled backends.
//!
//! The control plane never talks to a database directly; the host injects a
//! `dyn Store`. Phase 1 ships [`MemoryStore`] and [`FileStore`] only — SQL is
//! deferred behind the same trait.

mod document;
mod error;
mod file;
mod memory;
mod traits;
mod types;

pub use document::StoreDocument;
pub use error::StoreError;
pub use file::{FileStore, Format};
pub use memory::MemoryStore;
pub use traits::Store;
pub use types::{Network, NetworkId, Node, NodeId, PreAuthKey, PreAuthKeyId, User, UserId};
