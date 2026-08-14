// Trends section aggregations — Phase 6.
//
// Read-only. Powers the dashboard Trends page (analytics overview + report
// builder). Three aggregations:
//   - daily_series: contiguous, zero-filled per-day {sessions, cumshots,
//     watch_time_ms} over a date range. The frontend slices this client-side
//     for the summary cards, area chart, busiest-days, heatmap, and all
//     time-grouped report rows (mirrors the design's data-trends.js).
//   - hour_histogram: per-LOCAL-hour {cumshots, sessions, active_days}. The one
//     place that uses RAW timestamps (occurred_at / started_at) instead of
//     assigned_day, per the app's "time-of-day analysis uses raw timestamps"
//     convention.
//   - entity_breakdown: range-scoped per performer / studio / tag totals for
//     the leaderboards + report entity groupings.
//
// Day-bucketed metrics use sessions.assigned_day and exclude status='discarded',
// matching dashboard.rs / catalog.rs. Watch time uses raw elapsed
// (COALESCE(ended_at, now) - started_at), consistent with
// dashboard::daily_buckets / the unfiltered Overview chart path.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::now_ms;

#[derive(Debug, Clone, Serialize)]
pub struct DailyActivity {
    /// YYYY-MM-DD (assigned_day).
    pub date: String,
    pub sessions: i64,
    pub cumshots: i64,
    pub watch_time_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HourHistogram {
    /// 24 buckets, indexed by local hour (0..23).
    pub cumshots: Vec<i64>,
    pub sessions: Vec<i64>,
    pub active_days: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityBreakdownRow {
    pub id: String,
    pub name: String,
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub sessions: i64,
    pub scene_count: i64,
    /// Performer dimension only: Stash GenderEnum from entity_meta (None when
    /// not yet enriched / unset in Stash / not a performer row).
    pub gender: Option<String>,
}

/// `occurred_at` (epoch ms) of every session-less Stash-history O import whose
/// LOCAL date falls in `[start_day, end_day]`. These imports (Phase 7) have
/// `session_id IS NULL` and no `assigned_day`, so day/hour-bucketed trends
/// attribute them by the `chrono::Local` date/hour of `occurred_at` — the same
/// basis `hour_histogram` uses for session events. Returned raw so each caller
/// buckets as it needs (by day for the series, by hour for the histogram).
async fn import_o_timestamps(
    pool: &SqlitePool,
    start_day: &str,
    end_day: &str,
) -> Result<Vec<i64>> {
    let (ts_lo, ts_hi) = crate::db::local_day_bounds_ms(start_day, end_day)?;
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT occurred_at FROM o_events
         WHERE session_id IS NULL AND occurred_at >= ?1 AND occurred_at < ?2",
    )
    .bind(ts_lo)
    .bind(ts_hi)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(t,)| t).collect())
}

// ---------- daily series ----------

