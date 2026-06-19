//! Control-plane implementation: state coordinator, persistence, map fan-out,
//! policy, and HTTP route handlers.

#![forbid(unsafe_code)]

pub mod auth;
pub mod change;
pub mod derp;
pub mod ipalloc;
pub mod mapper;
pub mod policy;
pub mod server;
pub mod state;
pub mod store;
