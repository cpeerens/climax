// Local HTTP server, bound to 127.0.0.1.
//
// Two transports:
//   - HTTP REST endpoints (/health, /heartbeat, /active_session) for debugging
//     and any non-Stash sources.
//   - WebSocket /ws for the Stash bridge plugin. Stash has a Content Security
//     Policy that whitelists `ws:` to any host but blocks `http:` cross-origin
//     fetches, so the bridge must use WebSocket.

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use std::net::SocketAddr;

use serde::Deserialize;

use crate::models::{HeartbeatPayload, HeartbeatResponse, UiSignal};
use crate::state::AppState;


/// Incoming WebSocket message envelope. Tagged so the bridge can send
/// heartbeats and O-events down the same socket.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsIn {
    Heartbeat(HeartbeatPayload),
    OEvent(WsOPayload),
    ORemove(WsORemovePayload),
    /// Bridge navbar indicator clicked: start a session (if none) + show the
    /// tracker. Unit variant — matches `{"type":"open_tracker"}`.
    OpenTracker,
    /// Bridge indicator clicked while tracking: show the tracker and run its
    /// Stop flow (the wrap-up modal). Matches `{"type":"request_stop"}`.
    RequestStop,
    /// Bridge organising-mode toggle from the navbar pill's popover menu.
    /// Matches `{"type":"set_organising","on":true|false}`. Entering pauses
    /// Stash's play-history tracking; leaving resumes it.
    SetOrganising { on: bool },
    /// Start a session and show NOTHING. The browser-target counterpart of
    /// `open_tracker`: when the pill isn't pointed at the desktop app, raising
    /// its tracker window contradicts that choice, and the pill flipping to
    /// TRACKING is feedback enough. Matches `{"type":"start_session"}`.
    StartSession,
    /// "Open Climax" from the pill's menu: show the tracker WITHOUT touching the
    /// session. Distinct from `open_tracker`, which also starts one if none is
    /// running - that's the wrong behaviour for a plain "show me the app".
    /// Matches `{"type":"open_climax"}`.
    OpenClimax,
    /// Pause / resume the running session from the pill's menu.
    /// Matches `{"type":"pause_session"}` / `{"type":"resume_session"}`.
    PauseSession,
    ResumeSession,
}

/// Payload for an O event coming from a source (e.g. Stash bridge intercepted
/// the user clicking the O button in Stash's UI). The bridge no longer sends
/// scene metadata — Climax pulls title/performers/etc via GraphQL.
#[derive(Debug, Deserialize)]
struct WsOPayload {
    source: String,
    scene_id: String,
    occurred_at: Option<i64>,
}

/// Payload for an O *removal* coming from a source (e.g. the bridge intercepted
/// the user decrementing the O counter in Stash via sceneDecrementO). No
/// timestamp — we remove the most recent matching event in the active session.
#[derive(Debug, Deserialize)]
struct WsORemovePayload {
    source: String,
    scene_id: String,
}

/// Options for `spawn`. The desktop shell uses `SpawnOpts::local(port)`
/// (localhost bind, no token, no extra routes - the pre-Phase-3 surface,
/// unchanged); the standalone climax-server builds these from its env knobs.
pub struct SpawnOpts {
    /// Bind address. Localhost by default; `0.0.0.0` (opt-in, standalone server
    /// only) exposes the server to the LAN/Docker.
    pub addr: SocketAddr,
    /// Shared auth token (Phase 3). When set, the DATA endpoints (/rpc,
    /// /heartbeat, /active_session, /ws, /events, /backup, /restore, /reset)
    /// require it - via the `X-Climax-Token` header, or `?token=` for WebSockets
    /// (a browser can't set headers on a WS handshake). /health and the static
    /// web UI stay open so a browser can load the page and prompt for the token.
    pub token: Option<String>,
    /// Extra hostnames allowed past the DNS-rebinding Host check (see
    /// `host_allowed`) - for setups like Tailscale MagicDNS names.
    pub allowed_hosts: Vec<String>,
    /// Extra routes and/or a fallback merged into the router (Phase 5: the
    /// standalone climax-server passes its embedded web UI + GET /backup). The
    /// desktop passes None, so its in-process server is structurally incapable
    /// of growing web-client endpoints - no feature flags to accidentally unify
    /// in (cargo unifies features workspace-wide, which is exactly how an
    /// earlier cut leaked a /backup route into the desktop binary).
    pub extra: Option<Router<AppState>>,
}

impl SpawnOpts {
    /// The desktop shell's in-process server: localhost, no token, no extras.
    pub fn local(port: u16) -> Self {
        SpawnOpts {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            token: None,
            allowed_hosts: Vec::new(),
            extra: None,
        }
    }
}

