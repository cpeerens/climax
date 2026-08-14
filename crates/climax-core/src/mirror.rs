// Stash-history mirror (Phase 7, component 1).
//
// Turns Climax from a witness-only log into a faithful viewer over a user's whole
// Stash history, and upholds the 1:1 mirror guarantee: Stash-derived numbers
// (per-scene / performer / studio O-counts, play counts) shown in Climax equal
// what Stash shows at that moment — including pre-Climax and closed-app activity.
//
// Two pieces of Stash truth, mirrored differently:
//   - O's become real session-less `o_events` (origin='stash', session_id=NULL,
//     occurred_at = the Stash timestamp). They live in the SAME table as
//     witnessed O's, so the existing aggregations (once broadened to stop
//     dropping session-less rows) become correct automatically AND the Trends /
//     Overview graphs light up over the full history.
//   - Plays / watch-time / last-played can't honestly be reconstructed as
//     per-play events (no per-play durations; scene_plays.session_id is NOT
//     NULL), so they're snapshotted as scalar counts in metadata_json
//     (SceneInfo::to_metadata_json) and SUMmed per entity in catalog.rs.
//
// Reconciliation is PURE-READ against Stash + local merge — it NEVER writes to
// Stash (no feedback loop: session-less origin='stash' events are never pushed by
// log_o), and it is idempotent (a second run inserts nothing). It manages only
// the session-less import layer it owns: it adds Stash O's not represented by any
// stash_synced Climax event, and prunes session-less imports whose timestamp has
// vanished from Stash. It never deletes session-bound witnessed events (the live
// `remove_external_o` path already mirrors in-session removals; auto-deleting real
// session history in a background pass would be unsafe).

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::catalog;
use crate::db::now_ms;
use crate::settings;
use crate::stash;

/// Tolerance when matching a Stash `o_history` timestamp to an existing Climax
/// o_event. Climax-pushed O's are exact-to-the-second (Stash floors to whole
/// seconds), but bridge-mirrored stash-origin O's can carry minor clock skew, so
/// we allow ~1.5s — far smaller than the gap between real O's.
const MATCH_TOL_MS: i64 = 1500;

/// Scenes fetched per page during the sync walk. Bulk counts + o_history come
/// back inline, so a moderate page keeps responses sane on large libraries.
const SYNC_PAGE: i64 = 100;

/// Live progress for the mirror, surfaced to the Settings UI. Held in AppState
/// behind an RwLock and updated as a sync runs.
#[derive(Debug, Clone, Serialize)]
pub struct MirrorStatus {
    /// "idle" | "syncing"
    pub phase: String,
    pub done: i64,
    pub total: i64,
    /// ms epoch when the last full import / sync finished, if any.
    pub last_finished_at: Option<i64>,
    /// Last error message, if the most recent run failed.
    pub last_error: Option<String>,
}

impl Default for MirrorStatus {
    fn default() -> Self {
        // phase starts "idle" (NOT "" from derive(Default)) so the UI's
        // is-running check reads correctly on a fresh app.
        Self {
            phase: "idle".into(),
            done: 0,
            total: 0,
            last_finished_at: None,
            last_error: None,
        }
    }
}

/// Result of a single-scene mirror self-check (testing / verification aid):
/// compares Climax's stash-synced O-event count for a scene to Stash's live
/// `o_counter`. `matches` is the mirror guarantee for that scene.
#[derive(Debug, Clone, Serialize)]
pub struct MirrorCheck {
    pub scene_id: String,
    pub content_id: Option<i64>,
    pub climax_o_events: i64,
    pub stash_o_counter: i64,
    pub matches: bool,
}

/// Look up the `stash` source row id (stable; created by the initial migration).
async fn stash_source_id(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT id FROM sources WHERE key = 'stash'")
        .fetch_one(pool)
        .await
        .context("look up stash source id")
}

/// Find-or-create the content_item row for a Stash scene id (same bare-bones
/// shape `session.rs::record_heartbeat` inserts: source + external_id +
/// timestamps; descriptive fields filled in by the reconcile that follows).
async fn ensure_content_item(
    conn: &mut sqlx::SqliteConnection,
    source_id: i64,
    scene_id: &str,
    now: i64,
) -> Result<i64> {
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM content_items WHERE source_id = ?1 AND external_id = ?2",
    )
    .bind(source_id)
    .bind(scene_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(id);
    }
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO content_items (source_id, external_id, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?3) RETURNING id",
    )
    .bind(source_id)
    .bind(scene_id)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

