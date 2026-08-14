// Shared data types used by the database layer, HTTP server, Tauri commands,
// and the frontend (via serde_json).

use serde::{Deserialize, Serialize};

/// A signal from the (eventually headless) server to a desktop CLIENT, asking it
/// to perform a native action the server itself can't - raise/focus a window,
/// run the wrap-up flow, surface the capture toast. Phase 2 decoupling: instead
/// of the server reaching into an `AppHandle`, it broadcasts these on
/// `AppState.ui_tx`; a client-side task (which holds the handle) reacts.
/// Serializable so it can travel to a separate client process over the wire (2b).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiSignal {
    /// Raise + focus the tracker window.
    ShowTracker,
    /// Raise + focus the DASHBOARD window. The bridge pill's "Open Climax" uses
    /// this, matching what the same option does against a server (which opens
    /// the web dashboard) - "open Climax" should land on the same surface either
    /// way, not the compact tracker widget.
    ShowDashboard,
    /// Raise the tracker and run its Stop / wrap-up flow (the bridge pill's
    /// "request stop" - a misclick is non-destructive, it just opens the modal).
    TrackerStopRequested,
    /// Surface the passive-capture toast for this candidate. Carries the payload
    /// so the client can render it without a round-trip; the detector also writes
    /// it to `AppState.pending_capture` (which the toast also reads on mount).
    CapturePrompt(CapturePayload),
}

/// Status of a session at the data layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Paused,
    Ended,
    Discarded,
}

// Status is stored/compared as a string on the Session struct; these helpers
// are kept as the canonical mapping for future typed use.
#[allow(dead_code)]
impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Paused => "paused",
            SessionStatus::Ended => "ended",
            SessionStatus::Discarded => "discarded",
        }
    }

    // NOT `std::str::FromStr`: that trait must return Result, and an unknown
    // status here is an absence rather than an error - every caller wants the
    // Option. Renaming it to satisfy the lint would be worse than the lint.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "ended" => Some(Self::Ended),
            "discarded" => Some(Self::Discarded),
            _ => None,
        }
    }
}

/// One session row, shape that the frontend consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub last_heartbeat: Option<i64>,
    pub status: String,
    pub notes: Option<String>,
    pub excluded: bool,
    pub session_type: Option<String>,
    /// True for reconstructed / best-guess sessions (built from Stash history by
    /// the reconstruction engine or the first-launch import), false for
    /// live-tracked ones. Drives the "estimated" badge in the Sessions table.
    pub estimated: bool,
    /// User-attributable day (YYYY-MM-DD, local time). Defaults to the local
    /// date at started_at. Wrap-up modal can override on cross-midnight sessions.
    pub assigned_day: String,
    /// Computed: total elapsed minus paused, in milliseconds.
    pub effective_duration_ms: i64,
    /// Computed: number of scenes attached.
    pub scene_count: i64,
    /// Computed: number of O events.
    pub o_count: i64,
}

/// Column the Sessions table is sorted by. Parsed server-side from a string
/// so the SQL ORDER BY fragment is always one of a fixed set (no injection).
#[derive(Debug, Clone, Copy)]
pub enum SessionSort {
    Start,
    Duration,
    Scenes,
    Cumshots,
}

impl SessionSort {
    /// Map a frontend sort key to the enum, defaulting unknowns to `Start`.
    pub fn parse(s: &str) -> Self {
        match s {
            "duration" => Self::Duration,
            "scenes" => Self::Scenes,
            "cumshots" => Self::Cumshots,
            _ => Self::Start,
        }
    }
}

/// One page of sessions for the dashboard Sessions table: the page rows plus
/// the total count of matching sessions (for pagination). `total` reflects the
/// same WHERE filter as `rows` but ignores limit/offset.
#[derive(Debug, Clone, Serialize)]
pub struct SessionPage {
    pub rows: Vec<Session>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentItem {
    pub id: i64,
    pub source_id: i64,
    pub source_key: String,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_seconds: Option<i64>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

// Serde shape kept for the planned scene-play API surface; not constructed yet.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePlay {
    pub id: i64,
    pub session_id: i64,
    pub content_item_id: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub seconds_tracked: i64,
    pub content: ContentItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OEvent {
    pub id: i64,
    pub session_id: Option<i64>,
    pub content_item_id: Option<i64>,
    pub occurred_at: i64,
    pub intensity: Option<i64>,
    pub notes: Option<String>,
    pub stash_synced: bool,
    /// 'climax' | 'stash' | 'manual'. Determines whether Climax UI can delete
    /// the event (only 'climax' is removable from inside Climax).
    pub origin: String,
}

/// Payload accepted at POST /heartbeat from the Stash bridge plugin.
///
/// Slim protocol: the bridge tells us WHICH scene is being watched and HOW
/// the video element is behaving. Everything else (title, performers,
/// studio, etc) is fetched by Climax via Stash GraphQL — see
/// `stash::fetch_scene` + `SessionManager::refresh_stash_metadata`.
///
/// `serde` silently drops unknown fields, so a slightly-newer bridge sending
/// extra keys won't break this; a slightly-older bridge sending now-removed
/// keys (`scene_title`, `scene_url`, `video.duration`) is also fine — they
/// just get ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatPayload {
    pub source: String, // "stash" today; extensible later
    pub tab_id: String,
    pub scene_id: String,
    pub video: HeartbeatVideo,
    /// Bridge's send timestamp; deserialized for protocol completeness, unused.
    #[allow(dead_code)]
    pub sent_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatVideo {
    /// What the bridge reports based on video.paused. NOT authoritative — some
    /// plugins (Floating Scene Player) make the UI show paused while the bare
    /// video element keeps state=playing. We use `current_time` advancement as
    /// the real source of truth for whether the video is actually playing.
    pub state: String,
    pub current_time: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatResponse {
    pub session_active: bool,
    pub session_id: Option<i64>,
    pub recorded: bool,
}

/// Payload describing an in-progress idle stretch. Fired pre-emptively by
/// `presence.rs` the moment the user crosses the idle threshold, and kept on
/// AppState so the floating idle-prompt window can fetch it on mount.
///
/// Note: `idle_started_at` is the user's last-input wall-clock time before
/// going AFK — i.e. the natural "session end time" if they pick Discard & End.
#[derive(Debug, Clone, Serialize)]
pub struct IdlePayload {
    pub session_id: i64,
    pub idle_started_at: i64,
}

/// Payload for the passive-capture prompt (Phase 7 component 3): the bridge saw
/// a scene watched past the threshold with NO session running. Fired by
/// `capture.rs`, kept on AppState, and read by the tracker's bottom-right toast.
/// `watch_started_at` is when Climax first saw playback begin — the backdated
/// session start if the user accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturePayload {
    pub source: String,
    pub scene_id: String,
    pub content_item_id: i64,
    pub watch_started_at: i64,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
}

/// Payload for the crash-recovery prompt: on boot Climax found a session left
/// `active`/`paused` from a previous run that never wrapped up (crash, force-kill,
/// power loss, OS shutdown). Set by the boot check in `lib.rs` when the gap since
/// the last heartbeat exceeds the recovery threshold, kept on AppState, and read
/// by the tracker on mount. `gap_started_at` is the last heartbeat = the natural
/// "session end time" if the user picks Discard & End (mirrors IdlePayload).
#[derive(Debug, Clone, Serialize)]
pub struct CrashPayload {
    pub session_id: i64,
    pub gap_started_at: i64,
}