/// Endpoints that carry personal data (or can destroy it - /restore, /reset)
/// and therefore require the token when one is configured. Everything else
/// (/health, the static web UI) stays open.
const TOKEN_PROTECTED: [&str; 8] = [
    "/rpc", "/heartbeat", "/active_session", "/ws", "/events", "/backup", "/restore", "/reset",
];

/// DNS-rebinding guard. A malicious domain can rebind its DNS to 127.0.0.1 (or
/// a LAN IP), making the victim's browser treat requests to this server as
/// same-origin with the attacker's page - defeating the same-origin policy that
/// otherwise protects /rpc. The tell is the Host header: a rebinding attack
/// arrives with the ATTACKER'S public DNS name. So allow only hosts that can't
/// be attacker-controlled public DNS:
///   - IP literals (v4, bracketed v6) - rebinding needs a DNS *name*
///   - "localhost"
///   - single-label names (no dot - not registrable on public DNS; covers LAN
///     hostnames like "mynas")
///   - `*.local` (mDNS - link-local resolution, not public DNS)
///   - anything in `allowed_hosts` (e.g. a Tailscale ts.net name)
///
/// An absent Host header is allowed: browsers always send one, so that's only
/// raw tooling (curl scripts, health probes).
fn host_allowed(host_header: &str, allowed: &[String]) -> bool {
    if host_header.is_empty() {
        return true;
    }
    // Strip the port: "[::1]:9998" -> "[::1]", "example.com:9998" -> "example.com".
    let hostname = if let Some(end) = host_header.find(']') {
        &host_header[..=end]
    } else {
        host_header.split(':').next().unwrap_or(host_header)
    };
    let bare = hostname.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let lower = hostname.to_ascii_lowercase();
    if lower == "localhost" || !lower.contains('.') || lower.ends_with(".local") {
        return true;
    }
    allowed.iter().any(|a| a.eq_ignore_ascii_case(&lower))
}

/// Minimal percent-decoding for the `?token=` query value (clients
/// encodeURIComponent it; '+' is NOT treated as a space - this is a query
/// component, not form data).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Token check for one request: `X-Climax-Token` header, else `?token=` in the
/// query string (WebSocket handshakes from a browser can't set headers; the
/// query value may arrive percent-encoded).
fn token_ok(req: &axum::extract::Request, expected: &str) -> bool {
    if let Some(v) = req.headers().get("x-climax-token").and_then(|v| v.to_str().ok()) {
        if v == expected {
            return true;
        }
    }
    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some(v) = pair.strip_prefix("token=") {
                if v == expected || percent_decode(v) == expected {
                    return true;
                }
            }
        }
    }
    false
}