/// Contiguous per-day activity over [start_day, end_day] inclusive. Days with
/// no activity are present with zeros so the frontend has a gap-free series to
/// slice for every overview chart.
pub async fn daily_series(
    pool: &SqlitePool,
    start_day: &str,
    end_day: &str,
) -> Result<Vec<DailyActivity>> {
    let start = NaiveDate::parse_from_str(start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(end_day, "%Y-%m-%d")?;
    if end < start {
        return Ok(vec![]);
    }
    let now = now_ms();

    // Sessions + watch time per assigned_day (one grouped query).
    let sess_rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT assigned_day,
                COUNT(*) AS sessions,
                COALESCE(SUM(COALESCE(ended_at, ?3) - started_at), 0) AS watch_ms
         FROM sessions
         WHERE assigned_day BETWEEN ?1 AND ?2 AND status != 'discarded'
         GROUP BY assigned_day",
    )
    .bind(start_day)
    .bind(end_day)
    .bind(now)
    .fetch_all(pool)
    .await?;

    // Cumshots per assigned_day.
    let o_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.assigned_day, COUNT(*) AS cumshots
         FROM o_events oe JOIN sessions s ON s.id = oe.session_id
         WHERE s.assigned_day BETWEEN ?1 AND ?2 AND s.status != 'discarded'
         GROUP BY s.assigned_day",
    )
    .bind(start_day)
    .bind(end_day)
    .fetch_all(pool)
    .await?;

    let mut sess_map: HashMap<String, (i64, i64)> = HashMap::new();
    for (d, sessions, watch_ms) in sess_rows {
        sess_map.insert(d, (sessions, watch_ms));
    }
    let mut o_map: HashMap<String, i64> = HashMap::new();
    for (d, c) in o_rows {
        o_map.insert(d, c);
    }

    // Session-less Stash-history imports (Phase 7): no assigned_day, so
    // attribute each by the LOCAL date of its occurred_at and fold into the
    // per-day cumshot totals so the series covers full Stash history.
    for occurred_at in import_o_timestamps(pool, start_day, end_day).await? {
        if let Some(day) = crate::db::local_day_of_ms(occurred_at) {
            *o_map.entry(day).or_insert(0) += 1;
        }
    }

    let mut out = Vec::new();
    let mut d = start;
    while d <= end {
        let key = d.format("%Y-%m-%d").to_string();
        let (sessions, watch_time_ms) = sess_map.get(&key).copied().unwrap_or((0, 0));
        let cumshots = o_map.get(&key).copied().unwrap_or(0);
        out.push(DailyActivity {
            date: key,
            sessions,
            cumshots,
            watch_time_ms,
        });
        match d.succ_opt() {
            Some(n) => d = n,
            None => break,
        }
    }
    Ok(out)
}

// ---------- hour-of-day histogram ----------

/// Per-local-hour distribution. Buckets RAW timestamps by LOCAL hour. Scope:
/// activity whose owning session's assigned_day falls in [start_day, end_day]
/// and is non-discarded.
///   - cumshots[h]    = o_events at local-hour h
///   - sessions[h]    = sessions whose started_at is at local-hour h
///   - active_days[h] = distinct assigned_days with ANY activity at hour h
pub async fn hour_histogram(
    pool: &SqlitePool,
    start_day: &str,
    end_day: &str,
) -> Result<HourHistogram> {
    let o_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT oe.occurred_at, s.assigned_day
         FROM o_events oe JOIN sessions s ON s.id = oe.session_id
         WHERE s.assigned_day BETWEEN ?1 AND ?2 AND s.status != 'discarded'",
    )
    .bind(start_day)
    .bind(end_day)
    .fetch_all(pool)
    .await?;

    let s_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT started_at, assigned_day
         FROM sessions
         WHERE assigned_day BETWEEN ?1 AND ?2 AND status != 'discarded'",
    )
    .bind(start_day)
    .bind(end_day)
    .fetch_all(pool)
    .await?;

    let mut cumshots = vec![0i64; 24];
    let mut sessions = vec![0i64; 24];
    let mut active_sets: Vec<HashSet<String>> = (0..24).map(|_| HashSet::new()).collect();

    let hour_of = |ms: i64| -> Option<usize> {
        Local
            .timestamp_millis_opt(ms)
            .single()
            .map(|dt| dt.hour() as usize)
    };

    for (occurred_at, day) in o_rows {
        if let Some(h) = hour_of(occurred_at) {
            cumshots[h] += 1;
            active_sets[h].insert(day);
        }
    }
    for (started_at, day) in s_rows {
        if let Some(h) = hour_of(started_at) {
            sessions[h] += 1;
            active_sets[h].insert(day);
        }
    }

    // Session-less Stash-history imports (Phase 7): bucket by local hour and
    // attribute their active-day by the local date of occurred_at (they have no
    // session/assigned_day). A history-only O still counts as activity.
    for occurred_at in import_o_timestamps(pool, start_day, end_day).await? {
        if let (Some(h), Some(day)) = (hour_of(occurred_at), crate::db::local_day_of_ms(occurred_at))
        {
            cumshots[h] += 1;
            active_sets[h].insert(day);
        }
    }

    let active_days = active_sets.iter().map(|s| s.len() as i64).collect();
    Ok(HourHistogram {
        cumshots,
        sessions,
        active_days,
    })
}

