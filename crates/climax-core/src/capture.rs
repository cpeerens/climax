// Passive-capture prompt (Phase 7 component 3).
//
// While NO session is running, this loop watches for "you're watching a scene
// but not tracking it" across BOTH paths:
//   - LOCAL: the Stash bridge reports playback into SessionManager's no-session
//     watch cache (record_heartbeat); find_capture_candidate reads it.
//   - REMOTE (TV / phone / etc.): the bridge can't see other devices, so we ALSO
//     poll Stash's per-scene play_duration (throttled) and detect a scene
//     advancing past the threshold with no session.
//
// Once a scene crosses the threshold it fires a one-shot "want to track this?"
// nudge: stash a CapturePayload, emit `capture_prompt`, and show the tracker
// (which renders a bottom-right toast). Accept starts a session BACKDATED to
// when watching began; snooze suppresses re-firing for the grace. Gated on the
// setting + no active session + not snoozed + not already pending. Hosted in the
// tracker window (not its own) for the same IPC reason as the idle prompt.

use std::collections::HashMap;
use std::time::Duration;

use crate::db::now_ms;
use crate::models::{CapturePayload, UiSignal};
use crate::settings;
use crate::state::AppState;

/// How often the loop runs (the cheap, in-memory local check cadence).
const POLL_INTERVAL_SECS: u64 = 8;
/// A LOCAL watch must have advanced within this window to count as still playing.
const FRESH_MS: i64 = 15_000;
/// Minimum spacing between Stash play_duration polls (remote detection). The
/// threshold is in minutes, so coarse sampling is fine and keeps idle load low.
const STASH_POLL_MS: i64 = 28_000;
/// A REMOTE scene must have advanced within ~2 polls to count as still playing.
const STASH_FRESH_MS: i64 = 70_000;
/// Float-noise / real-advance threshold for play_duration deltas.
const ADVANCE_EPSILON: f64 = 0.5;
/// Reset a remote watch streak if it stalls longer than this (a pause).
const REMOTE_GAP_MS: i64 = 60_000;

/// Per-run state for REMOTE (Stash-poll) detection while no session runs. The
/// LOCAL path uses SessionManager's no_session_watch instead.
#[derive(Default)]
struct CaptureState {
    tv: HashMap<String, TvWatch>,
    last_stash_poll: i64,
}

#[derive(Clone)]
struct TvWatch {
    pd: f64,
    /// When this advancing streak began (the backdated start if accepted).
    first_advancing: i64,
    /// Last poll where play_duration advanced.
    last_advance: i64,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // Brief settle so the first poll doesn't compete with boot work.
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut cs = CaptureState::default();
        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            if let Err(e) = tick(&state, &mut cs).await {
                tracing::warn!("capture tick failed: {:#}", e);
            }
        }
    });
}

