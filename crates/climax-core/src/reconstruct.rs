// Retroactive session reconstruction (Phase 7, component 2).
//
// Engine: reads the session-less Stash-history imports (`play_imports` +
// session-less `o_events`), lays them on one timeline, and splits into CANDIDATE
// sessions wherever the gap between consecutive events exceeds the user's gap
// threshold. Pure-read; PROPOSES only.
//
// Write-path: `accept_candidate` turns a (possibly user-edited) candidate into a
// real session tagged `estimated=1` - it adopts the loose `o_events` (sets their
// session_id) and writes per-scene `scene_plays` with an estimated watch time.
// `dismiss_candidates` records the underlying import rows in
// `reconstruction_dismissed` so the "while you were away" prompt stops offering
// them. Neither touches Stash.
//
// Two exclusions keep things from being re-proposed:
//   - covered-by-session: an event inside a non-discarded session's window is
//     skipped. ALWAYS on -> an ACCEPTED candidate's new session covers its events,
//     so it never re-appears (even in the Settings full scan).
//   - dismissed: an event in `reconstruction_dismissed` is skipped. On for the
//     ongoing prompt (`respect_dismissed = true`); OFF for the Settings "import
//     older history" scan, so declined activity is still reachable later.
//
// Confidence tiers (see CLAUDE.md Phase 7 component 2):
//   - "boundable"      : >1 event in the cluster -> real start/end from the span.
//   - "lone_boundable" : 1 event, a play on a scene played exactly once -> its
//                        Stash play_duration bounds it ([t, t + dur]).
//   - "lone_fuzzy"     : 1 event, no usable duration -> a point, low confidence.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::{local_day_bounds_ms, local_day_of_ms, now_ms};

#[derive(Debug, Clone, Serialize)]
pub struct CandidateScene {
    pub content_item_id: i64,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    /// Play / cumshot timestamps (epoch ms) of this scene within the cluster.
    pub play_times: Vec<i64>,
    pub cumshot_times: Vec<i64>,
    /// Underlying import row ids, so accept/dismiss can act on exact rows.
    pub play_import_ids: Vec<i64>,
    pub o_event_ids: Vec<i64>,
    /// Estimated seconds watched in THIS session for this scene (the scene's
    /// lifetime play_duration apportioned across its plays). Approximate.
    pub est_seconds: i64,
    /// True when this scene was watched below the live play threshold (measured
    /// watch < scene_length × threshold%) and carries no cumshot. The Untracked
    /// review collapses these behind a toggle and excludes them from a default
    /// accept, the same as the wrap-up modal. A scene with a cumshot is never
    /// flagged (you came to it), and the sole scene of a cluster is never flagged
    /// (that would empty the candidate). frac == 0 (no threshold set) never flags.
    pub below_threshold: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateSession {
    /// First event of the cluster (epoch ms). For lone_boundable, the play time.
    pub start_ms: i64,
    /// Last event (epoch ms). For lone_boundable, start + the scene's play_duration.
    pub end_ms: i64,
    /// "boundable" | "lone_boundable" | "lone_fuzzy".
    pub tier: String,
    pub event_count: i64,
    pub play_count: i64,
    pub cumshot_count: i64,
    /// Local day (YYYY-MM-DD) the session would be attributed to (start's day).
    pub assigned_day: String,
    /// Distinct scenes in the cluster, ordered by first activity.
    pub scenes: Vec<CandidateScene>,
}

/// Result of the "while you were away" launch scan.
#[derive(Debug, Clone, Serialize)]
pub struct AwayResult {
    /// Recency floor = the most recent non-discarded session's end (epoch ms), so
    /// the UI can caption "since <date>". `None` = no prior sessions (a fresh DB);
    /// the caller then shows nothing (the onboarding "estimate history" step owns
    /// the cold backlog).
    pub since_ms: Option<i64>,
    pub candidates: Vec<CandidateSession>,
}

struct SceneInfo {
    external_id: Option<String>,
    title: Option<String>,
    thumbnail_url: Option<String>,
    play_duration_seconds: i64,
    /// The scene's real length (content_items.duration_seconds), used for the
    /// below-threshold check (watched / length vs the play-threshold %). Distinct
    /// from play_duration_seconds, which is Stash's CUMULATIVE watch across plays.
    length_seconds: i64,
}

#[derive(Default)]
struct SceneAccum {
    play_times: Vec<i64>,
    cumshot_times: Vec<i64>,
    play_import_ids: Vec<i64>,
    o_event_ids: Vec<i64>,
}

/// One event on the timeline. `is_play` distinguishes a play_import from an
/// o_event; `row_id` is that source row's id.
struct Ev {
    ts: i64,
    cid: i64,
    is_play: bool,
    row_id: i64,
}

/// Cluster the uncovered session-less imports in `[start_day, end_day]` into
/// candidate sessions at `gap_minutes`. `respect_dismissed` excludes rows the
/// user previously dismissed (true for the prompt; false for the Settings full
/// scan). Newest-first.
pub async fn candidate_sessions(
    pool: &SqlitePool,
    gap_minutes: u32,
    start_day: &str,
    end_day: &str,
    respect_dismissed: bool,
) -> Result<Vec<CandidateSession>> {
    let (lo, hi) = local_day_bounds_ms(start_day, end_day)?;
    let now = now_ms();
    // Live play-count threshold (fraction of a scene's length). Used to collapse
    // barely-watched scenes out of the default accept. 0.0 = no threshold set
    // (Stash's default) -> nothing collapses. Same source as live tracking.
    let frac = crate::settings::effective_play_threshold_frac(
        &crate::settings::get_play_counting(pool).await.unwrap_or_default(),
    );

    let o_dismiss = if respect_dismissed {
        "AND NOT EXISTS (SELECT 1 FROM reconstruction_dismissed d WHERE d.kind='o' AND d.source_id = oe.id)"
    } else {
        ""
    };
    let p_dismiss = if respect_dismissed {
        "AND NOT EXISTS (SELECT 1 FROM reconstruction_dismissed d WHERE d.kind='play' AND d.source_id = pi.id)"
    } else {
        ""
    };

    // Uncovered session-less O imports.
    let o_sql = format!(
        "SELECT oe.id, oe.occurred_at, oe.content_item_id FROM o_events oe
         WHERE oe.session_id IS NULL AND oe.content_item_id IS NOT NULL
           AND oe.occurred_at >= ?1 AND oe.occurred_at < ?2
           AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.status != 'discarded'
                           AND s.started_at <= oe.occurred_at
                           AND COALESCE(s.ended_at, ?3) >= oe.occurred_at)
           {o_dismiss}"
    );
    let o_rows: Vec<(i64, i64, i64)> = sqlx::query_as(&o_sql)
        .bind(lo)
        .bind(hi)
        .bind(now)
        .fetch_all(pool)
        .await?;