/// How long to keep trying a port that is only momentarily still held, and how
/// often. EVERY restart path starts its replacement process while the outgoing
/// one is still shutting down: the desktop's `app.restart()` (backup restore,
/// full reset, switching to/from an external server) spawns the new binary then
/// exits, and the headless server's web restore/reset exits for its supervisor
/// to relaunch. On Windows the listening socket is not released until the old
/// process has actually gone, so the newcomer can arrive a few hundred
/// milliseconds early. Binding once and giving up turned that race into
/// "Climax couldn't start on port N" - reproduced on a real release build, and
/// invisible before the bind failure was surfaced at all.
const BIND_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
const BIND_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Bind, tolerating a predecessor that has not finished letting go.
///
/// ONLY `AddrInUse` is worth waiting on. A port inside a Windows reserved range
/// fails with `WSAEACCES` -> `PermissionDenied` and will never come free, so it
/// returns immediately and keeps the error message honest rather than stalling
/// three seconds before saying the same thing.
async fn bind_with_retry(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let deadline = std::time::Instant::now() + BIND_RETRY_WINDOW;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                if attempt > 1 {
                    tracing::info!(
                        "bound {} on attempt {} - the previous instance was still releasing it",
                        addr,
                        attempt
                    );
                }
                return Ok(listener);
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse
                    && std::time::Instant::now() + BIND_RETRY_DELAY < deadline =>
            {
                tracing::debug!("{} still busy, retrying (attempt {})", addr, attempt);
                tokio::time::sleep(BIND_RETRY_DELAY).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Spawn the HTTP/WS server. See `SpawnOpts` for the knobs.
pub async fn spawn(state: AppState, opts: SpawnOpts) -> Result<()> {
    // No permissive CORS layer (deliberate). A wildcard
    // `Access-Control-Allow-Origin: *` would let ANY website the user visits
    // read these endpoints cross-origin from their browser - and /rpc returns
    // sensitive personal data. Nothing legitimate needs cross-origin HTTP here:
    // the bridge uses ws:// (WebSocket isn't subject to fetch CORS), the desktop
    // app uses Tauri IPC, and the web client is served SAME-ORIGIN by the
    // standalone server. A dev / separately-hosted client gets an explicit
    // origin allowlist if/when that's actually needed (later phase).
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/heartbeat", post(heartbeat))
        .route("/active_session", get(active_session))
        .route("/rpc", post(rpc))
        .route("/ws", any(ws_upgrade))
        .route("/events", any(events_upgrade));
    // Derived, not configured: only the standalone climax-server passes an extra
    // router (the web UI), so this is the authoritative "can a browser open me".
    let web_ui = WebUiAvailable(opts.extra.is_some());
    if let Some(extra) = opts.extra {
        // API routes above win; the extra router may carry a fallback (the
        // SPA's index.html), which merge adopts since the base has none.
        app = app.merge(extra);
    }
    let app = app.layer(axum::Extension(web_ui)).with_state(state);

    // Guard layer: Host validation on EVERY request (DNS rebinding - applies to
    // the desktop's localhost server too), then the token on data endpoints.
    let token = opts.token.clone();
    let allowed_hosts = std::sync::Arc::new(opts.allowed_hosts);
    let app = app.layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let token = token.clone();
            let allowed_hosts = allowed_hosts.clone();
            async move {
                let host = req
                    .headers()
                    .get(axum::http::header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !host_allowed(host, &allowed_hosts) {
                    tracing::warn!("rejected request with disallowed Host: {}", host);
                    return (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({ "error": "That address isn't allowed for this server." })),
                    )
                        .into_response();
                }
                if let Some(expected) = token.as_deref() {
                    if TOKEN_PROTECTED.contains(&req.uri().path()) && !token_ok(&req, expected) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(serde_json::json!({ "error": "This Climax server requires a token." })),
                        )
                            .into_response();
                    }
                }
                next.run(req).await
            }
        },
    ));

    // Still binds BEFORE returning, so an Err from spawn() genuinely means the
    // port is unusable - lib.rs records that into SERVER_BIND for the banner and
    // relies on it not being a "might still come good" state.
    let listener = bind_with_retry(opts.addr).await?;
    tracing::info!("http server listening on {}", opts.addr);

    // Remembered so `release_port` can shut this down before a restart. Kept in
    // module statics rather than AppState because there is exactly one server per
    // process, and AppState is built in three places that would all have to carry
    // a shutdown channel they never otherwise use.
    let _ = SERVER_ADDR.set(opts.addr);
    let stop = SERVER_STOP
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Notify::new()))
        .clone();
    let handle = tokio::spawn(async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stop.notified().await });
        if let Err(e) = served.await {
            tracing::error!("http server crashed: {}", e);
        }
    });
    if let Ok(mut slot) = SERVE_TASK.lock() {
        *slot = Some(handle);
    }
    Ok(())
}

static SERVER_ADDR: std::sync::OnceLock<SocketAddr> = std::sync::OnceLock::new();
static SERVER_STOP: std::sync::OnceLock<std::sync::Arc<tokio::sync::Notify>> =
    std::sync::OnceLock::new();
static SERVE_TASK: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>> =
    std::sync::Mutex::new(None);

/// Close the listener and confirm the port is genuinely rebindable, BEFORE a
/// restart spawns the replacement process.
///
/// THE BUG THIS EXISTS FOR. `app.restart()` spawns the new process while the old
/// one's socket is still open, and the child inherits the handle. The socket then
/// OUTLIVES its creator - netstat shows it LISTENING under a PID that no longer
/// exists - and the replacement cannot bind, because the thing holding the port
/// is the replacement itself. That is permanent and immune to any amount of
/// retrying, which is why every restore came back with no backend and a "couldn't
/// start on port" banner. Proven by watching the orphaned listener disappear the
/// instant the replacement process was killed.
///
/// Waiting is not enough and neither is a graceful shutdown on its own: a live
/// bridge WebSocket keeps connection sockets open on the same local port. So this
/// does not assume anything, it probes until a fresh bind actually succeeds.
///
/// Returns whether the port was confirmed free within `wait`. Callers should
/// restart regardless - `bind_with_retry` on the other side is the backstop, and
/// refusing to restart would strand a staged restore.
pub async fn release_port(wait: std::time::Duration) -> bool {
    let Some(addr) = SERVER_ADDR.get().copied() else {
        return true; // no server in this process (client mode) - nothing to release
    };

    // Ask axum to stop accepting and let connections wind down...
    if let Some(stop) = SERVER_STOP.get() {
        stop.notify_one();
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    // ...then guarantee the listener is dropped, which graceful shutdown will not
    // do while a WebSocket is still attached.
    if let Ok(mut slot) = SERVE_TASK.lock() {
        if let Some(handle) = slot.take() {
            handle.abort();
        }
    }

    let deadline = std::time::Instant::now() + wait;
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(probe) => {
                drop(probe);
                tracing::info!("released {} before restart", addr);
                return true;
            }
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    tracing::warn!("{} still not rebindable before restart: {}", addr, e);
                    return false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "service": "climax" }))
}