async fn tick(state: &AppState, cs: &mut CaptureState) -> anyhow::Result<()> {
    let cfg = settings::get_capture_prompt(&state.pool)
        .await
        .unwrap_or_default();
    if !cfg.enabled {
        cs.tv.clear();
        return Ok(());
    }
    // A prompt is already showing — leave it until resolved.
    if state.pending_capture.read().await.is_some() {
        return Ok(());
    }
    // Snoozed.
    if now_ms() < *state.capture_snooze_until.read().await {
        return Ok(());
    }
    // A session is already running — nothing to capture; drop remote tracking.
    if state.session.active_id().await?.is_some() {
        cs.tv.clear();
        return Ok(());
    }
    // Organising mode: the user is deliberately tidying their Stash library with
    // play-history tracking off. Scrubbing scenes then would fire a bogus
    // "track this?" prompt, so stay quiet until they leave organising.
    if state.session.is_organising() {
        cs.tv.clear();
        return Ok(());
    }

    let threshold_ms = (cfg.threshold_minutes as i64).max(1) * 60_000;

    // LOCAL (bridge) candidate first; else REMOTE (Stash poll, throttled).
    let candidate = match state
        .session
        .find_capture_candidate(threshold_ms, FRESH_MS)
        .await
    {
        Some(c) => Some(c),
        None => {
            let now = now_ms();
            if now - cs.last_stash_poll >= STASH_POLL_MS {
                cs.last_stash_poll = now;
                detect_remote_candidate(state, cs, threshold_ms).await
            } else {
                None
            }
        }
    };
    let Some((source, scene_id, content_item_id, watch_started_at)) = candidate else {
        return Ok(());
    };

    // Title/thumbnail for the toast. The background enrichment kicked off by the
    // remote detector (and the live bridge sighting) may not have landed yet for a
    // FIRST-SIGHT scene, leaving title NULL. Rather than fall back to "Scene <id>",
    // fetch it synchronously here so the toast shows the real card. Only a true
    // first sight pays the round-trip; an already-enriched scene reads from the DB.
    let (mut title, mut thumbnail_url): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT title, thumbnail_url FROM content_items WHERE id = ?1")
            .bind(content_item_id)
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or((None, None));
    if title.as_deref().unwrap_or("").trim().is_empty() && source == "stash" {
        if let Ok(Some(info)) = crate::stash::fetch_scene(&state.pool, &scene_id).await {
            if info.title.is_some() {
                title = info.title;
            }
            if info.thumbnail_url.is_some() {
                thumbnail_url = info.thumbnail_url;
            }
        }
    }

    let payload = CapturePayload {
        source,
        scene_id,
        content_item_id,
        watch_started_at,
        title,
        thumbnail_url,
    };

    {
        let mut slot = state.pending_capture.write().await;
        *slot = Some(payload.clone());
    }
    tracing::info!(
        "capture prompt: scene {} watched >= {}m with no session (backdate {})",
        payload.scene_id,
        cfg.threshold_minutes,
        watch_started_at
    );
    // Hand off to the client reactor (lib.rs), which holds the AppHandle: it
    // emits `capture_prompt` + raises the tracker (bottom-right toast, no focus
    // steal). The detector core never touches Tauri.
    let _ = state.ui_tx.send(UiSignal::CapturePrompt(payload));
    Ok(())
}

/// Poll Stash play_duration and detect a REMOTE scene (not locally bridged)
/// watched past the threshold with no session. Returns (source, scene_id,
/// content_item_id, watch_started_at) — watch_started_at is when the advancing
/// streak began (the backdated session start if accepted).
async fn detect_remote_candidate(
    state: &AppState,
    cs: &mut CaptureState,
    threshold_ms: i64,
) -> Option<(String, String, i64, i64)> {
    let rows = crate::stash::find_active_play_durations(&state.pool).await.ok()?;
    // Exclude scenes the local bridge is reporting — those go through the LOCAL
    // path (and would otherwise fire as a phantom "remote" watch).
    let local = state
        .session
        .live_tracked_external_ids(20_000)
        .await
        .unwrap_or_default();
    let now = now_ms();

    let mut candidate: Option<(String, String, i64, i64)> = None;
    let mut next: HashMap<String, TvWatch> = HashMap::with_capacity(rows.len());
    for (scene_id, pd_now) in rows {
        let prev = cs.tv.get(&scene_id);
        let advanced = prev.is_some_and(|p| pd_now - p.pd > ADVANCE_EPSILON);
        let (first_advancing, last_advance) = match prev {
            Some(p) if advanced => {
                if now - p.last_advance <= REMOTE_GAP_MS {
                    (p.first_advancing, now) // continuous streak
                } else {
                    (now, now) // resumed after a long pause -> new streak
                }
            }
            Some(p) => (p.first_advancing, p.last_advance), // not advancing -> hold
            None => (now, 0),                               // first sight, no delta yet
        };

        // A remote scene watched past the threshold, still playing, not local.
        if candidate.is_none()
            && !local.contains(&scene_id)
            && now - last_advance <= STASH_FRESH_MS
            && last_advance - first_advancing >= threshold_ms
        {
            if let Ok(cid) = state.session.ensure_content_item("stash", &scene_id).await {
                // Fill the card title/thumbnail (async + debounced).
                state
                    .session
                    .refresh_stash_metadata(cid, scene_id.clone(), "stash", false)
                    .await;
                candidate = Some(("stash".to_string(), scene_id.clone(), cid, first_advancing));
            }
        }
        next.insert(
            scene_id,
            TvWatch {
                pd: pd_now,
                first_advancing,
                last_advance,
            },
        );
    }
    cs.tv = next;
    candidate
}