    // Uncovered play imports.
    let p_sql = format!(
        "SELECT pi.id, pi.played_at, pi.content_item_id FROM play_imports pi
         WHERE pi.played_at >= ?1 AND pi.played_at < ?2
           AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.status != 'discarded'
                           AND s.started_at <= pi.played_at
                           AND COALESCE(s.ended_at, ?3) >= pi.played_at)
           {p_dismiss}"
    );
    let p_rows: Vec<(i64, i64, i64)> = sqlx::query_as(&p_sql)
        .bind(lo)
        .bind(hi)
        .bind(now)
        .fetch_all(pool)
        .await?;

    let mut events: Vec<Ev> = Vec::with_capacity(o_rows.len() + p_rows.len());
    for (id, ts, cid) in o_rows {
        events.push(Ev { ts, cid, is_play: false, row_id: id });
    }
    for (id, ts, cid) in p_rows {
        events.push(Ev { ts, cid, is_play: true, row_id: id });
    }
    if events.is_empty() {
        return Ok(vec![]);
    }
    events.sort_by_key(|e| e.ts);

    // Per-scene lifetime play count (ALL play_imports) for lone tier + estimate.
    let play_totals: HashMap<i64, i64> = sqlx::query_as::<_, (i64, i64)>(
        "SELECT content_item_id, COUNT(*) FROM play_imports GROUP BY content_item_id",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    // Display info + play_duration for the involved scenes (dynamic IN).
    let mut cids: Vec<i64> = events.iter().map(|e| e.cid).collect();
    cids.sort_unstable();
    cids.dedup();
    let placeholders = cids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let info_sql = format!(
        "SELECT id, external_id, title, thumbnail_url,
                COALESCE(CAST(json_extract(metadata_json, '$.play_duration_seconds') AS INTEGER), 0) AS dur,
                COALESCE(duration_seconds, 0) AS len
         FROM content_items WHERE id IN ({placeholders})"
    );
    let mut iq = sqlx::query_as::<_, (i64, Option<String>, Option<String>, Option<String>, i64, i64)>(
        &info_sql,
    );
    for c in &cids {
        iq = iq.bind(c);
    }
    let info: HashMap<i64, SceneInfo> = iq
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, external_id, title, thumbnail_url, dur, len)| {
            (
                id,
                SceneInfo {
                    external_id,
                    title,
                    thumbnail_url,
                    play_duration_seconds: dur,
                    length_seconds: len,
                },
            )
        })
        .collect();

    // Split into clusters wherever the inter-event gap exceeds the threshold.
    let gap_ms = (gap_minutes as i64) * 60 * 1000;
    let mut clusters: Vec<Vec<Ev>> = Vec::new();
    let mut cur: Vec<Ev> = Vec::new();
    for e in events {
        if let Some(last) = cur.last() {
            if e.ts - last.ts > gap_ms {
                clusters.push(std::mem::take(&mut cur));
            }
        }
        cur.push(e);
    }
    if !cur.is_empty() {
        clusters.push(cur);
    }

    let mut out: Vec<CandidateSession> = Vec::with_capacity(clusters.len());
    for cl in clusters {
        let event_count = cl.len() as i64;
        let play_count = cl.iter().filter(|e| e.is_play).count() as i64;
        let cumshot_count = event_count - play_count;
        let first_ts = cl.first().unwrap().ts;
        let last_ts = cl.last().unwrap().ts;

        let (tier, start_ms, mut end_ms) = if event_count > 1 {
            ("boundable", first_ts, last_ts)
        } else {
            let e = &cl[0];
            let lifetime_plays = play_totals.get(&e.cid).copied().unwrap_or(0);
            if e.is_play && lifetime_plays == 1 {
                let dur = info.get(&e.cid).map(|i| i.play_duration_seconds).unwrap_or(0);
                ("lone_boundable", e.ts, e.ts + dur * 1000)
            } else {
                ("lone_fuzzy", e.ts, e.ts)
            }
        };

        // Group by scene.
        let mut by_scene: HashMap<i64, SceneAccum> = HashMap::new();
        for e in &cl {
            let a = by_scene.entry(e.cid).or_default();
            if e.is_play {
                a.play_times.push(e.ts);
                a.play_import_ids.push(e.row_id);
            } else {
                a.cumshot_times.push(e.ts);
                a.o_event_ids.push(e.row_id);
            }
        }
        let mut scenes: Vec<CandidateScene> = by_scene
            .into_iter()
            .map(|(cid, mut a)| {
                a.play_times.sort_unstable();
                a.cumshot_times.sort_unstable();
                let i = info.get(&cid);
                CandidateScene {
                    content_item_id: cid,
                    external_id: i.and_then(|x| x.external_id.clone()),
                    title: i.and_then(|x| x.title.clone()),
                    thumbnail_url: i.and_then(|x| x.thumbnail_url.clone()),
                    play_times: a.play_times,
                    cumshot_times: a.cumshot_times,
                    play_import_ids: a.play_import_ids,
                    o_event_ids: a.o_event_ids,
                    est_seconds: 0,        // filled below from inter-scene windows
                    below_threshold: false, // computed below, after est_seconds
                }
            })
            .collect();
        scenes.sort_by_key(|s| {
            s.play_times
                .first()
                .copied()
                .or_else(|| s.cumshot_times.first().copied())
                .unwrap_or(i64::MAX)
        });

        // Watch-time estimate (per scene), BOUNDED so Climax never logs more than
        // Stash knows for a scene. Only PLAY-backed occurrences draw watch time;
        // each is bounded by two signals: (a) the GAP to the next scene opening (you
        // plausibly watched a scene until you opened the next one), and (b) each
        // play's FAIR SHARE of Stash's lifetime watch time = play_duration / (number
        // of plays). We take the SMALLER: never more than the gap, never more than
        // Stash's share. The share splits a scene's Stash total across ALL its plays
        // (covered + uncovered), and there's at most one play-backed occurrence per
        // play import, so a scene appearing in several sittings can't have the first
        // claim the whole budget - each gets its slice and the sum across all
        // reconstructed sessions can't exceed play_duration. A CUMSHOT-ONLY
        // occurrence (a cumshot but no play import in this cluster) draws ZERO watch
        // time: that budget belongs to plays, and crediting a cumshot-only occurrence
        // its share would let a favourite you cum to on several un-registered nights
        // log several times Stash's total. The LAST play-backed scene has no
        // next-scene gap, so it uses Stash's share directly = real evidence, not a
        // synthetic floor. When Stash has no duration for a scene we fall back to the
        // old gap / floored-window estimate. The session end is then stretched to
        // cover the last scene's estimate so its band renders.
        const LAST_SCENE_FLOOR_MS: i64 = 120_000;
        let anchor = |s: &CandidateScene| -> i64 {
            s.play_times
                .first()
                .copied()
                .into_iter()
                .chain(s.cumshot_times.first().copied())
                .min()
                .unwrap_or(start_ms)
        };
        let last_event = |s: &CandidateScene| -> i64 {
            s.play_times
                .last()
                .copied()
                .into_iter()
                .chain(s.cumshot_times.last().copied())
                .max()
                .unwrap_or(start_ms)
        };
        let anchors: Vec<i64> = scenes.iter().map(&anchor).collect();
        let last_events: Vec<i64> = scenes.iter().map(&last_event).collect();
        let n = scenes.len();
        // Stash's fair share of watch time for ONE play of a scene, in ms (lifetime
        // play_duration split evenly across its plays). None when Stash has no
        // duration - then we can't bound against Stash and fall back to the gap.
        let per_play_ms = |cid: i64| -> Option<i64> {
            let dur = info.get(&cid).map(|i| i.play_duration_seconds).unwrap_or(0);
            if dur <= 0 {
                return None;
            }
            let plays = play_totals.get(&cid).copied().unwrap_or(0).max(1);
            Some(dur * 1000 / plays)
        };
        for idx in 0..n {
            // Cumshot-only occurrence (no play import in this cluster): no watch
            // record + it must not draw from Stash's play_duration budget, so 0.
            // The cumshot itself is the evidence and is preserved separately.
            let est_ms = if scenes[idx].play_times.is_empty() {
                0
            } else {
                let share = per_play_ms(scenes[idx].content_item_id);
                if idx + 1 < n {
                    // Middle scene: watched until the next opened, but never more
                    // than Stash's per-play share.
                    let gap = (anchors[idx + 1] - anchors[idx]).max(0);
                    match share {
                        Some(s) => gap.min(s),
                        None => gap,
                    }
                } else {
                    // Last (or only) scene: no next-scene gap. Use Stash's share as
                    // the watch time (real evidence); else fall back to the floored
                    // window.
                    match share {
                        Some(s) => s.max(0),
                        None => (end_ms - anchors[idx])
                            .max(last_events[idx] - anchors[idx])
                            .max(LAST_SCENE_FLOOR_MS),
                    }
                }
            };
            scenes[idx].est_seconds = est_ms / 1000;
        }
        if n > 0 {
            end_ms = end_ms.max(anchors[n - 1] + scenes[n - 1].est_seconds * 1000);
        }

        // Below-threshold flag: a scene whose ESTIMATED watch time (the Stash-bounded
        // value above = exactly what we'd log) is under scene_length x the play
        // threshold, and which carries no cumshot. Collapsed behind a toggle and left
        // out of a default accept (mirrors the wrap-up modal). Judged on est_seconds
        // so the flag matches the watch time shown. The last play-backed scene uses
        // Stash's own per-play share; when Climax's threshold matches Stash's
        // minimum-play-percent that share is at/above threshold (a logged play
        // already crossed it), so the scene you stopped on is usually kept by DATA,
        // not a floor - though a raised Climax threshold, multi-play averaging, or a
        // boundary case can still flag it, which is fine (the user can re-include it).
        // Cumshot scenes are exempt (you came to it). The sole scene of a cluster is
        // never flagged (that would leave the candidate empty; the user Skips the
        // whole candidate). frac == 0 -> no threshold -> flag nothing. Unknown scene
        // length -> can't judge -> keep.
        if n > 1 && frac > 0.0 {
            for scene in scenes.iter_mut() {
                if !scene.cumshot_times.is_empty() {
                    continue; // came to it -> counts regardless of watch time
                }
                let len = info
                    .get(&scene.content_item_id)
                    .map(|i| i.length_seconds)
                    .unwrap_or(0);
                let counts = len <= 0 || (scene.est_seconds as f32) >= (len as f32) * frac;
                scene.below_threshold = !counts;
            }
        }

        out.push(CandidateSession {
            start_ms,
            end_ms,
            tier: tier.to_string(),
            event_count,
            play_count,
            cumshot_count,
            assigned_day: local_day_of_ms(start_ms).unwrap_or_default(),
            scenes,
        });
    }

    out.sort_by_key(|c| std::cmp::Reverse(c.start_ms));
    Ok(out)
}