/// Merge a scene's Stash `o_history` (epoch-ms timestamps) into Climax's ledger.
/// Greedy nearest-match each Stash timestamp against existing stash_synced events
/// for the scene (each consumed once); insert session-less imports for unmatched
/// Stash timestamps; delete session-less imports no longer present in Stash.
/// Returns (inserted, deleted). Session-bound witnessed events are never touched.
async fn merge_o_history(
    conn: &mut sqlx::SqliteConnection,
    content_id: i64,
    stash_history: &[i64],
) -> Result<(usize, usize)> {
    // Existing events known to be in Stash (stash_synced). is_import flags the
    // session-less import layer this function owns (and may prune).
    let existing: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT id, occurred_at,
                CASE WHEN session_id IS NULL AND origin = 'stash' THEN 1 ELSE 0 END AS is_import
         FROM o_events
         WHERE content_item_id = ?1 AND stash_synced = 1
         ORDER BY occurred_at",
    )
    .bind(content_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut stash: Vec<(i64, bool)> = stash_history.iter().map(|&t| (t, false)).collect();
    let mut event_matched = vec![false; existing.len()];

    for (ei, (_id, occurred, _imp)) in existing.iter().enumerate() {
        let mut best: Option<(usize, i64)> = None; // (stash index, abs diff)
        for (si, (ts, used)) in stash.iter().enumerate() {
            if *used {
                continue;
            }
            let diff = (ts - occurred).abs();
            if diff <= MATCH_TOL_MS && best.is_none_or(|(_, d)| diff < d) {
                best = Some((si, diff));
            }
        }
        if let Some((si, _)) = best {
            stash[si].1 = true;
            event_matched[ei] = true;
        }
    }

    // Insert unmatched Stash timestamps as session-less imports.
    let now = now_ms();
    let mut inserted = 0usize;
    for (ts, used) in &stash {
        if *used {
            continue;
        }
        sqlx::query(
            "INSERT INTO o_events
                (session_id, content_item_id, occurred_at, intensity, notes, stash_synced, origin, created_at)
             VALUES (NULL, ?1, ?2, NULL, NULL, 1, 'stash', ?3)",
        )
        .bind(content_id)
        .bind(ts)
        .bind(now)
        .execute(&mut *conn)
        .await?;
        inserted += 1;
    }

    // Prune session-less imports that no longer exist in Stash.
    let mut deleted = 0usize;
    for (ei, (id, _occurred, is_import)) in existing.iter().enumerate() {
        if !event_matched[ei] && *is_import == 1 {
            sqlx::query("DELETE FROM o_events WHERE id = ?1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            deleted += 1;
        }
    }

    Ok((inserted, deleted))
}

/// Merge a scene's Stash `play_history` (epoch-ms timestamps) into the local
/// `play_imports` table — the play analog of `merge_o_history`. Plays carry no
/// witnessed-event skew (Climax never pushes plays to Stash), so this is a plain
/// exact-match reconcile: insert Stash timestamps missing locally, prune local
/// rows whose timestamp is gone from Stash. Idempotent (the UNIQUE constraint
/// backs the insert). Returns (inserted, deleted). Feeds session reconstruction.
async fn merge_play_history(
    conn: &mut sqlx::SqliteConnection,
    content_id: i64,
    stash_plays: &[i64],
) -> Result<(usize, usize)> {
    let existing: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, played_at FROM play_imports WHERE content_item_id = ?1",
    )
    .bind(content_id)
    .fetch_all(&mut *conn)
    .await?;

    let existing_times: HashSet<i64> = existing.iter().map(|(_, t)| *t).collect();
    let stash_set: HashSet<i64> = stash_plays.iter().copied().collect();
    let now = now_ms();

    let mut inserted = 0usize;
    for &t in &stash_set {
        if !existing_times.contains(&t) {
            sqlx::query(
                "INSERT OR IGNORE INTO play_imports (content_item_id, played_at, created_at)
                 VALUES (?1, ?2, ?3)",
            )
            .bind(content_id)
            .bind(t)
            .bind(now)
            .execute(&mut *conn)
            .await?;
            inserted += 1;
        }
    }

    let mut deleted = 0usize;
    for (id, t) in &existing {
        if !stash_set.contains(t) {
            sqlx::query("DELETE FROM play_imports WHERE id = ?1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            deleted += 1;
        }
    }

    Ok((inserted, deleted))
}

/// Write the per-scene mirror for one already-fetched scene onto `conn`: refresh
/// the descriptive + count snapshot in metadata_json (title/thumbnail/duration
/// via COALESCE so a transient null doesn't wipe a good value; metadata_json
/// replaced wholesale so Stash deletions take effect — same rule as the live
/// enrichment path), stamp both clocks, then merge the O-history. Caller owns the
/// transaction so a page of scenes can share one commit.
async fn reconcile_scene_inner(
    conn: &mut sqlx::SqliteConnection,
    content_id: i64,
    info: &stash::SceneInfo,
) -> Result<(usize, usize)> {
    let now = now_ms();
    sqlx::query(
        "UPDATE content_items
         SET title = COALESCE(?1, title),
             thumbnail_url = COALESCE(?2, thumbnail_url),
             duration_seconds = COALESCE(?3, duration_seconds),
             metadata_json = ?4,
             metadata_fetched_at = ?5,
             mirror_synced_at = ?5
         WHERE id = ?6",
    )
    .bind(&info.title)
    .bind(&info.thumbnail_url)
    .bind(info.duration_seconds)
    .bind(info.to_metadata_json())
    .bind(now)
    .bind(content_id)
    .execute(&mut *conn)
    .await?;

    // Mirror the per-play timestamps (feeds session reconstruction) alongside the
    // O-history. Plays go to their own table; the O merge's return is the one the
    // caller historically reports, so keep returning it.
    merge_play_history(conn, content_id, &info.play_history).await?;
    merge_o_history(conn, content_id, &info.o_history).await
}

/// Mirror sync (Phase 7): ask Stash for every scene WITH activity (play_count>0
/// OR o_counter>0), create/refresh its content_item, and reconcile it (snapshot
/// counts + merge O-history). This DISCOVERS scenes watched while Climax was
/// closed AND refreshes known ones — the faithful-mirror pass behind the
/// on-launch + periodic loop and the Settings "Sync now" button. Skips
/// zero-activity scenes so the catalog stays history-only. Batches a page per
/// transaction. Updates `status`; returns scenes processed.
pub async fn sync_active(pool: &SqlitePool, status: &Arc<RwLock<MirrorStatus>>) -> Result<usize> {
    {
        let mut s = status.write().await;
        s.phase = "syncing".into();
        s.done = 0;
        s.total = 0;
        s.last_error = None;
    }

    let source_id = stash_source_id(pool).await?;
    let mut page = 1i64;
    let mut processed = 0usize;

    let run = async {
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            let (scenes, total) = stash::find_scenes_page(pool, page, SYNC_PAGE, true).await?;
            if page == 1 {
                status.write().await.total = total;
            }
            if scenes.is_empty() {
                break;
            }
            let batch = scenes.len();

            let mut tx = pool.begin().await?;
            for (scene_id, info) in &scenes {
                let content_id = ensure_content_item(&mut tx, source_id, scene_id, now_ms()).await?;
                reconcile_scene_inner(&mut tx, content_id, info).await?;
                seen.insert(scene_id.clone());
            }
            tx.commit().await?;

            processed += batch;
            status.write().await.done = processed as i64;

            if (batch as i64) < SYNC_PAGE || processed as i64 >= total {
                break;
            }
            page += 1;
        }

        sweep_stale_imports(pool, &seen).await?;
        Ok::<usize, anyhow::Error>(processed)
    }
    .await;

    let mut s = status.write().await;
    match &run {
        Ok(_) => {
            s.phase = "idle".into();
            s.last_finished_at = Some(now_ms());
            s.last_error = None;
        }
        Err(e) => {
            s.phase = "idle".into();
            // last_error renders verbatim in Settings -> Sync. Stash failures
            // are already designed sentences, but a mid-sync DB failure's
            // top-level message is raw sqlx text - substitute it. The full
            // chain goes to the log either way.
            tracing::warn!("play-history sync failed: {:#}", e);
            s.last_error = Some(if e.chain().any(|c| c.downcast_ref::<sqlx::Error>().is_some()) {
                "The sync stopped partway - Climax couldn't read or write its own database. Try again.".to_string()
            } else {
                format!("{}", e)
            });
        }
    }
    run
}

