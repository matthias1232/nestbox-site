//! Nestbox control API.
//!
//! The binary starts the process; everything else lives here so tests can build
//! the router without binding a socket.
//!
//! Read `AGENTS.md` in this folder for the layer model, and `docs/adr/` for the
//! reasoning behind it.

pub mod api;
pub mod config;
pub mod domain;
pub mod error;
pub mod events;
pub mod hosting;
pub mod openapi;
pub mod runtime;
pub mod security;
pub mod services;
pub mod state;
pub mod store;

use axum::Router;

use crate::state::AppState;

/// Build the application router.
///
/// Kept as a free function so a test can mount it with `tower::ServiceExt`
/// without a listener, which is what makes the handler tests fast.
pub fn app(state: AppState) -> Router {
    api::router(state)
}
