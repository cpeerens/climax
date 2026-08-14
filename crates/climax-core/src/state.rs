// Shared application state - injected into Tauri commands and the HTTP server.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::{broadcast, RwLock};

use crate::mirror::MirrorStatus;
use crate::models::{CapturePayload, CrashPayload, IdlePayload, UiSignal};
use crate::session::SessionManager;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub session: Arc<SessionManager>,
    /// The currently-active idle prompt's payload, if presence.rs has detected
    /// an idle stretch that hasn't been resolved yet. The idle-prompt window
    /// reads this on mount to populate the modal; resolve commands clear it.
    pub pending_idle: Arc<RwLock<Option<IdlePayload>>>,
    /// Live progress of the play-history mirror sync (Phase 7). The Settings
    /// "Play history" section polls it.
    pub mirror_status: Arc<RwLock<MirrorStatus>>,
    /// Live progress of the library-metadata sync (scene + entity details).
    /// Separate from mirror_status so the two Settings sections poll
    /// independently.
    pub metadata_status: Arc<RwLock<MirrorStatus>>,
    /// The pending passive-capture prompt, if `capture.rs` has detected
    /// untracked watching. The tracker's bottom-right toast reads this on mount;
    /// accept/snooze commands clear it.
    pub pending_capture: Arc<RwLock<Option<CapturePayload>>>,
    /// Wall-clock ms until which capture prompts are suppressed (set when the
    /// user snoozes/dismisses a prompt). 0 = not snoozed.
    pub capture_snooze_until: Arc<RwLock<i64>>,
    /// True while a Quit was requested with a session running and the user
    /// hasn't resolved the wrap-up-or-go-back prompt yet. The tray Quit handler
    /// sets it (instead of exiting); the tracker reads it on mount + listens for
    /// the `quit_requested` event; cancel_quit clears it, confirm_quit exits.
    pub pending_quit: Arc<RwLock<bool>>,
    /// Set on boot when a session was left running from a previous run (crash /
    /// force-kill / power loss) and the gap since its last heartbeat exceeds the
    /// recovery threshold. The tracker reads it on mount and shows the recovery
    /// prompt (same flow as idle return); the crash resolve commands clear it.
    pub pending_crash_recovery: Arc<RwLock<Option<CrashPayload>>>,
    /// Broadcasts `session_state` JSON to connected WS clients (the Stash
    /// bridge's navbar indicator). SessionManager fires it on every status
    /// change; each WS handler subscribes. Event-driven — no polling.
    pub ws_tx: broadcast::Sender<String>,
    /// Broadcasts `UiSignal`s for native actions the server core can't perform
    /// itself (raise/focus a window, run the wrap-up flow). A client-side task
    /// that holds the `AppHandle` subscribes and acts - so the server never
    /// touches Tauri. (Phase 2 decoupling; replaced a direct `app: AppHandle`.)
    pub ui_tx: broadcast::Sender<UiSignal>,
    /// Directory holding the SQLite file(s) (= app_data_dir). The backup/restore
    /// commands resolve the live DB + staged-restore paths from here.
    pub db_dir: PathBuf,
    /// The active profile's DB filename ("climax.sqlite" for real,
    /// "climax-test.sqlite" for the scratch profile). Fixed at boot.
    pub db_filename: String,
    /// Whether this profile talks to Stash at all. False in the scratch profile,
    /// so test sessions never push to / pull from the real Stash library. A
    /// boot-time snapshot (the profile can't change without a relaunch).
    pub stash_enabled: bool,
    /// When set, this app is a thin CLIENT of an external Climax server at this
    /// origin (Phase 2c) - the session / DB / Stash sync live there, not here.
    /// `None` = the default in-process backend (this AppState owns everything).
    /// A boot snapshot (read from client.json; changing it needs a relaunch).
    /// The native desktop loops (tray / hotkey / presence / shutdown-guard) read
    /// it to decide whether to drive the LOCAL session manager or route session
    /// reads/commands over HTTP to the server (see the shell's `remote_session`).
    /// Always `None` in the standalone server (it never runs as a client).
    pub client_server_url: Option<String>,
    /// Shared auth token for that server (Phase 3), if it requires one. Sent as
    /// X-Climax-Token by the Rust-side session proxy (the shell's
    /// `remote_session`). Only meaningful alongside `client_server_url`; always
    /// `None` in the standalone server.
    pub client_server_token: Option<String>,
    /// The version of the BINARY that built this state, e.g. "0.1.0". Passed in
    /// rather than read from a constant because core is a library: `env!` here
    /// would report climax-core's own version, and the three crates carry
    /// independent version literals. The desktop shell passes its Tauri package
    /// version (the one that brands the installer); the standalone server passes
    /// its own. Surfaced by the update check so a client can say what it is
    /// running - and, in client mode, what the server it is attached to is.
    pub app_version: String,
    /// Count of currently-connected `/ws` clients. Only the Stash bridge plugin
    /// connects to that socket, so a count > 0 means the bridge is live in an
    /// open Stash tab. `handle_socket` increments on connect / decrements on
    /// disconnect; onboarding's bridge step reads it for the "● live" cue.
    pub bridge_conns: Arc<AtomicUsize>,
}

impl AppState {
    // Nine arguments, deliberately. These are the process-wide facts settled once
    // at boot (pool, channels, profile, paths, client-mode target, version), and
    // they are set in three different places - the desktop shell, the standalone
    // server, and tests. A builder would let a caller forget one; the compiler
    // catching a missing argument is worth more here than the lint.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        ws_tx: broadcast::Sender<String>,
        ui_tx: broadcast::Sender<UiSignal>,
        db_dir: PathBuf,
        db_filename: String,
        stash_enabled: bool,
        client_server_url: Option<String>,
        client_server_token: Option<String>,
        app_version: String,
    ) -> Self {
        let session = Arc::new(SessionManager::new(pool.clone(), ws_tx.clone()));
        Self {
            pool,
            session,
            pending_idle: Arc::new(RwLock::new(None)),
            mirror_status: Arc::new(RwLock::new(MirrorStatus::default())),
            metadata_status: Arc::new(RwLock::new(MirrorStatus::default())),
            pending_capture: Arc::new(RwLock::new(None)),
            capture_snooze_until: Arc::new(RwLock::new(0)),
            pending_quit: Arc::new(RwLock::new(false)),
            pending_crash_recovery: Arc::new(RwLock::new(None)),
            ws_tx,
            ui_tx,
            db_dir,
            db_filename,
            stash_enabled,
            client_server_url,
            client_server_token,
            app_version,
            bridge_conns: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Whether a Stash bridge is currently connected over the `/ws` socket.
    pub fn bridge_connected(&self) -> bool {
        self.bridge_conns.load(Ordering::Relaxed) > 0
    }
}
