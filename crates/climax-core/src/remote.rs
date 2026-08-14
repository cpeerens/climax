// Live remote-device playback detection (Phase 7).
//
// The Stash bridge plugin only sees scenes played in the Stash WEB UI on THIS
// computer. Scenes played on OTHER devices (e.g. the StashAppAndroidTV app on a
// TV) report their activity straight to Stash: that app calls
// `sceneSaveActivity` ~every 10s of playback, advancing Stash's per-scene
// `play_duration` (cumulative, monotonic watch time) in real time.
//
// So while a session is active, this poller asks Stash for every played scene's
// `play_duration`, diffs it against the previous tick, and credits any scene
// whose value ADVANCED as watch time on the active session — exactly as if the
// bridge had reported it. Remote scenes then appear in the tracker's
// current-session cards and fold into the session like local ones, which is the
// whole point: watch on the TV, see it live on the computer.
//
// De-dup (the one correctness catch): a scene played LOCALLY also advances
// Stash's play_duration, and the bridge already credits it. So each tick we
// EXCLUDE every scene the live bridge is currently reporting
// (`session.live_tracked_external_ids`). Net rule: the bridge owns the local
// machine; the poller only ever adds scenes Stash sees but the bridge doesn't
// (i.e. other devices).
//
// Accuracy at the edges (two fixes, since a diff poller naturally loses one
// interval at each end of a watch):
//   * HEAD START — when a scene first appears AFTER the session baseline, it was
//     un-watched at session start, so its whole current play_duration is this
//     session's watch. We credit that full amount on first detection (not 0),
//     capped at the session's age. That recovers the watch between scene-start
//     and the first poll that noticed it.
//   * TAIL FLUSH — when the session ends, the watch between the last poll and
//     Stop hasn't been credited yet (play_duration froze when playback stopped,
//     but no later poll diffed it). On the first tick after the session ends we
//     do one final pass and credit that remainder to the now-ended session.
//
// Gated on the `remote_tracking` setting AND an ACTIVE session. When no session
// is active (or the feature is off) the poller makes ZERO Stash calls.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::settings;
use crate::state::AppState;

/// How fresh a bridge tab's last heartbeat must be for its scene to count as
/// "the local bridge owns this right now" (and thus be excluded from remote
/// crediting). Bridge cadence is 5s; 20s tolerates a couple of missed beats.
const LOCAL_FRESH_MS: i64 = 20_000;

/// Minimum play_duration growth (seconds) between ticks to read as a real
/// advance. Guards against Stash's float noise (e.g. 2698.7120000000014) being
/// mistaken for playback.
const ADVANCE_EPSILON: f64 = 0.5;

/// Settle delay before the first poll so it doesn't compete with boot work.
const STARTUP_DELAY_SECS: u64 = 25;

/// Per-run poller memory. Reset whenever the tracked session changes.
#[derive(Default)]
struct PollerState {
    /// Last tick's play_duration per scene_id — the diff anchor.
    prev: HashMap<String, f64>,
    /// The active session we're tracking. A change triggers a tail flush of the
    /// old one + a fresh baseline for the new one.
    last_session: Option<i64>,
    /// False until the first poll of a session has captured the baseline. The
    /// baseline tick credits nothing (a single snapshot can't tell this-session
    /// watch from pre-existing); crediting starts the tick after.
    baselined: bool,
    /// Scenes we've actually credited as remote this session. The tail flush
    /// only touches these, so it can never double-count a bridge-owned scene.
    remote: HashSet<String>,
    /// The tracked session's started_at (may be BACKDATED, e.g. a capture-prompt
    /// accept). Captured on the baseline tick.
    session_started: i64,
    /// Wall-clock ms when we baselined this session. The gap to `session_started`
    /// is what a first-credit backfill fills for scenes already playing then.
    baseline_at: i64,
    /// play_duration per scene AT the baseline tick — so backfill only fires for
    /// scenes that genuinely ADVANCED since baseline (not merely present with a
    /// nonzero lifetime value).
    baseline_pd: HashMap<String, f64>,
    /// Every scene the local bridge has owned at any point this session. The
    /// backdate backfill skips these so it can never double-credit a
    /// locally-watched or capture-seeded scene.
    ever_local: HashSet<String>,
}

