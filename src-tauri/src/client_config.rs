// Client-mode configuration (Phase 2c). A small JSON file the desktop shell reads
// at BOOT to decide whether this app is a thin CLIENT of an external Climax server
// (and of which one) vs running its own in-process backend (the default).
//
// Why a file and not the frontend's localStorage: the boot path (lib.rs setup)
// has to make the spawn-the-backend-or-not decision BEFORE the webview exists, so
// it can't read `localStorage.climax_server_url`. The two are kept in sync - the
// Settings "Server / connection" page writes BOTH (localStorage for the frontend
// transport, this file for the Rust boot). Absent / empty = the in-process default.

use std::path::Path;

const FILE: &str = "client.json";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ClientConfig {
    /// External server origin (e.g. "http://localhost:9998"), or None = the
    /// built-in in-process backend.
    #[serde(default)]
    server_url: Option<String>,
    /// Shared auth token for that server (Phase 3), if it requires one. Sent as
    /// X-Climax-Token by the Rust-side proxies (rpc_http, remote_session).
    #[serde(default)]
    token: Option<String>,
}

fn normalize(url: Option<String>) -> Option<String> {
    url.map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_token(token: Option<String>) -> Option<String> {
    token.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn read(data_dir: &Path) -> ClientConfig {
    std::fs::read_to_string(data_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The configured external server URL, or None for the in-process default.
pub fn read_server_url(data_dir: &Path) -> Option<String> {
    normalize(read(data_dir).server_url)
}

/// The configured server token, if any. Only meaningful alongside a server URL.
pub fn read_token(data_dir: &Path) -> Option<String> {
    normalize_token(read(data_dir).token)
}

/// Persist (or clear, with None) the configured external server URL + token.
/// Takes effect on the next launch (the boot branch reads it).
pub fn write_server_url(
    data_dir: &Path,
    server_url: Option<String>,
    token: Option<String>,
) -> std::io::Result<()> {
    let cfg = ClientConfig {
        server_url: normalize(server_url),
        token: normalize_token(token),
    };
    let text = serde_json::to_string_pretty(&cfg).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(data_dir.join(FILE), text)
}