/// The "while you were away" launch scan: reconstruction candidates whose activity
/// falls AFTER the user's most recent logged session (the recency floor), honoring
/// dismissed memory. A lighter, proactive cousin of the Settings full scan - it
/// nudges "since you last tracked, here's what you watched untracked." The floor
/// is self-advancing: an accepted candidate becomes an `estimated` session whose
/// end is the new floor (and which covers its own events), so it never re-appears;
/// "Not now" leaves everything and it shows again next launch; "Skip" records the
/// rows in `reconstruction_dismissed` so they're gone. Older-than-floor activity
/// (e.g. a late mirror sync) is intentionally NOT shown here - it stays reachable
/// in Settings -> Untracked (which scans all of time, no floor).
pub async fn away_candidates(pool: &SqlitePool, gap_minutes: u32) -> Result<AwayResult> {
    // Floor = most recent non-discarded session's end. Estimated/reconstructed
    // sessions count (they're status 'ended' with a non-null ended_at).
    let floor: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(ended_at) FROM sessions WHERE status != 'discarded' AND ended_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    let Some(floor_ms) = floor else {
        return Ok(AwayResult { since_ms: None, candidates: vec![] });
    };
    let start_day =
        local_day_of_ms(floor_ms).ok_or_else(|| anyhow::anyhow!("bad floor timestamp"))?;
    let end_day =
        local_day_of_ms(now_ms()).ok_or_else(|| anyhow::anyhow!("bad now timestamp"))?;
    let cands = candidate_sessions(pool, gap_minutes, &start_day, &end_day, true).await?;
    // The scan is day-granular, so it can include a cluster earlier on the floor's
    // own day (before the session ended). Keep only clusters with REAL activity
    // after the floor, so the prompt is strictly "since your last session". Filter
    // on the latest real event, NOT c.end_ms: for a lone_boundable, end_ms is
    // synthetic (play time + the scene's lifetime play_duration) and can land hours
    // after the actual play, which would leak a pre-floor play into the prompt.
    let candidates: Vec<CandidateSession> = cands
        .into_iter()
        .filter(|c| {
            let last_activity = c
                .scenes
                .iter()
                .flat_map(|s| s.play_times.iter().chain(s.cumshot_times.iter()))
                .copied()
                .max()
                .unwrap_or(c.start_ms);
            last_activity > floor_ms
        })
        .collect();
    Ok(AwayResult { since_ms: Some(floor_ms), candidates })
}