// ---------- entity breakdown (range-scoped) ----------

#[derive(sqlx::FromRow)]
struct EntityStatRow {
    id: String,
    name: String,
    scene_count: i64,
    watch_time_ms: i64,
    cumshots: i64,
}

#[derive(sqlx::FromRow)]
struct SessionCountRow {
    id: String,
    sessions: i64,
}

/// Range-scoped per-entity totals for `dimension` ∈ {scene, performer, studio,
/// tag}. Facet dimensions reuse the json_each pattern from catalog.rs; the
/// scene dimension is a direct per-content_item rollup. Every per-scene rollup
/// is clipped to sessions whose assigned_day is in [start_day, end_day].
/// Ordered cumshots-desc, watch-desc by default; the frontend re-sorts by the
/// selected metric.
pub async fn entity_breakdown(
    pool: &SqlitePool,
    dimension: &str,
    start_day: &str,
    end_day: &str,
) -> Result<Vec<EntityBreakdownRow>> {
    if dimension == "scene" {
        return scene_breakdown(pool, start_day, end_day).await;
    }
    let path = match dimension {
        "performer" => Some("$.performers"),
        "tag" => Some("$.tags"),
        "studio" => None,
        _ => return Ok(vec![]),
    };

    // Epoch bounds for attributing session-less imports by occurred_at (Phase 7).
    let (ts_lo, ts_hi) = crate::db::local_day_bounds_ms(start_day, end_day)?;

    // Range-scoped per-scene rollup. ?1 = start_day, ?2 = end_day (reused);
    // ?3 = ts_lo, ?4 = ts_hi (import occurred_at bounds, cumshots only).
    let cte = "WITH scene_stats AS (
        SELECT
            ci.id AS cid,
            ci.metadata_json AS meta,
            COALESCE((SELECT SUM(sp.seconds_tracked) FROM scene_plays sp
                      JOIN sessions s ON s.id = sp.session_id
                      WHERE sp.content_item_id = ci.id AND s.status != 'discarded'
                        AND s.assigned_day BETWEEN ?1 AND ?2), 0) AS watch_s,
            COALESCE((SELECT COUNT(*) FROM o_events oe
                      LEFT JOIN sessions s ON s.id = oe.session_id
                      WHERE oe.content_item_id = ci.id
                        AND ((oe.session_id IS NOT NULL AND s.status != 'discarded'
                              AND s.assigned_day BETWEEN ?1 AND ?2)
                          OR (oe.session_id IS NULL
                              AND oe.occurred_at >= ?3 AND oe.occurred_at < ?4))), 0) AS cumshots
        FROM content_items ci
        WHERE ci.metadata_json IS NOT NULL
    )";

    let stats: Vec<EntityStatRow> = if let Some(p) = path {
        let sql = format!(
            "{cte}
             SELECT
                CAST(json_extract(j.value, '$.id') AS TEXT) AS id,
                CAST(json_extract(j.value, '$.name') AS TEXT) AS name,
                COUNT(*) AS scene_count,
                SUM(ss.watch_s) * 1000 AS watch_time_ms,
                SUM(ss.cumshots) AS cumshots
             FROM scene_stats ss, json_each(ss.meta, '{path}') j
             WHERE (ss.watch_s > 0 OR ss.cumshots > 0)
               AND json_extract(j.value, '$.id') IS NOT NULL
             GROUP BY json_extract(j.value, '$.id')",
            cte = cte,
            path = p,
        );
        sqlx::query_as(&sql)
            .bind(start_day)
            .bind(end_day)
            .bind(ts_lo)
            .bind(ts_hi)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!(
            "{cte}
             SELECT
                CAST(json_extract(ss.meta, '$.studio.id') AS TEXT) AS id,
                CAST(json_extract(ss.meta, '$.studio.name') AS TEXT) AS name,
                COUNT(*) AS scene_count,
                SUM(ss.watch_s) * 1000 AS watch_time_ms,
                SUM(ss.cumshots) AS cumshots
             FROM scene_stats ss
             WHERE (ss.watch_s > 0 OR ss.cumshots > 0)
               AND json_extract(ss.meta, '$.studio.id') IS NOT NULL
             GROUP BY json_extract(ss.meta, '$.studio.id')",
            cte = cte,
        );
        sqlx::query_as(&sql)
            .bind(start_day)
            .bind(end_day)
            .bind(ts_lo)
            .bind(ts_hi)
            .fetch_all(pool)
            .await?
    };

    let sessions = entity_sessions(pool, path, start_day, end_day).await?;

    // Performer gender from the entity_meta cache (empty string = fetched but
    // unset in Stash → normalised to None here).
    let genders: HashMap<String, String> = if dimension == "performer" {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT external_id, gender FROM entity_meta WHERE kind = 'performer'",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|(id, g)| g.filter(|s| !s.is_empty()).map(|s| (id, s)))
        .collect()
    } else {
        HashMap::new()
    };

    let mut out: Vec<EntityBreakdownRow> = stats
        .into_iter()
        .map(|r| EntityBreakdownRow {
            sessions: sessions.get(&r.id).copied().unwrap_or(0),
            gender: genders.get(&r.id).cloned(),
            id: r.id,
            name: r.name,
            scene_count: r.scene_count,
            watch_time_ms: r.watch_time_ms,
            cumshots: r.cumshots,
        })
        .collect();
    out.sort_by(|a, b| {
        b.cumshots
            .cmp(&a.cumshots)
            .then(b.watch_time_ms.cmp(&a.watch_time_ms))
    });
    Ok(out)
}

