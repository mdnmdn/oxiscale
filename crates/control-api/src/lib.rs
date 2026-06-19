//! Public embedding API consumed by host applications.
//!
//! Phase 1 exposes `ControlServer`, `Network`, `PreAuthKey`, and an event
//! stream. HTTP and gRPC management surfaces are out of scope.

#![forbid(unsafe_code)]

mod events;
mod routing;
mod server;

pub use events::Event;
pub use routing::TenantRouting;
pub use server::{ControlServer, ControlServerBuilder};