// ---------- write-path ----------

/// Bulk-accept EVERY reconstruction candidate across all history at
/// `gap_minutes` — the first-launch "estimate my past sessions" path. No
/// line-by-line review and no dismissed-filtering (a fresh import accepts the
/// lot). Each candidate becomes an `estimated` session via `accept_candidate`.
/// No Stash writes: `extra_cumshots` is always 0 here, so every cumshot is an
/// already-imported Stash o_event that's simply adopted (the returned
/// PendingPush lists are empty and ignored). Returns the number of sessions
/// created.
pub async fn accept_all_candidates(pool: &SqlitePool, gap_minutes: u32) -> Result<usize> {
    // Whole history; respect_dismissed=false so onboarding accepts everything.
    let candidates =
        candidate_sessions(pool, gap_minutes, "2000-01-01", "2099-12-31", false).await?;
    let mut created = 0usize;
    for c in candidates {
        let o_event_ids: Vec<i64> = c
            .scenes
            .iter()
            .flat_map(|s| s.o_event_ids.iter().copied())
            .collect();
        // Skip below-threshold scenes here too: onboarding's bulk "estimate my
        // history" is a default accept, so it obeys the same "don't bring in
        // barely-watched scenes" rule as the interactive Untracked page. Cumshot
        // scenes are never flagged, so they still come in.
        let scenes: Vec<AcceptScene> = c
            .scenes
            .iter()
            .filter(|s| !s.below_threshold)
            .map(|s| {
                let first = s
                    .play_times
                    .iter()
                    .chain(s.cumshot_times.iter())
                    .copied()
                    .min()
                    .unwrap_or(0);
                let last = s
                    .play_times
                    .iter()
                    .chain(s.cumshot_times.iter())
                    .copied()
                    .max()
                    .unwrap_or(0);
                AcceptScene {
                    content_item_id: s.content_item_id,
                    est_seconds: s.est_seconds,
                    first_seen_ms: first,
                    last_seen_ms: last,
                    extra_cumshots: 0,
                }
            })
            .collect();
        // Nothing above threshold and no cumshots to adopt -> don't create an
        // empty estimated session.
        if scenes.is_empty() && o_event_ids.is_empty() {
            continue;
        }
        accept_candidate(pool, c.start_ms, c.end_ms, &c.assigned_day, &o_event_ids, &scenes).await?;
        created += 1;
    }
    Ok(created)
}

/// One scene to record on an accepted candidate's session.
#[derive(Debug, Clone, Deserialize)]
pub struct AcceptScene {
    pub content_item_id: i64,
    pub est_seconds: i64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    /// Extra cumshots to LOG on this scene that Stash doesn't have yet. They're
    /// created as origin='climax' o_events and pushed to Stash by the caller.
    /// Detected Stash cumshots arrive via `o_event_ids` and are NOT re-pushed.
    #[serde(default)]
    pub extra_cumshots: i64,
}

/// A reconstruction cumshot that was just created (origin='climax') and must be
/// pushed to Stash (`sceneAddO`) by the caller, then flipped to stash_synced=1.
/// Mirrors the live `log_o` push contract so a later mirror run dedups it.
pub struct PendingPush {
    pub o_event_id: i64,
    /// Stash scene external id.
    pub scene_id: String,
    pub at_ms: i64,
}

/// Turn a (possibly user-edited) candidate into a real session tagged
/// `estimated=1`. Adopts the loose `o_events` (sets session_id; they stay
/// stash_synced) and writes per-scene `scene_plays` with the estimated watch
/// time. One transaction. Returns the new session id plus any newly-created
/// cumshots that still need pushing to Stash (the engine itself never touches
/// Stash - the caller does that in the background).
pub async fn accept_candidate(
    pool: &SqlitePool,
    start_ms: i64,
    end_ms: i64,
    assigned_day: &str,
    o_event_ids: &[i64],
    scenes: &[AcceptScene],
) -> Result<(i64, Vec<PendingPush>)> {
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let sid: i64 = sqlx::query_scalar(
        "INSERT INTO sessions
            (started_at, ended_at, last_heartbeat, status, notes, excluded,
             session_type, estimated, assigned_day, created_at, updated_at)
         VALUES (?1, ?2, NULL, 'ended', NULL, 0, 'reconstructed', 1, ?3, ?4, ?4)
         RETURNING id",
    )
    .bind(start_ms)
    .bind(end_ms)
    .bind(assigned_day)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // Adopt loose cumshots (guard on still-session-less so we never steal a
    // witnessed/already-owned event).
    for oid in o_event_ids {
        sqlx::query("UPDATE o_events SET session_id = ?1 WHERE id = ?2 AND session_id IS NULL")
            .bind(sid)
            .bind(oid)
            .execute(&mut *tx)
            .await?;
    }

    // Per-scene watch rows (estimated). first/last default to the session bounds
    // when a scene carried only a cumshot (no play time of its own). counted_at
    // is stamped (= the play time): these plays registered in Stash, so they
    // count by definition - without it the SessionDetail scene list (which shows
    // only counted plays or scenes with a cumshot) would hide them.
    for sc in scenes {
        // Clamp into the (possibly user-edited) session window so an estimated
        // play row never lands outside [start, end] and trips the bounds guard
        // on a later session edit.
        let first = (if sc.first_seen_ms > 0 { sc.first_seen_ms } else { start_ms })
            .max(start_ms)
            .min(end_ms);
        // Band span = the estimated watch time, anchored at first_seen, so the
        // SessionDetail watch-band renders the estimate (and matches the "Nm
        // watched" label) instead of collapsing to ~0 for a single-timestamp
        // scene. Clamped to the (already end-stretched) session window.
        let last = (first + sc.est_seconds * 1000).max(start_ms).min(end_ms);
        sqlx::query(
            "INSERT INTO scene_plays
                (session_id, content_item_id, first_seen_at, last_seen_at, seconds_tracked, counted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?3)
             ON CONFLICT(session_id, content_item_id)
             DO UPDATE SET seconds_tracked = excluded.seconds_tracked,
                           counted_at = excluded.counted_at",
        )
        .bind(sid)
        .bind(sc.content_item_id)
        .bind(first)
        .bind(last)
        .bind(sc.est_seconds)
        .execute(&mut *tx)
        .await?;
    }

    // Manually-added cumshots (Stash doesn't know about these yet): create them
    // as origin='climax' o_events (stash_synced=0) inside the session. Collect the
    // ones on stash-sourced scenes so the caller can push them + flip synced.
    let mut pushes: Vec<PendingPush> = Vec::new();
    for sc in scenes {
        if sc.extra_cumshots <= 0 {
            continue;
        }
        // Clamp to the session window so every added cumshot stays in-bounds
        // (this raw INSERT bypasses log_o's bounds enforcement).
        let last = (if sc.last_seen_ms > 0 { sc.last_seen_ms } else { end_ms })
            .max(start_ms)
            .min(end_ms);
        // Only push to Stash for stash-sourced scenes with a real external id.
        let src: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT s.key, c.external_id
             FROM content_items c JOIN sources s ON s.id = c.source_id
             WHERE c.id = ?1",
        )
        .bind(sc.content_item_id)
        .fetch_optional(&mut *tx)
        .await?;
        for i in 0..sc.extra_cumshots {
            // Space added cumshots 2s apart (> the mirror's +/-1.5s dedup window)
            // so each is a distinct o_history entry; anchor at the scene's last
            // activity, walking back, floored at the session start.
            let at = (last - i * 2000).max(start_ms);
            let oid: i64 = sqlx::query_scalar(
                "INSERT INTO o_events
                    (session_id, content_item_id, occurred_at, intensity, notes, stash_synced, origin, created_at)
                 VALUES (?1, ?2, ?3, NULL, NULL, 0, 'climax', ?4)
                 RETURNING id",
            )
            .bind(sid)
            .bind(sc.content_item_id)
            .bind(at)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
            if let Some((ref key, Some(ref ext))) = src {
                if key == "stash" {
                    pushes.push(PendingPush {
                        o_event_id: oid,
                        scene_id: ext.clone(),
                        at_ms: at,
                    });
                }
            }
        }
    }

    tx.commit().await?;
    Ok((sid, pushes))
}