/// Per-scene range-scoped totals — the "scene" entity dimension. id is the
/// content_items.id (stringified, matching the scene chip filter), name falls
/// back to "Scene <external_id>" for untitled rows. No metadata requirement
/// (unlike the facet dimensions, a scene without enrichment still counts).
async fn scene_breakdown(
    pool: &SqlitePool,
    start_day: &str,
    end_day: &str,
) -> Result<Vec<EntityBreakdownRow>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        name: String,
        watch_time_ms: i64,
        cumshots: i64,
    }
    // Epoch bounds for attributing session-less imports by occurred_at (Phase 7).
    let (ts_lo, ts_hi) = crate::db::local_day_bounds_ms(start_day, end_day)?;
    let rows: Vec<Row> = sqlx::query_as(
        "WITH scene_stats AS (
            SELECT ci.id AS cid,
                   COALESCE(NULLIF(ci.title, ''), 'Scene ' || COALESCE(ci.external_id, CAST(ci.id AS TEXT))) AS title,
                   COALESCE((SELECT SUM(sp.seconds_tracked) FROM scene_plays sp
                             JOIN sessions s ON s.id = sp.session_id
                             WHERE sp.content_item_id = ci.id AND s.status != 'discarded'
                               AND s.assigned_day BETWEEN ?1 AND ?2), 0) AS watch_s,
                   COALESCE((SELECT COUNT(*) FROM o_events oe
                             LEFT JOIN sessions s ON s.id = oe.session_id
                             WHERE oe.content_item_id = ci.id
                               AND ((oe.session_id IS NOT NULL AND s.status != 'discarded'
                                     AND s.assigned_day BETWEEN ?1 AND ?2)
                                 OR (oe.session_id IS NULL
                                     AND oe.occurred_at >= ?3 AND oe.occurred_at < ?4))), 0) AS cumshots
            FROM content_items ci
        )
        SELECT CAST(ss.cid AS TEXT) AS id,
               ss.title AS name,
               ss.watch_s * 1000 AS watch_time_ms,
               ss.cumshots AS cumshots
        FROM scene_stats ss
        WHERE ss.watch_s > 0 OR ss.cumshots > 0",
    )
    .bind(start_day)
    .bind(end_day)
    .bind(ts_lo)
    .bind(ts_hi)
    .fetch_all(pool)
    .await?;

    // Distinct sessions per scene (plays ∪ o_events, range-clipped) — aliased
    // derived table + qualified columns per the SQLite flattening gotcha.
    let sess_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT CAST(u.cid AS TEXT) AS id, COUNT(DISTINCT u.session_id) AS sessions FROM (
            SELECT sp.content_item_id AS cid, sp.session_id AS session_id
            FROM scene_plays sp
              JOIN sessions s ON s.id = sp.session_id AND s.status != 'discarded'
            WHERE s.assigned_day BETWEEN ?1 AND ?2
            UNION
            SELECT oe.content_item_id AS cid, oe.session_id AS session_id
            FROM o_events oe
              JOIN sessions s ON s.id = oe.session_id AND s.status != 'discarded'
            WHERE oe.content_item_id IS NOT NULL AND s.assigned_day BETWEEN ?1 AND ?2
         ) AS u GROUP BY u.cid",
    )
    .bind(start_day)
    .bind(end_day)
    .fetch_all(pool)
    .await?;
    let sessions: HashMap<String, i64> = sess_rows.into_iter().collect();

    let mut out: Vec<EntityBreakdownRow> = rows
        .into_iter()
        .map(|r| EntityBreakdownRow {
            sessions: sessions.get(&r.id).copied().unwrap_or(0),
            gender: None,
            scene_count: 1,
            id: r.id,
            name: r.name,
            watch_time_ms: r.watch_time_ms,
            cumshots: r.cumshots,
        })
        .collect();
    out.sort_by(|a, b| {
        b.cumshots
            .cmp(&a.cumshots)
            .then(b.watch_time_ms.cmp(&a.watch_time_ms))
    });
    Ok(out)
}