async fn heartbeat(
    State(state): State<AppState>,
    Json(payload): Json<HeartbeatPayload>,
) -> impl IntoResponse {
    tracing::info!(
        "http heartbeat: source={} tab={} scene={} state={}",
        payload.source,
        payload.tab_id,
        payload.scene_id,
        payload.video.state,
    );
    match state.session.record_heartbeat(payload).await {
        Ok(rec) => (
            StatusCode::OK,
            Json(HeartbeatResponse {
                session_active: rec.session_id.is_some(),
                session_id: rec.session_id,
                recorded: rec.recorded,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("heartbeat error: {:#}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// Handle an O event reported by a source (e.g. user clicked O in Stash).
/// Looks up / creates the content_item, finds the active session if any,
/// and logs the O event WITHOUT writing back to Stash (since Stash already
/// incremented its own counter).
async fn record_external_o(state: &AppState, payload: WsOPayload) -> anyhow::Result<()> {
    let content_id = state
        .session
        .ensure_content_item(&payload.source, &payload.scene_id)
        .await?;

    let session_id = state.session.active_id().await?;

    let logged = state
        .session
        .log_o(
            session_id,
            Some(content_id),
            payload.occurred_at,
            None,
            None,
            "stash", // origin: don't push back to Stash, can't be deleted from Climax UI
        )
        .await?;

    tracing::info!(
        "external o_event recorded id={} session={:?} content={} from {}/scene_{}",
        logged.id,
        session_id,
        content_id,
        payload.source,
        payload.scene_id,
    );
    Ok(())
}

/// Handle an O removal reported by a source (e.g. user decremented the O
/// counter in Stash). Mirrors it into Climax's active session WITHOUT pushing
/// back to Stash — Stash already removed its own entry.
async fn record_external_o_remove(state: &AppState, payload: WsORemovePayload) -> anyhow::Result<()> {
    let content_id = state
        .session
        .ensure_content_item(&payload.source, &payload.scene_id)
        .await?;

    let removed = state.session.remove_external_o(content_id).await?;

    tracing::info!(
        "external o-remove from {}/scene_{}: content={} removed={}",
        payload.source,
        payload.scene_id,
        content_id,
        removed,
    );
    Ok(())
}

async fn active_session(State(state): State<AppState>) -> impl IntoResponse {
    match state.session.active().await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Server-side backup download (Phase 5, for the browser web client - it has no
/// native save dialog, so the SERVER snapshots its own DB and streams it down as
/// a normal browser download). Mirrors the desktop `backup_now` mechanics:
/// `VACUUM INTO` a temp file (hot + consistent, WAL merged), best-effort REINDEX
/// (VACUUM faithfully copies a corrupt index; restore's quick_check wouldn't
/// catch it), profile-stemmed timestamped filename. The temp file is removed
/// after reading.
///
/// Deliberately NOT routed here: only the standalone climax-server mounts this
/// (via `spawn`'s `extra` router), so the desktop's in-process server never
/// exposes a whole-database download endpoint - a drive-by page navigating to
/// localhost:9998/backup would otherwise silently drop the DB into Downloads
/// (same threat class as the /active_session leak closed in Phase 1).
/// Token-gating arrives with the Phase-3 auth middleware.
pub async fn backup_download(State(state): State<AppState>) -> Response {
    // Unique temp suffix: now_ms alone can collide across two same-millisecond
    // requests (the loser's cleanup would delete the winner's snapshot mid-read),
    // so add a process-wide counter.
    static BACKUP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stem = state.db_filename.trim_end_matches(".sqlite");
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let download_name = format!("{stem}-backup-{stamp}.sqlite");
    let tmp = state
        .db_dir
        .join(format!(".{stem}-webbackup-{}-{seq}.sqlite", crate::db::now_ms()));
    let tmp_str = tmp.to_string_lossy().to_string();

    // VACUUM INTO takes a string literal, not a bound param - escape quotes.
    let escaped = tmp_str.replace('\'', "''");
    if let Err(e) = sqlx::query(&format!("VACUUM INTO '{escaped}'")).execute(&state.pool).await {
        tracing::error!("web backup VACUUM INTO failed: {:#}", e);
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Couldn't create a backup. Try again in a moment." })),
        )
            .into_response();
    }

    // Best-effort REINDEX of the snapshot (never sinks the download).
    {
        use sqlx::Connection;
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&tmp)
            .create_if_missing(false);
        match sqlx::SqliteConnection::connect_with(&opts).await {
            Ok(mut conn) => {
                if let Err(e) = sqlx::query("REINDEX").execute(&mut conn).await {
                    tracing::warn!("REINDEX of web backup failed (kept as-is): {:#}", e);
                }
                let _ = conn.close().await;
            }
            Err(e) => tracing::warn!("could not open web backup to REINDEX (kept as-is): {:#}", e),
        }
    }

    let bytes = match std::fs::read(&tmp) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("web backup read failed: {:#}", e);
            let _ = std::fs::remove_file(&tmp);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Couldn't create a backup. Try again in a moment." })),
            )
                .into_response();
        }
    };
    let _ = std::fs::remove_file(&tmp);

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{download_name}\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Exit the process shortly after the current response has flushed, so the
/// supervisor brings the server back up and boot applies whatever was staged
/// (`apply_pending_restore` / `apply_pending_wipe` run before the pool opens).
/// This is a CLEAN exit (code 0), so the supervisor must restart on clean exits
/// too: Docker `restart: unless-stopped` does; systemd needs `Restart=always`
/// (NOT `on-failure`, which ignores code 0 - the shipped unit file says always).
/// A bare unsupervised binary just exits and stays down - deploy/README says so.
fn exit_soon(what: &'static str) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        tracing::info!("exiting to apply the {what}; expecting the supervisor to restart us");
        std::process::exit(0);
    });
}

/// Server-side restore upload (the web client's counterpart to the desktop's
/// pick-a-file restore; a browser can't hand the server a local path, so it
/// uploads the backup's bytes instead). Streams the body to a temp file next to
/// the DB, validates it (`db::validate_db_file` - same checks as desktop),
/// snapshots the CURRENT library as the promised `*-pre-restore-*` rollback
/// (a failure there CANCELS the restore, mirroring the desktop contract), then
/// renames the upload into the staging slot and exits so the supervisor
/// restarts us - boot re-validates and swaps it in before the pool opens.
///
/// Deliberately NOT routed here: only the standalone climax-server mounts this
/// (via `spawn`'s `extra` router), same as `backup_download`, so the desktop's
/// in-process server can never expose a remote DB-replacement endpoint.
///
/// CSRF guard: requires `Content-Type: application/octet-stream`. A cross-origin
/// page can fire a no-preflight POST only with "simple" content types
/// (text/plain etc.), and this server never answers CORS preflights - so a
/// drive-by page can't reach the staging logic even on a tokenless localhost
/// server. (POST /rpc gets the same protection implicitly from its Json
/// extractor's content-type requirement.)
pub async fn restore_upload(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Response {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct.starts_with("application/octet-stream") {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(serde_json::json!({ "error": "Send the backup file as application/octet-stream." })),
        )
            .into_response();
    }

    // Stream to a temp file NEXT TO the DB (same volume, so the staging rename
    // below is atomic). The cap is a SANITY bound (real Climax DBs are MB-scale;
    // 1 GiB is absurd headroom), not DoS protection - that's the token's job.
    const MAX_RESTORE_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB
    static RESTORE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = RESTORE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stem = state.db_filename.trim_end_matches(".sqlite");
    let tmp = state
        .db_dir
        .join(format!(".{stem}-webrestore-{}-{seq}.sqlite", crate::db::now_ms()));

    let mut file = match tokio::fs::File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("web restore: create temp failed: {:#}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Couldn't save the upload. Nothing was changed." })),
            )
                .into_response();
        }
    };
    let mut stream = body.into_data_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("web restore: upload interrupted: {:#}", e);
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "The upload was interrupted. Try again." })),
                )
                    .into_response();
            }
        };
        total += chunk.len() as u64;
        if total > MAX_RESTORE_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "That file is too large to be a Climax backup." })),
            )
                .into_response();
        }
        if let Err(e) = file.write_all(&chunk).await {
            tracing::error!("web restore: write temp failed: {:#}", e);
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Couldn't save the upload. Nothing was changed." })),
            )
                .into_response();
        }
    }
    if let Err(e) = file.flush().await {
        tracing::error!("web restore: flush temp failed: {:#}", e);
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Couldn't save the upload. Nothing was changed." })),
        )
            .into_response();
    }
    drop(file);

    // Validate BEFORE touching anything. The bail! messages are designed
    // sentences ("That file isn't a Climax backup.") that render in the UI.
    if let Err(e) = crate::db::validate_db_file(&tmp.to_string_lossy()).await {
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // Safety snapshot of the CURRENT library - the rollback the UI promises, so
    // a failure CANCELS the restore (desktop contract, backup_restore_apply).
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let safety = state.db_dir.join(format!("{stem}-pre-restore-{stamp}.sqlite"));
    let escaped = safety.to_string_lossy().replace('\'', "''");
    if let Err(e) = sqlx::query(&format!("VACUUM INTO '{escaped}'")).execute(&state.pool).await {
        tracing::error!("web restore: pre-restore safety copy failed: {:#}", e);
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Couldn't write the pre-restore safety copy, so the restore was cancelled." })),
        )
            .into_response();
    }

    // Stage it (profile-stemmed slot; boot re-validates + swaps pre-pool).
    let pending = crate::db::restore_pending_path(&state.db_dir, &state.db_filename);
    if let Err(e) = std::fs::rename(&tmp, &pending) {
        tracing::error!("web restore: staging rename failed: {:#}", e);
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Couldn't stage the restore. Nothing was changed." })),
        )
            .into_response();
    }

    tracing::info!("web restore staged ({total} bytes); restarting to apply");
    exit_soon("staged restore");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