/// Prune imported history for scenes Stash's active set no longer includes.
///
/// THE BUG THIS EXISTS FOR: `find_scenes_page` asks for scenes that still HAVE
/// activity, so a scene whose history dropped to zero is never revisited and its
/// `play_imports` linger forever. Those rows are uncovered by any session, so
/// reconstruction offers them straight back - meaning **deleting a session in
/// Climax would immediately propose recreating it**, since the delete's rollback
/// strips the history from Stash but leaves Climax's imported copies behind.
/// Found when a deleted test session reappeared under Untracked sessions.
///
/// Only scenes Climax holds imports for are considered, so this is normally a
/// no-op costing one query.
async fn sweep_stale_imports(pool: &SqlitePool, seen: &HashSet<String>) -> Result<()> {
    let candidates: Vec<(i64, String)> = sqlx::query_as(
        "SELECT ci.id, ci.external_id
           FROM content_items ci
           JOIN sources s ON s.id = ci.source_id
          WHERE s.key = 'stash' AND ci.external_id IS NOT NULL
            AND (EXISTS (SELECT 1 FROM play_imports pi WHERE pi.content_item_id = ci.id)
              OR EXISTS (SELECT 1 FROM o_events oe
                          WHERE oe.content_item_id = ci.id
                            AND oe.session_id IS NULL AND oe.origin = 'stash'))",
    )
    .fetch_all(pool)
    .await?;

    for (content_id, scene_id) in candidates {
        if seen.contains(&scene_id) {
            continue; // still active, already reconciled above
        }
        match stash::fetch_scene(pool, &scene_id).await {
            // Still exists, just with no activity left. Reconciling against its
            // real (empty) history is what prunes - same code path as any other
            // scene, so there is no second definition of "correct" to drift.
            Ok(Some(info)) => {
                let mut tx = pool.begin().await?;
                reconcile_scene_inner(&mut tx, content_id, &info).await?;
                tx.commit().await?;
            }
            // Gone from Stash entirely. Try to trace a merge first; if it can't
            // be traced, its imports can never be confirmed against Stash again,
            // so they are dropped rather than left to propose phantom sessions.
            // Deliberately re-importable: if the scene comes back, the next sync
            // reinstates them.
            Ok(None) => {
                let _ = sqlx::query(
                    "UPDATE content_items SET stash_missing_at = COALESCE(stash_missing_at, ?2)
                     WHERE id = ?1",
                )
                .bind(content_id)
                .bind(now_ms())
                .execute(pool)
                .await;
                match resolve_merged_scene(pool, content_id).await {
                    Ok(Some(target)) => {
                        tracing::info!("scene {} was merged into {}; history moved", scene_id, target);
                        continue;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("merge trace for scene {} failed: {:#}", scene_id, e);
                        continue; // a lookup failure is not evidence of anything
                    }
                }
                sqlx::query("DELETE FROM play_imports WHERE content_item_id = ?1")
                    .bind(content_id)
                    .execute(pool)
                    .await?;
                sqlx::query(
                    "DELETE FROM o_events
                      WHERE content_item_id = ?1 AND session_id IS NULL AND origin = 'stash'",
                )
                .bind(content_id)
                .execute(pool)
                .await?;
                // Nothing references it any more: no session ever saw it and no
                // imports remain, so the row is a pure orphan. A row a session
                // still points at is always kept - dropping it would blank that
                // scene out of the session's own history.
                let _ = sqlx::query(
                    "DELETE FROM content_items WHERE id = ?1
                       AND NOT EXISTS (SELECT 1 FROM scene_plays sp WHERE sp.content_item_id = ?1)
                       AND NOT EXISTS (SELECT 1 FROM o_events oe WHERE oe.content_item_id = ?1)",
                )
                .bind(content_id)
                .execute(pool)
                .await;
                tracing::info!("scene {} is gone from Stash; stale imports pruned", scene_id);
            }
            Err(e) => tracing::warn!("stale-import check for scene {} failed: {:#}", scene_id, e),
        }
    }
    Ok(())
}