/// Distinct non-discarded sessions (in range) in which a scene carrying each
/// entity was played OR had a cumshot logged — kept consistent with the
/// cumshots stat. `path` None → studio object facet. Mirrors
/// catalog::array_facet_sessions with an assigned_day range clip.
async fn entity_sessions(
    pool: &SqlitePool,
    path: Option<&str>,
    start_day: &str,
    end_day: &str,
) -> Result<HashMap<String, i64>> {
    let sql = if let Some(p) = path {
        format!(
            "SELECT u.id, COUNT(DISTINCT u.session_id) AS sessions FROM (
                SELECT CAST(json_extract(j.value, '$.id') AS TEXT) AS id, sp.session_id AS session_id
                FROM content_items ci
                  JOIN json_each(ci.metadata_json, '{path}') j
                  JOIN scene_plays sp ON sp.content_item_id = ci.id
                  JOIN sessions s ON s.id = sp.session_id AND s.status != 'discarded'
                WHERE ci.metadata_json IS NOT NULL AND json_extract(j.value, '$.id') IS NOT NULL
                  AND s.assigned_day BETWEEN ?1 AND ?2
                UNION
                SELECT CAST(json_extract(j.value, '$.id') AS TEXT) AS id, oe.session_id AS session_id
                FROM content_items ci
                  JOIN json_each(ci.metadata_json, '{path}') j
                  JOIN o_events oe ON oe.content_item_id = ci.id
                  JOIN sessions s ON s.id = oe.session_id AND s.status != 'discarded'
                WHERE ci.metadata_json IS NOT NULL AND json_extract(j.value, '$.id') IS NOT NULL
                  AND s.assigned_day BETWEEN ?1 AND ?2
             ) AS u GROUP BY u.id",
            path = p,
        )
    } else {
        "SELECT u.id, COUNT(DISTINCT u.session_id) AS sessions FROM (
            SELECT CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT) AS id, sp.session_id AS session_id
            FROM content_items ci
              JOIN scene_plays sp ON sp.content_item_id = ci.id
              JOIN sessions s ON s.id = sp.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL
              AND s.assigned_day BETWEEN ?1 AND ?2
            UNION
            SELECT CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT) AS id, oe.session_id AS session_id
            FROM content_items ci
              JOIN o_events oe ON oe.content_item_id = ci.id
              JOIN sessions s ON s.id = oe.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL
              AND s.assigned_day BETWEEN ?1 AND ?2
         ) AS u GROUP BY u.id"
            .to_string()
    };
    let rows: Vec<SessionCountRow> = sqlx::query_as(&sql)
        .bind(start_day)
        .bind(end_day)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.sessions)).collect())
}