/// Server-side full reset (the web client's Danger zone). Stages the wipe
/// marker and exits; the supervisor restarts us and boot deletes the DB before
/// the pool opens, recreating a pristine one - the headless analog of the
/// desktop's `backup_reset` + `app.restart()`.
///
/// Deliberately NOT routed here (climax-server only, like `backup_download`).
/// CSRF guard: requires the custom `X-Climax-Reset` header, which makes the
/// request non-"simple" - a cross-origin page would need a CORS preflight this
/// server never approves, so a drive-by page can't wipe a tokenless localhost
/// server. The web client always sends it.
pub async fn reset_server(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !headers.contains_key("x-climax-reset") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing the X-Climax-Reset header." })),
        )
            .into_response();
    }
    if let Err(e) = crate::db::stage_wipe(&state.db_dir, &state.db_filename) {
        tracing::error!("web reset: staging wipe failed: {:#}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Couldn't stage the reset. Nothing was deleted." })),
        )
            .into_response();
    }
    tracing::warn!("web reset staged; restarting to apply");
    exit_soon("staged reset");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

/// Body of a POST /rpc call: a command name + its named args (the same object
/// the frontend's transport-agnostic `call()` sends, identical to a Tauri
/// invoke's args).
#[derive(Debug, Deserialize)]
struct RpcRequest {
    cmd: String,
    #[serde(default)]
    args: serde_json::Value,
}