/// Write one scene's descriptive metadata snapshot (title / thumbnail / duration
/// via COALESCE, full metadata_json via SceneInfo::to_metadata_json, stamping
/// metadata_fetched_at). Unlike the history reconcile this does NOT merge
/// o_history or touch the mirror clock — it refreshes DETAILS only.
async fn write_scene_metadata(
    pool: &SqlitePool,
    content_id: i64,
    info: &stash::SceneInfo,
) -> Result<()> {
    sqlx::query(
        "UPDATE content_items
         SET title = COALESCE(?1, title),
             thumbnail_url = COALESCE(?2, thumbnail_url),
             duration_seconds = COALESCE(?3, duration_seconds),
             metadata_json = ?4,
             metadata_fetched_at = ?5
         WHERE id = ?6",
    )
    .bind(&info.title)
    .bind(&info.thumbnail_url)
    .bind(info.duration_seconds)
    .bind(info.to_metadata_json())
    .bind(now_ms())
    .bind(content_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// "Library metadata" sync: refresh scene DETAILS (title / thumbnail /
/// performers / studio / tags) for known scenes, then performer / studio / tag
/// details + images (entity_meta). This is the catalog side, kept separate from
/// the play-history mirror (sync_active). `force` (manual "Refresh now")
/// refreshes everything; otherwise only stale scenes (metadata_fetched_at NULL
/// or older than the cadence) are re-fetched. Updates `status`; returns scenes
/// processed.
pub async fn sync_metadata(
    pool: &SqlitePool,
    status: &Arc<RwLock<MirrorStatus>>,
    force: bool,
) -> Result<usize> {
    {
        let mut s = status.write().await;
        s.phase = "syncing".into();
        s.done = 0;
        s.total = 0;
        s.last_error = None;
    }

    let run = async {
        let cfg = settings::get_metadata_refresh(pool).await.unwrap_or_default();
        let cutoff = now_ms() - cfg.ttl_seconds() * 1000;
        let scenes: Vec<(i64, String)> = if force {
            sqlx::query_as(
                "SELECT ci.id, ci.external_id FROM content_items ci
                 JOIN sources s ON s.id = ci.source_id
                 WHERE s.key = 'stash' AND ci.external_id IS NOT NULL",
            )
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT ci.id, ci.external_id FROM content_items ci
                 JOIN sources s ON s.id = ci.source_id
                 WHERE s.key = 'stash' AND ci.external_id IS NOT NULL
                   AND (ci.metadata_fetched_at IS NULL OR ci.metadata_fetched_at < ?1)",
            )
            .bind(cutoff)
            .fetch_all(pool)
            .await?
        };
        status.write().await.total = scenes.len() as i64;

        let mut processed = 0usize;
        let mut missing: Vec<(i64, String)> = Vec::new();
        for (content_id, scene_id) in &scenes {
            match stash::fetch_scene(pool, scene_id).await {
                Ok(Some(info)) => {
                    // Came back (or never left): clear any missing flag first, so a
                    // scene that reappears stops being hidden.
                    let _ = sqlx::query(
                        "UPDATE content_items SET stash_missing_at = NULL
                         WHERE id = ?1 AND stash_missing_at IS NOT NULL",
                    )
                    .bind(content_id)
                    .execute(pool)
                    .await;
                    if let Err(e) = write_scene_metadata(pool, *content_id, &info).await {
                        tracing::warn!("metadata write scene {} failed: {:#}", scene_id, e);
                    }
                }
                // Stash definitively has no such scene. Flag it and leave the stored
                // snapshot ALONE - overwriting it with an empty one is what produced
                // the "0 plays / never" tiles. Resolution happens after the loop.
                Ok(None) => {
                    let _ = sqlx::query(
                        "UPDATE content_items SET stash_missing_at = COALESCE(stash_missing_at, ?2)
                         WHERE id = ?1",
                    )
                    .bind(content_id)
                    .bind(now_ms())
                    .execute(pool)
                    .await;
                    missing.push((*content_id, scene_id.clone()));
                }
                Err(e) => tracing::warn!("metadata fetch scene {} failed: {:#}", scene_id, e),
            }
            processed += 1;
            status.write().await.done = processed as i64;
        }

        // Trace anything that vanished to the scene it was merged into, and move
        // its session history across. Runs after the fetch loop so it only ever
        // looks at scenes this pass confirmed missing.
        for (content_id, scene_id) in &missing {
            match resolve_merged_scene(pool, *content_id).await {
                Ok(Some(target)) => {
                    tracing::info!("scene {} was merged into {}; history moved", scene_id, target)
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("merge trace for scene {} failed: {:#}", scene_id, e),
            }
        }

        // Entity details + images (performer / studio / tag).
        for kind in ["performer", "studio", "tag"] {
            if let Err(e) = catalog::enrich_entities(pool, kind, force).await {
                tracing::warn!("metadata enrich {} failed: {:#}", kind, e);
            }
        }
        Ok::<usize, anyhow::Error>(processed)
    }
    .await;

    let mut s = status.write().await;
    match &run {
        Ok(_) => {
            s.phase = "idle".into();
            s.last_finished_at = Some(now_ms());
            s.last_error = None;
        }
        Err(e) => {
            s.phase = "idle".into();
            // Same as sync_active: the UI shows last_error verbatim, so a raw
            // sqlx top-level message gets substituted; the log keeps the detail.
            tracing::warn!("library metadata sync failed: {:#}", e);
            s.last_error = Some(if e.chain().any(|c| c.downcast_ref::<sqlx::Error>().is_some()) {
                "The refresh stopped partway - Climax couldn't read or write its own database. Try again.".to_string()
            } else {
                format!("{}", e)
            });
        }
    }
    run
}

/// Single-scene mirror self-check (verification aid). Compares Climax's
/// Trace a scene Stash no longer has to the scene it was MERGED into, and move
/// this one's history onto it. Returns the survivor's Stash id when it resolved.
///
/// Why the filename. `sceneMerge` deletes the source and records nothing about
/// where it went, but it hands the source's FILES to the survivor - so the
/// basename is the only thread that crosses a merge. Confirmed against three
/// real merges in a live library, including one where the survivor's title bore
/// no resemblance to the source's and the file was the sole link.
///
/// Deliberately CONSERVATIVE. It applies only when exactly one scene owns that
/// exact basename. `path`'s INCLUDES is a substring test, so a short or generic
/// filename can hit several scenes, and a wrong guess would silently rewrite
/// which scene a past session says you watched. Ambiguous cases stay flagged and
/// simply drop out of the browse grids.
async fn resolve_merged_scene(pool: &SqlitePool, content_id: i64) -> Result<Option<String>> {
    // The basename is stored in the snapshot from the first sighting onward. For
    // rows that predate that, fall back to the title, which IS the basename when
    // Stash had no title of its own (the existing title fallback).
    let meta: Option<String> =
        sqlx::query_scalar("SELECT metadata_json FROM content_items WHERE id = ?1")
            .bind(content_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM content_items WHERE id = ?1")
        .bind(content_id)
        .fetch_optional(pool)
        .await?
        .flatten();

    let stored = meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("file_basename").and_then(|b| b.as_str()).map(String::from));
    let looks_like_a_file = |s: &str| {
        let l = s.to_ascii_lowercase();
        [".mp4", ".mkv", ".avi", ".wmv", ".mov", ".m4v", ".webm", ".flv"]
            .iter()
            .any(|ext| l.ends_with(ext))
    };
    let Some(needle) = stored.or(title) else {
        return Ok(None); // nothing to trace it by
    };
    let needle = needle.trim().to_string();
    if needle.len() < 8 {
        return Ok(None); // too generic to identify anything safely
    }

    // A stored basename is the whole filename, so anchor it to the end of the
    // path. A title is only ever PART of one (Stash drops the extension, and
    // often a suffix like " [UPSCALE]"), so it has to match loosely - which is
    // why the uniqueness check below carries the weight in that case.
    let is_filename = looks_like_a_file(&needle);
    let pattern = if is_filename {
        format!("{}$", stash::escape_regex_literal(&needle))
    } else {
        stash::escape_regex_literal(&needle)
    };

    let matches = stash::find_scenes_by_path(pool, &pattern).await?;
    let hits: Vec<_> = matches
        .into_iter()
        .filter(|m| {
            m.basenames.iter().any(|b| {
                if is_filename {
                    b == &needle
                } else {
                    b.contains(&needle)
                }
            })
        })
        .collect();
    // Exactly one, or nothing happens. A wrong guess would silently rewrite which
    // scene a past session says you watched, which is worse than leaving a row
    // hidden, so anything ambiguous stays flagged for a human to look at.
    let [target] = hits.as_slice() else {
        return Ok(None);
    };
    let target_id = target.id.clone();

    repoint_to_merged_scene(pool, content_id, &target_id).await?;
    Ok(Some(target_id))
}

/// Move a merged-away scene's CLIMAX-OWNED history onto the survivor, then drop
/// the dead row.
///
/// Only the session links move. Stash-derived numbers (play counts, watch time,
/// o_history) are deliberately left behind, because the merge already moved them
/// to the survivor in Stash and the mirror has already imported them there -
/// carrying them over as well would count the same watching twice.
async fn repoint_to_merged_scene(
    pool: &SqlitePool,
    dead_id: i64,
    target_external_id: &str,
) -> Result<()> {
    let source_id = stash_source_id(pool).await?;

    let mut tx = pool.begin().await?;
    let target_id =
        ensure_content_item(&mut tx, source_id, target_external_id, now_ms()).await?;
    if target_id == dead_id {
        tx.rollback().await?;
        return Ok(());
    }

    // A session can already hold the survivor (you watched both halves of what is
    // now one scene). Fold into that row rather than violating the per-session
    // uniqueness: earliest first-seen, latest last-seen, summed seconds, and keep
    // a counted_at if either side had one.
    sqlx::query(
        "UPDATE scene_plays AS keep SET
            first_seen_at    = MIN(keep.first_seen_at, dead.first_seen_at),
            last_seen_at     = MAX(keep.last_seen_at, dead.last_seen_at),
            seconds_tracked  = keep.seconds_tracked + dead.seconds_tracked,
            counted_at       = COALESCE(keep.counted_at, dead.counted_at),
            last_advance_at  = MAX(COALESCE(keep.last_advance_at, 0), COALESCE(dead.last_advance_at, 0)),
            -- Must be summed too, and this was missed first time round: the
            -- rollback on a session delete prefers stash_watched_secs over
            -- seconds_tracked, so dropping the dead row's copy left watch time
            -- stranded in Stash when that session was later deleted. Caught for
            -- real - 244s went to 117 instead of 22. NULL only when BOTH are
            -- NULL, so the fallback to seconds_tracked still works for rows that
            -- never had a measurement.
            stash_watched_secs = CASE
                WHEN keep.stash_watched_secs IS NULL AND dead.stash_watched_secs IS NULL THEN NULL
                ELSE COALESCE(keep.stash_watched_secs, 0) + COALESCE(dead.stash_watched_secs, 0)
            END
         FROM scene_plays AS dead
         WHERE dead.content_item_id = ?1
           AND keep.content_item_id = ?2
           AND keep.session_id = dead.session_id",
    )
    .bind(dead_id)
    .bind(target_id)
    .execute(&mut *tx)
    .await?;

    // The runs belong to the playback, not to the row, so they move onto the
    // survivor before the dead row is dropped - otherwise the CASCADE below
    // takes them and a folded session silently loses half its detail. The
    // un-folded case further down just repoints content_item_id on the same
    // row, so its runs come along for free.
    sqlx::query(
        "UPDATE scene_play_runs
            SET play_id = (SELECT keep.id
                             FROM scene_plays keep
                             JOIN scene_plays dead ON dead.session_id = keep.session_id
                            WHERE dead.id = scene_play_runs.play_id
                              AND keep.content_item_id = ?2)
          WHERE play_id IN (SELECT id FROM scene_plays
                             WHERE content_item_id = ?1
                               AND session_id IN (SELECT session_id FROM scene_plays
                                                   WHERE content_item_id = ?2))",
    )
    .bind(dead_id)
    .bind(target_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM scene_plays WHERE content_item_id = ?1
           AND session_id IN (SELECT session_id FROM scene_plays WHERE content_item_id = ?2)",
    )
    .bind(dead_id)
    .bind(target_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE scene_plays SET content_item_id = ?2 WHERE content_item_id = ?1")
        .bind(dead_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;

    // Cumshots move wholesale. A session-less Stash import would be re-derived
    // from the survivor anyway, and one logged in Climax belongs to the watching,
    // not to the scene row that happened to hold it.
    sqlx::query("UPDATE o_events SET content_item_id = ?2 WHERE content_item_id = ?1")
        .bind(dead_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;

    // play_imports are Stash's own timestamps; the survivor already has them from
    // the merge, so the dead scene's copies are duplicates.
    sqlx::query("DELETE FROM play_imports WHERE content_item_id = ?1")
        .bind(dead_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM content_items WHERE id = ?1")
        .bind(dead_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// stash_synced O-event count for the scene to Stash's live `o_counter`.
pub async fn check_scene(pool: &SqlitePool, scene_id: &str) -> Result<MirrorCheck> {
    let info = stash::fetch_scene(pool, scene_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("That scene no longer exists in Stash."))?;
    let content_id: Option<i64> = sqlx::query_scalar(
        "SELECT ci.id FROM content_items ci
         JOIN sources s ON s.id = ci.source_id
         WHERE s.key = 'stash' AND ci.external_id = ?1",
    )
    .bind(scene_id)
    .fetch_optional(pool)
    .await?;
    let climax: i64 = match content_id {
        Some(cid) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM o_events WHERE content_item_id = ?1 AND stash_synced = 1")
                .bind(cid)
                .fetch_one(pool)
                .await?
        }
        None => 0,
    };
    Ok(MirrorCheck {
        scene_id: scene_id.to_string(),
        content_id,
        climax_o_events: climax,
        stash_o_counter: info.o_counter,
        matches: climax == info.o_counter,
    })
}

#[cfg(test)]
mod merge_repoint_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory db");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    /// A scene row plus the session scaffolding a scene_play needs.
    async fn seed(pool: &SqlitePool) -> (i64, i64, i64) {
        // An in-memory pool is capped at ONE connection (a second would be a
        // different database), so never hold an acquired connection across a
        // query that takes its own - that deadlocks into PoolTimedOut.
        let source_id = stash_source_id(pool).await.expect("stash source");
        let session: i64 = sqlx::query_scalar(
            "INSERT INTO sessions (started_at, ended_at, status, assigned_day, created_at, updated_at)
             VALUES (1000, 2000, 'ended', '2026-07-01', 1000, 1000) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let dead = ensure_content_item(&mut conn, source_id, "6807", 1000)
            .await
            .unwrap();
        let target = ensure_content_item(&mut conn, source_id, "4617", 1000)
            .await
            .unwrap();
        drop(conn);
        (dead, target, session)
    }

    async fn play(pool: &SqlitePool, session: i64, item: i64, first: i64, last: i64, secs: i64) {
        sqlx::query(
            "INSERT INTO scene_plays (session_id, content_item_id, first_seen_at, last_seen_at, seconds_tracked)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(session).bind(item).bind(first).bind(last).bind(secs)
        .execute(pool).await.unwrap();
    }

    /// The plain case: the survivor was not in that session, so the row moves.
    #[tokio::test]
    async fn moves_the_session_link_to_the_survivor() {
        let pool = fresh_pool().await;
        let (dead, target, session) = seed(&pool).await;
        play(&pool, session, dead, 1100, 1500, 42).await;

        repoint_to_merged_scene(&pool, dead, "4617").await.unwrap();

        let (item, secs): (i64, i64) =
            sqlx::query_as("SELECT content_item_id, seconds_tracked FROM scene_plays")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(item, target, "the play should now belong to the survivor");
        assert_eq!(secs, 42, "watch time must not change when it simply moves");

        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM content_items WHERE id = ?1")
            .bind(dead).fetch_one(&pool).await.unwrap();
        assert_eq!(gone, 0, "the dead row should be removed once nothing points at it");
    }

    /// THE ONE THAT MATTERS: both halves of what is now one scene were watched in
    /// the SAME session. The rows must fold together, not collide.
    #[tokio::test]
    async fn folds_into_an_existing_row_in_the_same_session() {
        let pool = fresh_pool().await;
        let (dead, target, session) = seed(&pool).await;
        play(&pool, session, target, 1200, 1400, 30).await;
        play(&pool, session, dead, 1100, 1600, 45).await;

        repoint_to_merged_scene(&pool, dead, "4617").await.unwrap();

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scene_plays")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1, "one scene in one session means one row");

        let (item, first, last, secs): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT content_item_id, first_seen_at, last_seen_at, seconds_tracked FROM scene_plays",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(item, target);
        assert_eq!(first, 1100, "keeps the earliest sighting");
        assert_eq!(last, 1600, "keeps the latest sighting");
        assert_eq!(secs, 75, "watch time adds up rather than being lost");
    }

    /// THE REGRESSION FROM THE LIVE TEST. The rollback on a session delete
    /// prefers stash_watched_secs, so if the fold drops one side's copy the
    /// watch time is stranded in Stash forever. Seen for real: 244s should have
    /// rolled back to 22 and stopped at 117.
    #[tokio::test]
    async fn measured_watch_time_survives_the_fold() {
        let pool = fresh_pool().await;
        let (dead, target, session) = seed(&pool).await;
        play(&pool, session, target, 1200, 1400, 30).await;
        play(&pool, session, dead, 1100, 1600, 45).await;
        sqlx::query("UPDATE scene_plays SET stash_watched_secs = 127 WHERE content_item_id = ?1")
            .bind(target).execute(&pool).await.unwrap();
        sqlx::query("UPDATE scene_plays SET stash_watched_secs = 117 WHERE content_item_id = ?1")
            .bind(dead).execute(&pool).await.unwrap();

        repoint_to_merged_scene(&pool, dead, "4617").await.unwrap();

        let measured: Option<i64> =
            sqlx::query_scalar("SELECT stash_watched_secs FROM scene_plays")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(measured, Some(244), "both measurements must survive the fold");
    }

    /// A row that never had a measurement must stay NULL, so the rollback still
    /// falls back to seconds_tracked rather than reading a fabricated zero.
    #[tokio::test]
    async fn unmeasured_rows_stay_null_after_the_fold() {
        let pool = fresh_pool().await;
        let (dead, target, session) = seed(&pool).await;
        play(&pool, session, target, 1200, 1400, 30).await;
        play(&pool, session, dead, 1100, 1600, 45).await;

        repoint_to_merged_scene(&pool, dead, "4617").await.unwrap();

        let measured: Option<i64> =
            sqlx::query_scalar("SELECT stash_watched_secs FROM scene_plays")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(measured, None, "no measurement on either side means no measurement after");
    }

    /// Cumshots follow the watching, not the scene row that happened to hold them.
    #[tokio::test]
    async fn cumshots_move_across() {
        let pool = fresh_pool().await;
        let (dead, target, session) = seed(&pool).await;
        sqlx::query(
            "INSERT INTO o_events (session_id, content_item_id, occurred_at, origin, stash_synced, created_at)
             VALUES (?1, ?2, 1300, 'climax', 1, 1300)",
        )
        .bind(session).bind(dead).execute(&pool).await.unwrap();

        repoint_to_merged_scene(&pool, dead, "4617").await.unwrap();

        let item: i64 = sqlx::query_scalar("SELECT content_item_id FROM o_events")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(item, target);
    }

    /// Re-pointing a scene at itself must be a no-op, not a self-delete.
    #[tokio::test]
    async fn pointing_at_itself_changes_nothing() {
        let pool = fresh_pool().await;
        let (dead, _target, session) = seed(&pool).await;
        play(&pool, session, dead, 1100, 1500, 42).await;

        repoint_to_merged_scene(&pool, dead, "6807").await.unwrap();

        let still: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM content_items WHERE id = ?1")
            .bind(dead).fetch_one(&pool).await.unwrap();
        assert_eq!(still, 1, "must not delete the row it was told to keep");
        let plays: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scene_plays")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(plays, 1);
    }
}