impl PollerState {
    fn reset(&mut self, to: Option<i64>) {
        self.prev.clear();
        self.remote.clear();
        self.baselined = false;
        self.last_session = to;
        self.session_started = 0;
        self.baseline_at = 0;
        self.baseline_pd.clear();
        self.ever_local.clear();
    }
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;

        let mut st = PollerState::default();
        loop {
            let sleep_secs = match tick(&state, &mut st).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("remote-tracking tick failed: {:#}", e);
                    15
                }
            };
            tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        }
    });
}

/// One poll. Returns how long to sleep before the next one (the configured
/// cadence). Short-circuits whenever the feature is off or no session is active,
/// so a disabled/idle Climax never touches Stash.
async fn tick(state: &AppState, st: &mut PollerState) -> anyhow::Result<u64> {
    let cfg = settings::get_remote_tracking(&state.pool)
        .await
        .unwrap_or_default();
    // Keep well under record_heartbeat's 30s currentTime clamp; floor at 5s.
    let poll_secs = (cfg.poll_seconds as u64).clamp(5, 60);

    if !cfg.enabled {
        st.reset(None);
        return Ok(poll_secs);
    }

    // Active session (active only; not paused / ended). Mirrors presence.rs.
    let active: Option<(i64, i64)> = sqlx::query_as(
        "SELECT id, started_at FROM sessions WHERE status = 'active' ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    let active_id = active.map(|(id, _)| id);

    // Session ended / switched since last tick → flush its tail before resetting.
    match st.last_session {
        Some(old) if active_id != Some(old) => {
            if let Err(e) = flush_tail(state, old, st, poll_secs).await {
                tracing::warn!("remote-tracking: tail flush for session {} failed: {:#}", old, e);
            }
            st.reset(active_id);
        }
        None => st.last_session = active_id,
        _ => {} // same session as last tick — carry on
    }

    let (Some(sid), Some(start_ms)) = (active_id, active.map(|(_, s)| s)) else {
        return Ok(poll_secs); // no active session
    };

    // Lean fetch: every played scene's cumulative play_duration.
    let rows = match crate::stash::find_active_play_durations(&state.pool).await {
        Ok(r) => r,
        Err(e) => {
            // Stash unreachable / slow — keep the baseline and retry next tick.
            tracing::warn!("remote-tracking: stash poll failed: {:#}", e);
            return Ok(poll_secs);
        }
    };

    // Scenes the local bridge is actively reporting — the bridge credits these,
    // so exclude them to avoid double counting.
    let local = state.session.live_tracked_external_ids(LOCAL_FRESH_MS).await?;
    // Remember every scene the bridge has owned this session (even once) so the
    // backdate backfill never lands on a locally-watched / capture-seeded scene.
    for s in local.iter() {
        st.ever_local.insert(s.clone());
    }

    let now = crate::db::now_ms();
    // A single tick can't credit more than ~3 intervals (laptop-sleep guard).
    let max_tick = (poll_secs as i64) * 3;
    // A fresh scene can't have accrued more this-session watch than the session
    // has existed — cap the head-start credit at the session's age.
    let elapsed = ((now - start_ms) / 1000).max(0);

    // On the baseline tick, record the session start (possibly backdated) and
    // the baseline wall-clock, so first-credit backfill can fill the gap.
    if !st.baselined {
        st.session_started = start_ms;
        st.baseline_at = now;
        st.baseline_pd = rows.iter().map(|(k, v)| (k.clone(), *v)).collect();
    }

    let mut next: HashMap<String, f64> = HashMap::with_capacity(rows.len());
    for (scene_id, pd_now) in rows {
        // The baseline tick credits nothing; it just records starting values.
        // `known` = the scene already had watch time at the baseline (so it was
        // being watched at/before the session start) — only those get the
        // start-gap backfill; a genuinely-new mid-session scene does not.
        let (credit, known): (Option<i64>, bool) = if !st.baselined || local.contains(&scene_id) {
            // Baseline tick (only records starting values) or a locally-bridged
            // scene (credited via the bridge path) → credit nothing here.
            (None, false)
        } else {
            match st.prev.get(&scene_id) {
                // Known scene: credit the growth since last tick.
                Some(&pd_prev) => {
                    let delta = pd_now - pd_prev;
                    (
                        (delta > ADVANCE_EPSILON).then(|| (delta.round() as i64).min(max_tick)),
                        true,
                    )
                }
                // New scene since the baseline → it was un-watched at session
                // start, so its full current value is this session's watch.
                None => (
                    (pd_now > ADVANCE_EPSILON).then(|| (pd_now.round() as i64).min(elapsed)),
                    false,
                ),
            }
        };

        if let Some(secs) = credit {
            let mut total = secs;
            // First credit of a scene that was already playing at session start:
            // backfill the gap from the (possibly backdated) session start to the
            // baseline. This makes a backdated capture session credit a
            // concurrently-watched remote scene from the start, not from when the
            // poller first noticed it — and closes the ~one-interval baseline gap
            // on normal sessions too. Only for `known` scenes still detected
            // within ~90s of baselining (i.e. genuinely playing then).
            if known
                && !st.remote.contains(&scene_id)
                // Never backfill a scene the bridge owns/owned (avoids double-
                // counting the capture-seeded scene or a locally-watched one).
                && !st.ever_local.contains(&scene_id)
                // First credited within ~3 poll intervals of baselining → it was
                // watched continuously across the session-start boundary, not a
                // scene that only began afterwards.
                && now - st.baseline_at <= 45_000
                // And it actually ADVANCED vs the baseline snapshot (was being
                // watched, not just present with a nonzero lifetime value).
                && st
                    .baseline_pd
                    .get(&scene_id)
                    .is_some_and(|&bpd| pd_now - bpd > ADVANCE_EPSILON)
            {
                let backdate = ((st.baseline_at - st.session_started) / 1000).clamp(0, 1800);
                if backdate > 0 {
                    total += backdate;
                }
            }
            match state.session.credit_remote_play(sid, &scene_id, total).await {
                Ok(Some(content_id)) => {
                    st.remote.insert(scene_id.clone());
                    tracing::info!(
                        "remote play: scene {} watched on another device (+{}s) -> content {}",
                        scene_id,
                        total,
                        content_id
                    );
                }
                Ok(None) => {} // session vanished between the gate and here
                Err(e) => tracing::warn!(
                    "remote-tracking: crediting scene {} failed: {:#}",
                    scene_id,
                    e
                ),
            }
        }
        next.insert(scene_id, pd_now);
    }
    st.prev = next;
    st.baselined = true;
    Ok(poll_secs)
}

/// Final pass for a session that just ended: credit the watch between the last
/// poll and Stop. Only scenes we already credited this session are touched (so
/// a stale local-bridge entry can't be double-counted), and each tail credit is
/// capped at one poll interval — the most real time that can sit between the
/// last poll and Stop. play_duration has frozen at the stop value by now, so
/// `current - prev` is exactly that remainder.
async fn flush_tail(
    state: &AppState,
    session_id: i64,
    st: &PollerState,
    poll_secs: u64,
) -> anyhow::Result<()> {
    if st.remote.is_empty() {
        return Ok(());
    }
    let rows = crate::stash::find_active_play_durations(&state.pool).await?;
    let local = state.session.live_tracked_external_ids(LOCAL_FRESH_MS).await?;
    let cap = poll_secs as i64;
    for (scene_id, pd_now) in rows {
        if !st.remote.contains(&scene_id) || local.contains(&scene_id) {
            continue;
        }
        if let Some(&pd_prev) = st.prev.get(&scene_id) {
            let delta = pd_now - pd_prev;
            if delta > ADVANCE_EPSILON {
                let secs = (delta.round() as i64).min(cap);
                match state.session.credit_remote_play(session_id, &scene_id, secs).await {
                    Ok(Some(content_id)) => tracing::info!(
                        "remote tail: scene {} +{}s flushed to ended session {} -> content {}",
                        scene_id,
                        secs,
                        session_id,
                        content_id
                    ),
                    Ok(None) => {} // session discarded/deleted at stop
                    Err(e) => tracing::warn!(
                        "remote-tracking: tail credit scene {} failed: {:#}",
                        scene_id,
                        e
                    ),
                }
            }
        }
    }
    Ok(())
}