/// Transport-agnostic command endpoint (Phase 1 client-server pivot). A browser
/// / web client POSTs `{cmd, args}` here to reach the same backend commands the
/// desktop app invokes over Tauri IPC. Success -> 200 with the raw result JSON;
/// failure -> 400 with `{error}`. Mirrors how Tauri's `invoke` resolves/rejects.
/// Bound to 127.0.0.1 like the rest of the server for now; LAN exposure + auth
/// arrive in a later phase.
async fn rpc(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    match crate::rpc::rpc_dispatch(&state, &req.cmd, &req.args).await {
        Ok(val) => Json(val).into_response(),
        Err(msg) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": msg })),
        )
            .into_response(),
    }
}

/// Whether THIS Climax serves a browser-openable web UI. Derived in `spawn`
/// from `opts.extra.is_some()` (only the standalone climax-server mounts the
/// web router), so it can never disagree with reality.
///
/// Told to the bridge in its `hello` frame: the bridge's stop click needs a
/// wrap-up UI, and a headless server has no window to raise. When true the
/// bridge opens the wrap-up in a browser tab at the same origin it's already
/// connected to; when false it sends `request_stop` for the desktop app to
/// handle natively, as before.
#[derive(Clone, Copy)]
pub struct WebUiAvailable(pub bool);

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    axum::Extension(web_ui): axum::Extension<WebUiAvailable>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, web_ui))
}

async fn events_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_events_socket(socket, state))
}