/// Record dismissed candidates' underlying import rows so the prompt won't
/// re-offer them. Idempotent (PK on (kind, source_id)). Returns rows recorded.
pub async fn dismiss_candidates(
    pool: &SqlitePool,
    play_import_ids: &[i64],
    o_event_ids: &[i64],
) -> Result<usize> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let mut n = 0usize;
    for id in play_import_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO reconstruction_dismissed (kind, source_id, dismissed_at)
             VALUES ('play', ?1, ?2)",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        n += 1;
    }
    for id in o_event_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO reconstruction_dismissed (kind, source_id, dismissed_at)
             VALUES ('o', ?1, ?2)",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        n += 1;
    }
    tx.commit().await?;
    Ok(n)
}

// ---------- session history review (fold Stash history into an EXISTING session) ----------
//
// Distinct from reconstruction (which CREATES sessions from session-less gaps):
// this surfaces Stash play_history / o_history that falls inside an already-real
// session's window but isn't reflected in it (e.g. watched on another device, so
// the bridge on this machine never saw real watch time). Review-first: the UI
// offers each item to Add or Skip. Pure-local on absorb - the events already
// exist in Stash, so nothing is pushed back.

/// A scene Stash logged play(s) for inside a session's window that the session
/// doesn't already track (no counted scene_play). Surfaced for review.
#[derive(Debug, Clone, Serialize)]
pub struct PendingHistoryScene {
    pub content_item_id: i64,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    /// Stash play timestamps (epoch ms) inside the session window (empty for a
    /// bridge-only candidate - seen here but never logged in Stash).
    pub play_times: Vec<i64>,
    /// Best-effort estimated seconds (max of apportioned play_duration + the
    /// bridge's tab-open span).
    pub est_seconds: i64,
}

