//! Embedded web UI (Phase 5). The SvelteKit SPA build (repo-root `build/`) is
//! baked into the binary (release) or read from disk (debug), and served as the
//! router FALLBACK - the API routes registered in core's `server::spawn` always
//! win, and any unmatched GET becomes either a static asset or the SPA's
//! `index.html` (client-side routing handles /dashboard etc).
//!
//! Lives in THIS crate, not climax-core, so the desktop binary is structurally
//! incapable of serving the web UI or the /backup download - cargo unifies
//! features workspace-wide, so a core feature flag would leak into desktop
//! workspace builds (caught in the Milestone-A adversarial review).

use axum::http::{header, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use climax_core::state::AppState;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../build"]
struct Assets;

/// The web-client router extension merged into core's API router: the embedded
/// SPA as the fallback + the server-side backup download, restore upload, and
/// full reset (the last two stage + exit; the supervisor restarts the process
/// and boot applies the change before the pool opens - all three handlers live
/// in core as pub-but-unrouted, mounted ONLY here so the desktop binary never
/// exposes them).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backup", get(climax_core::server::backup_download))
        .route("/restore", post(climax_core::server::restore_upload))
        .route("/reset", post(climax_core::server::reset_server))
        .fallback(static_handler)
}

async fn static_handler(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    // Exact asset first; anything else falls back to index.html so the SPA's
    // client router handles paths like /dashboard. (rust-embed keys are the
    // shipped relative paths - traversal/encoded probes simply miss the map.)
    let (asset, key) = match Assets::get(path) {
        Some(a) => (a, path),
        None => match Assets::get("index.html") {
            Some(a) => (a, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let mime = mime_guess::from_path(key).first_or_octet_stream();
    // SvelteKit's content-hashed files live under _app/immutable/ - cache those
    // hard. Everything else (index.html, fonts.css, icons) must revalidate so a
    // server upgrade shows the new UI immediately.
    let cache = if key.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref().to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        asset.data.into_owned(),
    )
        .into_response()
}
