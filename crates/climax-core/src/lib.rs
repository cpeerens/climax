//! Climax core engine - the Tauri-free "home".
//!
//! Phase 2a of the client-server pivot (see CLAUDE.md "Direction: Climax is
//! becoming a SERVER") carved these modules out of the Tauri app into their own
//! crate so they can later run headless inside a standalone server with no
//! WebView / Windows-native dependencies. Everything here is platform-neutral
//! (Linux/Docker-buildable): the SQLite DB layer (owns the `migrations/`), the
//! domain models, persisted settings, the Stash GraphQL client, the
//! dashboard / catalog / trends aggregations, the mirror + reconstruction
//! engines, and the session state machine.
//!
//! The desktop app (`src-tauri`) depends on this crate and re-exports these
//! modules, so its existing `crate::db::...` paths still resolve unchanged.
//! Inter-module references inside this crate use `crate::` and resolve here.

pub mod capture;
pub mod catalog;
pub mod dashboard;
pub mod db;
pub mod mirror;
pub mod models;
pub mod reconstruct;
pub mod remote;
// `rpc` is the transport-agnostic command surface (POST /rpc), and `server` is
// the Axum HTTP/WS server that exposes it + the bridge socket. Both moved into
// core in Phase 2b so the standalone (headless) server is fully core-resident;
// the desktop shell re-exports them so its existing paths still resolve.
pub mod rpc;
pub mod server;
pub mod session;
pub mod settings;
pub mod stash;
pub mod state;
pub mod trends;
pub mod update;