/// A session-less Stash cumshot inside a session's window, surfaced for review.
#[derive(Debug, Clone, Serialize)]
pub struct PendingHistoryO {
    pub o_event_id: i64,
    pub content_item_id: Option<i64>,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionPendingHistory {
    pub scenes: Vec<PendingHistoryScene>,
    pub cumshots: Vec<PendingHistoryO>,
}

/// Stash play/o history inside a COMPLETED session's window that isn't reflected
/// in the session yet (and the user hasn't skipped). Empty for active / paused /
/// discarded sessions - only completed ones get the review. Read-only.
pub async fn session_pending_history(
    pool: &SqlitePool,
    session_id: i64,
    include_active: bool,
    // When true, ALSO surface items the user previously skipped (so "Skip" is
    // reversible - the review can show skipped items to re-add). Default false.
    include_dismissed: bool,
) -> Result<SessionPendingHistory> {
    let empty = SessionPendingHistory { scenes: vec![], cumshots: vec![] };
    let row: Option<(i64, Option<i64>, String)> =
        sqlx::query_as("SELECT started_at, ended_at, status FROM sessions WHERE id = ?1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    let Some((start, ended, status)) = row else { return Ok(empty); };
    if status == "discarded" {
        return Ok(empty);
    }
    // Completed sessions use their real end. An in-progress session being wrapped
    // up (include_active) uses now as the window end so the wrap-up review can
    // surface activity from the just-finished sitting.
    let end = match ended {
        Some(e) => e,
        None if include_active => now_ms(),
        None => return Ok(empty),
    };

    // Candidate scenes: a scene the session DOESN'T already count (no counted
    // scene_play) and the user hasn't skipped, that EITHER has a Stash play in the
    // window OR actually PLAYED locally but stayed below the count threshold
    // (seconds_tracked > 0). We require real playback evidence - we do NOT surface
    // a scene merely because its tab was open (the old `last_seen - first_seen >=
    // 60s` gate caught idle paused tabs, which then got logged with bogus
    // session-length watch time). A scene watched purely on another device only
    // surfaces if Stash recorded a play for it.
    let cand_cids: Vec<i64> = sqlx::query_scalar(
        "SELECT u.cid FROM (
            SELECT content_item_id AS cid FROM play_imports
             WHERE played_at >= ?1 AND played_at <= ?2
            UNION
            SELECT content_item_id AS cid FROM scene_plays
             WHERE session_id = ?3 AND counted_at IS NULL
               AND seconds_tracked > 0
         ) u
         WHERE NOT EXISTS (SELECT 1 FROM scene_plays sp
                           WHERE sp.session_id = ?3 AND sp.content_item_id = u.cid
                             AND sp.counted_at IS NOT NULL)
           AND (?4 = 1 OR NOT EXISTS (SELECT 1 FROM session_history_dismissed d
                           WHERE d.session_id = ?3 AND d.kind = 'play' AND d.ref_id = u.cid))",
    )
    .bind(start)
    .bind(end)
    .bind(session_id)
    .bind(if include_dismissed { 1_i64 } else { 0_i64 })
    .fetch_all(pool)
    .await?;

    let mut scenes: Vec<PendingHistoryScene> = Vec::new();
    if !cand_cids.is_empty() {
        let play_totals: HashMap<i64, i64> = sqlx::query_as::<_, (i64, i64)>(
            "SELECT content_item_id, COUNT(*) FROM play_imports GROUP BY content_item_id",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();

        let mut cids = cand_cids.clone();
        cids.sort_unstable();
        let placeholders = cids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let info_sql = format!(
            "SELECT id, external_id, title, thumbnail_url,
                    COALESCE(CAST(json_extract(metadata_json, '$.play_duration_seconds') AS INTEGER), 0) AS dur
             FROM content_items WHERE id IN ({placeholders})"
        );
        let mut iq =
            sqlx::query_as::<_, (i64, Option<String>, Option<String>, Option<String>, i64)>(&info_sql);
        for c in &cids {
            iq = iq.bind(c);
        }
        type InfoRow = (Option<String>, Option<String>, Option<String>, i64);
        let info: HashMap<i64, InfoRow> = iq
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(id, ext, title, thumb, dur)| (id, (ext, title, thumb, dur)))
            .collect();

        for cid in cids {
            // Stash play timestamps for this scene inside the window (empty for a
            // bridge-only candidate, i.e. seen here but never logged in Stash).
            let mut times: Vec<i64> = sqlx::query_scalar(
                "SELECT played_at FROM play_imports
                 WHERE content_item_id = ?1 AND played_at >= ?2 AND played_at <= ?3
                 ORDER BY played_at",
            )
            .bind(cid)
            .bind(start)
            .bind(end)
            .fetch_all(pool)
            .await?;
            times.sort_unstable();
            let (ext, title, thumb, dur) =
                info.get(&cid).cloned().unwrap_or((None, None, None, 0));
            let lifetime = play_totals.get(&cid).copied().unwrap_or(0).max(1);
            let apportioned =
                ((dur as f64) * (times.len() as f64) / (lifetime as f64)).round() as i64;
            // Watch estimate = the larger of Stash's recorded play_duration
            // (apportioned) and the REAL local playback (seconds_tracked - video
            // actually advanced here, below the count threshold). Both are genuine
            // watch time. We never use the tab-open span: last_seen_at advances on
            // every heartbeat incl. paused ones, so an idle open tab's span = the
            // whole session, which isn't watch time. No record at all -> 0.
            let local_secs: i64 = sqlx::query_scalar(
                "SELECT COALESCE(seconds_tracked, 0) FROM scene_plays
                 WHERE session_id = ?1 AND content_item_id = ?2",
            )
            .bind(session_id)
            .bind(cid)
            .fetch_optional(pool)
            .await?
            .unwrap_or(0);
            scenes.push(PendingHistoryScene {
                content_item_id: cid,
                external_id: ext,
                title,
                thumbnail_url: thumb,
                play_times: times,
                est_seconds: apportioned.max(local_secs),
            });
        }
        // Stash-timed candidates first (by play time), bridge-only ones after.
        scenes.sort_by_key(|s| s.play_times.first().copied().unwrap_or(i64::MAX));
    }

    // Candidate cumshots: session-less Stash o_events in the window, not skipped.
    let cumshots: Vec<PendingHistoryO> = sqlx::query_as::<_, (i64, Option<i64>, i64)>(
        "SELECT oe.id, oe.content_item_id, oe.occurred_at FROM o_events oe
         WHERE oe.session_id IS NULL AND oe.occurred_at >= ?1 AND oe.occurred_at <= ?2
           AND (?4 = 1 OR NOT EXISTS (SELECT 1 FROM session_history_dismissed d
                           WHERE d.session_id = ?3 AND d.kind = 'o' AND d.ref_id = oe.id))
         ORDER BY oe.occurred_at",
    )
    .bind(start)
    .bind(end)
    .bind(session_id)
    .bind(if include_dismissed { 1_i64 } else { 0_i64 })
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, cid, at)| PendingHistoryO { o_event_id: id, content_item_id: cid, occurred_at: at })
    .collect();

    Ok(SessionPendingHistory { scenes, cumshots })
}

/// Fold the selected Stash history into the session: count/create scene_plays for
/// the chosen scenes (estimated watch time) and adopt the chosen session-less
/// cumshots. Pure-local (events already exist in Stash - nothing pushed back).
/// Idempotent. Recomputes everything server-side (ignores any client estimate).
pub async fn absorb_session_history(
    pool: &SqlitePool,
    session_id: i64,
    content_item_ids: &[i64],
    o_event_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    absorb_into_conn(&mut tx, session_id, content_item_ids, o_event_ids).await?;
    tx.commit().await?;
    Ok(())
}
// NOTE: `&mut tx` (Transaction) coerces to `&mut SqliteConnection` via DerefMut.