/// Server -> client EVENT push channel (Phase 2 app-as-client). A connected
/// client - the desktop app in client mode, or a future web client - receives
/// the server's `UiSignal`s here as tagged JSON: passive-capture prompts +
/// bridge-pill requests (show tracker / request stop), the things the in-process
/// `lib.rs` reactor performs locally. The client reacts the same way over the
/// wire. This is SEPARATE from the bridge's `/ws`: it does NOT bump `bridge_conns`
/// (so bridge-detection isn't fooled by a connected app/web client), and it's a
/// pure server->client PUSH - inbound frames are ignored (bar Close). UiSignals
/// are internally tagged (`{"type":"show_tracker"|"tracker_stop_requested"|
/// "capture_prompt", ...}`), so the client matches on `type`.
async fn handle_events_socket(socket: WebSocket, state: AppState) {
    use futures_util::{SinkExt, StreamExt};
    tracing::info!("events client connected");
    let (mut sink, mut stream) = socket.split();
    let mut ui_rx = state.ui_tx.subscribe();

    let _ = sink
        .send(Message::Text(
            serde_json::json!({ "type": "hello", "service": "climax", "channel": "events" })
                .to_string(),
        ))
        .await;

    let forward = tokio::spawn(async move {
        loop {
            match ui_rx.recv().await {
                Ok(sig) => {
                    if let Ok(txt) = serde_json::to_string(&sig) {
                        if sink.send(Message::Text(txt)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }
    forward.abort();
    tracing::info!("events client disconnected");
}

/// Bumps `AppState.bridge_conns` for the lifetime of one `/ws` connection so the
/// count reflects live bridges even if the handler task unwinds. Decrements on
/// drop (the single exit path of `handle_socket`).
struct BridgeConnGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl BridgeConnGuard {
    fn new(counter: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BridgeConnGuard(counter)
    }
}
impl Drop for BridgeConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, web_ui: WebUiAvailable) {
    use futures_util::{SinkExt, StreamExt};
    let _conn_guard = BridgeConnGuard::new(state.bridge_conns.clone());
    tracing::info!("ws client connected");

    // Split so the server can PUSH (session-state) AND reply (acks) while the
    // recv loop handles inbound. A single forward task owns the sink; the recv
    // loop queues replies via `out_tx`, and session-state pushes arrive on the
    // broadcast. No polling anywhere.
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let mut bcast_rx = state.ws_tx.subscribe();

    // Greet + send the current tracking state immediately, so the bridge's
    // navbar indicator is correct the moment it connects.
    let _ = out_tx.send(Message::Text(
        serde_json::json!({ "type": "hello", "service": "climax", "web_ui": web_ui.0 })
            .to_string(),
    ));
    let _ = out_tx.send(Message::Text(state.session.session_state_json().await));

    let forward = tokio::spawn(async move {
        loop {
            tokio::select! {
                m = out_rx.recv() => match m {
                    Some(msg) => { if sink.send(msg).await.is_err() { break; } }
                    None => break,
                },
                b = bcast_rx.recv() => match b {
                    Ok(txt) => { if sink.send(Message::Text(txt)).await.is_err() { break; } }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    });

    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("ws recv error: {}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<WsIn>(&text) {
                    Ok(WsIn::Heartbeat(payload)) => {
                        tracing::info!(
                            "ws heartbeat: source={} tab={} scene={} state={} ct={:.2}",
                            payload.source,
                            payload.tab_id,
                            payload.scene_id,
                            payload.video.state,
                            payload.video.current_time,
                        );
                        match state.session.record_heartbeat(payload).await {
                            Ok(rec) => {
                                let reply = serde_json::json!({
                                    "type": "ack",
                                    "session_active": rec.session_id.is_some(),
                                    "session_id": rec.session_id,
                                    "recorded": rec.recorded,
                                });
                                let _ = out_tx.send(Message::Text(reply.to_string()));
                            }
                            Err(e) => {
                                tracing::error!("ws record_heartbeat error: {:#}", e);
                                let _ = out_tx.send(Message::Text(
                                    serde_json::json!({ "type": "error", "message": e.to_string() })
                                        .to_string(),
                                ));
                            }
                        }
                    }
                    Ok(WsIn::OEvent(payload)) => {
                        tracing::info!(
                            "ws o_event: source={} scene={}",
                            payload.source, payload.scene_id
                        );
                        match record_external_o(&state, payload).await {
                            Ok(()) => {
                                let _ = out_tx.send(Message::Text(
                                    serde_json::json!({ "type": "o_ack", "ok": true }).to_string(),
                                ));
                            }
                            Err(e) => {
                                tracing::error!("ws external o_event error: {:#}", e);
                            }
                        }
                    }
                    Ok(WsIn::ORemove(payload)) => {
                        tracing::info!(
                            "ws o_remove: source={} scene={}",
                            payload.source, payload.scene_id
                        );
                        match record_external_o_remove(&state, payload).await {
                            Ok(()) => {
                                let _ = out_tx.send(Message::Text(
                                    serde_json::json!({ "type": "o_remove_ack", "ok": true }).to_string(),
                                ));
                            }
                            Err(e) => {
                                tracing::error!("ws external o_remove error: {:#}", e);
                            }
                        }
                    }
                    Ok(WsIn::OpenTracker) => {
                        tracing::info!("ws open_tracker request from bridge");
                        // Mirror the hotkey: start a session if none is running
                        // (never stop one), then ask the client to show the tracker.
                        let active = state.session.active().await.unwrap_or(None);
                        if active.is_none() {
                            if let Err(e) = state.session.start().await {
                                tracing::error!("ws open_tracker: start failed: {:#}", e);
                            }
                        }
                        let _ = state.ui_tx.send(UiSignal::ShowTracker);
                    }
                    Ok(WsIn::RequestStop) => {
                        tracing::info!("ws request_stop from bridge");
                        // Ask the client to show the tracker + run the same Stop
                        // flow as the in-app button (opens the wrap-up modal; "go
                        // back" keeps the session running, so a misclick is safe).
                        let _ = state.ui_tx.send(UiSignal::ShowTracker);
                        let _ = state.ui_tx.send(UiSignal::TrackerStopRequested);
                    }
                    Ok(WsIn::StartSession) => {
                        tracing::info!("ws start_session from bridge");
                        // Same start as open_tracker (never stops one), minus the
                        // window signal. `start` broadcasts the new session_state,
                        // so the pill updates itself.
                        let active = state.session.active().await.unwrap_or(None);
                        if active.is_none() {
                            if let Err(e) = state.session.start().await {
                                tracing::error!("ws start_session: start failed: {:#}", e);
                            }
                        }
                    }
                    Ok(WsIn::OpenClimax) => {
                        tracing::info!("ws open_climax from bridge");
                        // Dashboard, not the tracker: the same menu option against
                        // a server opens the web dashboard, so both should land on
                        // the same surface. Never starts a session (that's what
                        // open_tracker is for).
                        let _ = state.ui_tx.send(UiSignal::ShowDashboard);
                    }
                    Ok(WsIn::PauseSession) => {
                        tracing::info!("ws pause_session from bridge");
                        // Both broadcast the new session_state on success, so
                        // the pill updates itself with no reply needed.
                        if let Err(e) = state.session.pause("manual").await {
                            tracing::warn!("ws pause_session failed: {:#}", e);
                        }
                    }
                    Ok(WsIn::ResumeSession) => {
                        tracing::info!("ws resume_session from bridge");
                        if let Err(e) = state.session.resume().await {
                            tracing::warn!("ws resume_session failed: {:#}", e);
                        }
                    }
                    Ok(WsIn::SetOrganising { on }) => {
                        tracing::info!("ws set_organising({}) from bridge", on);
                        // Toggles Stash's play-history tracking + flips the
                        // navbar pill (set_organising broadcasts the new
                        // session_state to every socket via ws_tx, so no reply
                        // is needed here). Surfaces an error reply if refused
                        // (e.g. asked to organise while a session is active).
                        if let Err(e) = state.session.set_organising(on).await {
                            tracing::warn!("set_organising failed: {:#}", e);
                            let _ = out_tx.send(Message::Text(
                                serde_json::json!({ "type": "error", "message": e.to_string() })
                                    .to_string(),
                            ));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("invalid ws json: {} - body: {}", e, text);
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(p) => {
                let _ = out_tx.send(Message::Pong(p));
            }
            _ => {}
        }
    }

    forward.abort();
    tracing::info!("ws client disconnected");
}

#[cfg(test)]
mod bind_tests {
    use super::*;

    fn localhost() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    /// The normal case must not pay for the retry loop.
    #[tokio::test]
    async fn binds_a_free_port_immediately() {
        let probe = tokio::net::TcpListener::bind(localhost()).await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let started = std::time::Instant::now();
        let listener = bind_with_retry(addr).await.expect("free port should bind");
        assert_eq!(listener.local_addr().unwrap(), addr);
        assert!(
            started.elapsed() < BIND_RETRY_DELAY,
            "a free port must bind on the first attempt, took {:?}",
            started.elapsed()
        );
    }

    /// THE REGRESSION THIS EXISTS FOR: a restart hands the port over a moment
    /// after the replacement process starts. Binding once fails here; waiting
    /// briefly succeeds.
    #[tokio::test]
    async fn waits_for_a_predecessor_to_let_go() {
        let held = tokio::net::TcpListener::bind(localhost()).await.unwrap();
        let addr = held.local_addr().unwrap();

        // Prove the single-shot bind that shipped before genuinely fails here,
        // so this test cannot pass for the wrong reason.
        assert_eq!(
            tokio::net::TcpListener::bind(addr).await.unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            drop(held);
        });

        let listener = bind_with_retry(addr)
            .await
            .expect("should have waited for the port to free up");
        assert_eq!(listener.local_addr().unwrap(), addr);
    }

    /// A port that never frees still fails, and still reports why - the retry
    /// must not turn a real conflict into a hang or a misleading success.
    #[tokio::test]
    async fn gives_up_on_a_port_that_never_frees() {
        let _held = tokio::net::TcpListener::bind(localhost()).await.unwrap();
        let addr = _held.local_addr().unwrap();

        let started = std::time::Instant::now();
        let err = bind_with_retry(addr).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        assert!(
            started.elapsed() >= BIND_RETRY_DELAY,
            "should have retried at least once before giving up"
        );
        assert!(
            started.elapsed() < BIND_RETRY_WINDOW * 2,
            "must not overrun its own window, took {:?}",
            started.elapsed()
        );
    }
}