/// Inner body of `absorb_session_history`, on a caller-supplied connection so it
/// can be composed into a LARGER transaction (extend_session extends the
/// session AND folds the activity in atomically). Does NOT begin/commit.
async fn absorb_into_conn(
    tx: &mut sqlx::SqliteConnection,
    session_id: i64,
    content_item_ids: &[i64],
    o_event_ids: &[i64],
) -> Result<()> {
    let row: Option<(i64, Option<i64>)> =
        sqlx::query_as("SELECT started_at, ended_at FROM sessions WHERE id = ?1")
            .bind(session_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((start, ended)) = row else {
        tracing::warn!("absorb_into_conn: session {} not found", session_id);
        return Err(anyhow::anyhow!("That session no longer exists."));
    };
    // .max(start) keeps clamp bounds ordered even if a session somehow has
    // ended_at < started_at (no current path allows it, but avoid a panic).
    let end = ended.unwrap_or_else(now_ms).max(start);

    for &cid in content_item_ids {
        let times: Vec<i64> = sqlx::query_scalar(
            "SELECT played_at FROM play_imports
             WHERE content_item_id = ?1 AND played_at >= ?2 AND played_at <= ?3
             ORDER BY played_at",
        )
        .bind(cid)
        .bind(start)
        .bind(end)
        .fetch_all(&mut *tx)
        .await?;
        // The existing local scene_play, if any (first/last + real tracked seconds).
        let existing: Option<(i64, i64, i64)> = sqlx::query_as(
            "SELECT first_seen_at, last_seen_at, seconds_tracked FROM scene_plays
             WHERE session_id = ?1 AND content_item_id = ?2",
        )
        .bind(session_id)
        .bind(cid)
        .fetch_optional(&mut *tx)
        .await?;
        // Nothing to fold: no Stash play in window AND the bridge never saw it.
        if times.is_empty() && existing.is_none() {
            continue;
        }
        // first/last for a NEW row (Stash-only). Ignored on conflict, where the
        // bridge row's own span is kept; for a bridge-only scene the upsert
        // conflicts so these just need to be in-bounds.
        let (first, last) = if !times.is_empty() {
            ((*times.first().unwrap()).clamp(start, end), (*times.last().unwrap()).clamp(start, end))
        } else {
            let (f, l, _) = existing.unwrap();
            (f.clamp(start, end), l.clamp(start, end))
        };
        // Estimate: lifetime play_duration apportioned across this scene's plays.
        let dur: i64 = sqlx::query_scalar(
            "SELECT COALESCE(CAST(json_extract(metadata_json,'$.play_duration_seconds') AS INTEGER),0)
             FROM content_items WHERE id = ?1",
        )
        .bind(cid)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        let lifetime: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM play_imports WHERE content_item_id = ?1")
                .bind(cid)
                .fetch_one(&mut *tx)
                .await?;
        let apportioned =
            ((dur as f64) * (times.len() as f64) / (lifetime.max(1) as f64)).round() as i64;
        // Watch estimate = the larger of Stash's recorded play_duration
        // (apportioned) and the REAL local playback (seconds_tracked - video
        // actually advanced, below the count threshold). Both are genuine watch
        // time. NEVER the tab-open span: last_seen_at ticks on every heartbeat
        // incl. paused ones, so an idle open tab's span = the whole session, which
        // isn't watch time (that's what wrongly logged session-length minutes).
        let local_secs = existing.map(|(_, _, s)| s).unwrap_or(0);
        let est = apportioned.max(local_secs);
        // Upsert: keep an existing (bridge) row's first/last span, just mark it
        // counted + bump watch time; create a new row otherwise.
        sqlx::query(
            "INSERT INTO scene_plays
                (session_id, content_item_id, first_seen_at, last_seen_at, seconds_tracked, counted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?3)
             ON CONFLICT(session_id, content_item_id) DO UPDATE SET
                counted_at = COALESCE(scene_plays.counted_at, excluded.counted_at),
                seconds_tracked = MAX(scene_plays.seconds_tracked, excluded.seconds_tracked)",
        )
        .bind(session_id)
        .bind(cid)
        .bind(first)
        .bind(last)
        .bind(est)
        .execute(&mut *tx)
        .await?;
    }

    for &oid in o_event_ids {
        // Adopt the session-less Stash cumshot (guard on still session-less +
        // inside the window). Keep stash_synced=1 - it's already in Stash.
        sqlx::query(
            "UPDATE o_events SET session_id = ?1
             WHERE id = ?2 AND session_id IS NULL AND occurred_at >= ?3 AND occurred_at <= ?4",
        )
        .bind(session_id)
        .bind(oid)
        .bind(start)
        .bind(end)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

/// Record SKIP decisions so the session-history review stops offering them.
///
/// When `mirror_to_reconstruction` is set, ALSO suppresses the skipped items from
/// RECONSTRUCTION (the "while you were away" / Untracked prompt). This is for the
/// WRAP-UP "watched elsewhere" skip only: there the session can auto-discard (its
/// only activity was the skipped items -> no content), and a discarded session no
/// longer "covers" its events, so `candidate_sessions` would re-propose exactly
/// what the user just declined. We mirror the skip into `reconstruction_dismissed`
/// (the table that scan consults): cumshots by their o_event id, each skipped
/// scene's `play_imports` inside the session window.
///
/// The SessionDetail fold-review skip passes this FALSE: it acts on an already-
/// ENDED session (no auto-discard), the events are covered while the session
/// exists, and mirroring would wrongly hide them from reconstruction forever if
/// that session is later trimmed or deleted (its activity is meant to be
/// re-homeable via Untracked sessions).
pub async fn dismiss_session_history(
    pool: &SqlitePool,
    session_id: i64,
    content_item_ids: &[i64],
    o_event_ids: &[i64],
    mirror_to_reconstruction: bool,
) -> Result<()> {
    let now = now_ms();
    // Session window (to map a skipped scene -> the play_imports it covers). Only
    // needed for the reconstruction mirror, so skip the read otherwise.
    let win: Option<(i64, Option<i64>)> = if mirror_to_reconstruction {
        sqlx::query_as("SELECT started_at, ended_at FROM sessions WHERE id = ?1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };

    let mut tx = pool.begin().await?;
    for &cid in content_item_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO session_history_dismissed (session_id, kind, ref_id, dismissed_at)
             VALUES (?1, 'play', ?2, ?3)",
        )
        .bind(session_id)
        .bind(cid)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    for &oid in o_event_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO session_history_dismissed (session_id, kind, ref_id, dismissed_at)
             VALUES (?1, 'o', ?2, ?3)",
        )
        .bind(session_id)
        .bind(oid)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // Mirror the skip into reconstruction_dismissed (see the doc comment).
    if let Some((sstart, sended)) = win {
        let send = sended.unwrap_or(now);
        for &oid in o_event_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO reconstruction_dismissed (kind, source_id, dismissed_at)
                 VALUES ('o', ?1, ?2)",
            )
            .bind(oid)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for &cid in content_item_ids {
            let pids: Vec<i64> = sqlx::query_scalar(
                "SELECT id FROM play_imports
                 WHERE content_item_id = ?1 AND played_at >= ?2 AND played_at <= ?3",
            )
            .bind(cid)
            .bind(sstart)
            .bind(send)
            .fetch_all(&mut *tx)
            .await?;
            for pid in pids {
                sqlx::query(
                    "INSERT OR IGNORE INTO reconstruction_dismissed (kind, source_id, dismissed_at)
                     VALUES ('play', ?1, ?2)",
                )
                .bind(pid)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(())
}

// ---------- EXTEND an existing session (the Sessions-page "Extend") ----------
//
// Named "Continue" until July 2026, which was the wrong word: users reach for
// "continue" to mean REOPEN a session they stopped by accident (see
// SessionManager::reopen). This one is initiated FROM an existing session and
// pulls the session-less activity that FOLLOWS it - up to the next non-discarded
// session, or now - into it, extending its end. The untracked gap before that
// activity is either a pause (excluded from session time) or counted, the user's
// choice (same prompt as merge). Reuses absorb_session_history for the fold, so
// the watch-time estimate is the same honest one (Stash play_duration / real
// local seconds, never the tab-open span).
//
// The three session actions are deliberately distinct:
//   Resume - un-pause a PAUSED session (SessionManager::resume)
//   Reopen - make an ENDED session live again (SessionManager::reopen)
//   Extend - fold trailing activity in + push ended_at forward (here)

#[derive(Debug, Clone, Serialize)]
pub struct ExtendSessionCheck {
    /// True if there's untracked activity after this session to fold in.
    pub can_extend: bool,
    /// Untracked gap (ms) between the session's end and that activity - what
    /// becomes a pause (or counts) per the user's choice.
    pub gap_ms: i64,
    /// Distinct scenes that would be folded in.
    pub scene_count: i64,
    /// Session-less cumshots that would be adopted.
    pub cumshot_count: i64,
}

/// Find the session-less activity immediately after `session_id` (up to the next
/// non-discarded session, or now). Returns (old_end, cids, oids, first_ts, last_ts).
async fn following_untracked(
    pool: &SqlitePool,
    session_id: i64,
) -> Result<Option<(i64, Vec<i64>, Vec<i64>, i64, i64)>> {
    let row: Option<(Option<i64>, String)> =
        sqlx::query_as("SELECT ended_at, status FROM sessions WHERE id = ?1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    let Some((ended, status)) = row else { return Ok(None) };
    if status == "discarded" {
        return Ok(None);
    }
    // An active/unfinished session has no end to continue from.
    let Some(old_end) = ended else { return Ok(None) };

    let now = now_ms();
    let next_start: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(started_at) FROM sessions WHERE status != 'discarded' AND started_at > ?1",
    )
    .bind(old_end)
    .fetch_one(pool)
    .await?;
    let window_end = next_start.unwrap_or(now);
    if window_end <= old_end {
        return Ok(None);
    }

    let cids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT cid FROM (
            SELECT content_item_id AS cid FROM play_imports
             WHERE played_at > ?1 AND played_at <= ?2
            UNION
            SELECT content_item_id AS cid FROM o_events
             WHERE session_id IS NULL AND content_item_id IS NOT NULL
               AND occurred_at > ?1 AND occurred_at <= ?2
         )",
    )
    .bind(old_end)
    .bind(window_end)
    .fetch_all(pool)
    .await?;
    let oids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM o_events
         WHERE session_id IS NULL AND occurred_at > ?1 AND occurred_at <= ?2",
    )
    .bind(old_end)
    .bind(window_end)
    .fetch_all(pool)
    .await?;
    if cids.is_empty() && oids.is_empty() {
        return Ok(None);
    }

    let (pmin, pmax): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT MIN(played_at), MAX(played_at) FROM play_imports
         WHERE played_at > ?1 AND played_at <= ?2",
    )
    .bind(old_end)
    .bind(window_end)
    .fetch_one(pool)
    .await?;
    let (omin, omax): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT MIN(occurred_at), MAX(occurred_at) FROM o_events
         WHERE session_id IS NULL AND occurred_at > ?1 AND occurred_at <= ?2",
    )
    .bind(old_end)
    .bind(window_end)
    .fetch_one(pool)
    .await?;
    let first_ts = [pmin, omin].into_iter().flatten().min();
    let last_ts = [pmax, omax].into_iter().flatten().max();
    let (Some(first_ts), Some(last_ts)) = (first_ts, last_ts) else {
        return Ok(None);
    };
    Ok(Some((old_end, cids, oids, first_ts, last_ts)))
}

/// Preview continuing a session: whether there's following untracked activity +
/// the gap + counts (for the Sessions-page prompt). Read-only.
pub async fn extend_session_check(
    pool: &SqlitePool,
    session_id: i64,
) -> Result<ExtendSessionCheck> {
    match following_untracked(pool, session_id).await? {
        None => Ok(ExtendSessionCheck {
            can_extend: false,
            gap_ms: 0,
            scene_count: 0,
            cumshot_count: 0,
        }),
        Some((old_end, cids, oids, first_ts, _last_ts)) => Ok(ExtendSessionCheck {
            can_extend: true,
            gap_ms: (first_ts - old_end).max(0),
            scene_count: cids.len() as i64,
            cumshot_count: oids.len() as i64,
        }),
    }
}

/// Continue a session: extend its end over the following untracked activity,
/// (optionally) pause the gap, and fold that activity in. Pure-local. Returns
/// whether anything was folded (false = nothing to continue).
pub async fn extend_session(
    pool: &SqlitePool,
    session_id: i64,
    gap_as_pause: bool,
) -> Result<bool> {
    let Some((old_end, cids, oids, first_ts, last_ts)) =
        following_untracked(pool, session_id).await?
    else {
        return Ok(false);
    };
    let now = now_ms();
    let new_end = old_end.max(last_ts);

    // ONE transaction: extend the session, (optionally) pause the untracked gap,
    // and fold the activity in (absorb reads the extended ended_at within this tx).
    // Atomic so a failure can't leave a phantom-extended session with the activity
    // stranded (a later retry would then read the new ended_at and miss it).
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE sessions SET ended_at = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(new_end)
        .bind(now)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    if gap_as_pause && first_ts > old_end {
        sqlx::query(
            "INSERT INTO session_pauses (session_id, paused_at, resumed_at, reason)
             VALUES (?1, ?2, ?3, 'manual')",
        )
        .bind(session_id)
        .bind(old_end)
        .bind(first_ts)
        .execute(&mut *tx)
        .await?;
    }
    // Fold the following activity into the now-extended session (counted
    // scene_plays + adopt the session-less cumshots), on the SAME tx.
    absorb_into_conn(&mut tx, session_id, &cids, &oids).await?;
    tx.commit().await?;
    Ok(true)
}
