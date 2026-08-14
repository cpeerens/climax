// Dashboard / Overview queries.
//
// All queries here are read-only and tuned for the Overview section's
// "high-level" view: hero stats + monthly bar chart. Detail-level work
// (per-session timeline, per-day session strip) goes in later passes.
//
// Day-bucketed metrics use `sessions.assigned_day` (the user-attributed
// day) rather than raw `started_at`, matching the convention used
// everywhere else in the app.

use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct HeroStats {
    /// Cumshots whose owning session has assigned_day = today's local date.
    pub cumshots_today: i64,
    /// Most cumshots ever logged for a single assigned_day, plus the day it
    /// happened on (YYYY-MM-DD).
    pub record_day_count: i64,
    pub record_day_date: Option<String>,
    /// Consecutive days (including today) of any session activity. Streak
    /// counts logging activity — NOT cumshot frequency. The two are
    /// separate metrics per the design conventions.
    pub current_streak_days: i64,
    /// True when today hasn't been logged yet but the streak is otherwise
    /// alive (latest logged day is yesterday). UI shows ⚠ icon for this.
    pub streak_at_risk: bool,
    /// The LONGEST run of consecutive logged days ever, and the days it ran
    /// between (YYYY-MM-DD, inclusive). Measures the same thing as
    /// `current_streak_days` - days with any session - just unanchored from
    /// today, so it can sit anywhere in history. When the run still going IS
    /// the record these simply describe it; no special case.
    pub record_streak_days: i64,
    pub record_streak_start: Option<String>,
    pub record_streak_end: Option<String>,
    /// Days since the most recent cumshot (by assigned_day). None if no
    /// cumshots have ever been logged.
    pub days_since_last_cumshot: Option<i64>,
    /// Raw elapsed time of the longest session in the DB, ignoring pauses.
    /// Good enough for v1; we can switch to effective_duration_ms later if
    /// it matters (pauses are uncommon in normal use).
    pub longest_session_ms: i64,
    pub longest_session_date: Option<String>,
    /// Session id of the longest session, so the Overview hero card can open it
    /// in the day drill-down. None when no ended session exists yet.
    pub longest_session_id: Option<i64>,

    // ---- "this month" period stats (sub-row of hero) ----
    /// Watch time this calendar month, ms. Sum of effective duration of
    /// every session whose assigned_day falls in the current YYYY-MM.
    pub month_watch_time_ms: i64,
    pub month_cumshots: i64,
    pub month_sessions: i64,
    /// Distinct days within the current month that have at least one
    /// session — gives "active days this month".
    pub month_active_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthlyBucket {
    /// YYYY-MM for the bucket.
    pub month: String,
    /// Sum of effective session duration for that month, ms.
    pub watch_time_ms: i64,
    /// Total cumshots in that month.
    pub cumshots: i64,
    /// Total session count in that month.
    pub sessions: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyBucket {
    /// YYYY-MM-DD for the bucket (assigned_day).
    pub day: String,
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub sessions: i64,
}

/// Bucket for the filter-aware bar chart. Granularity is chosen by the
/// caller based on the date range duration: daily for short ranges,
/// monthly for long ones. The `key` is the human label of the bucket
/// (YYYY-MM-DD or YYYY-MM) and also doubles as the drill-down key when
/// the user clicks a bar.
#[derive(Debug, Clone, Serialize)]
pub struct RangeBucket {
    pub key: String,
    pub granularity: String, // "day" | "month"
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub sessions: i64,
}

/// Aggregate stats for the second hero row when a filter is active.
/// Mirrors the "this month" fields on HeroStats but scoped to the
/// filter's date range and entity selections.
#[derive(Debug, Clone, Serialize)]
pub struct RangeStats {
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub sessions: i64,
    pub active_days: i64,
}

/// Filter parameters that all the *_filtered queries share. start/end
/// are YYYY-MM-DD inclusive (matching sessions.assigned_day). Entity ID
/// vectors apply with OR within and AND across when populated; empty
/// vectors mean "no filter on this dimension".
#[derive(Debug, Clone, Default)]
pub struct FilterParams {
    pub start_day: String,
    pub end_day: String,
    pub scene_content_item_ids: Vec<i64>,
    pub performer_ids: Vec<String>,
    pub studio_ids: Vec<String>,
    pub tag_ids: Vec<String>,
}

/// Repeat `?` n times comma-joined, for dynamic IN clauses. Returns
/// "NULL" when n=0 so callers can drop it into `IN (NULL)` (an always-
/// false expression) and skip a branch — but in practice every caller
/// gates on `!ids.is_empty()` and skips the whole clause.
fn ph(n: usize) -> String {
    if n == 0 {
        return "NULL".to_string();
    }
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

/// True when ANY chip filter (scene/performer/studio/tag) has selections.
/// Callers use this to switch metrics between session-level semantics
/// (unfiltered) and scene-attributable semantics (filtered) — when a
/// chip is active, "watch time" and "cumshots" should only count what's
/// directly tied to matching scenes, not the whole session.
fn has_chip_filter(f: &FilterParams) -> bool {
    !f.scene_content_item_ids.is_empty()
        || !f.performer_ids.is_empty()
        || !f.studio_ids.is_empty()
        || !f.tag_ids.is_empty()
}

/// AND-joined predicate on a `content_items` row aliased `ci`. Returns
/// "1=1" when no chips are set (callers can splice unconditionally) and
/// the bind args still flow through bind_chip_scalar in the canonical
/// order (scene IDs, performer IDs, studio IDs, tag IDs).
///
/// Semantics:
///   - scene_ids:     `ci.id IN (...)`
///   - performer_ids: inner EXISTS over json_each($.performers) — scene must feature ANY selected performer
///   - studio_ids:    `json_extract(ci.metadata_json, '$.studio.id') IN (...)`
///   - tag_ids:       inner EXISTS over json_each($.tags), ANY of selected
///
/// OR within a chip type; AND across chip types. Matches the "performer X
/// AND studio Y → scenes featuring X that are from Y" expectation.
///
/// Note on string formatting: use real newlines, NOT `\<newline>` line
/// continuations. Rust's `\<newline>` eats both the newline AND the next
/// line's leading whitespace, which mashes table aliases into keywords
/// (`sp_pJOIN ...`) and silently produces invalid SQL.
fn content_item_match_sql(f: &FilterParams) -> String {
    let mut parts: Vec<String> = vec![];
    if !f.scene_content_item_ids.is_empty() {
        parts.push(format!("ci.id IN ({})", ph(f.scene_content_item_ids.len())));
    }
    if !f.performer_ids.is_empty() {
        parts.push(format!(
            "EXISTS (
                SELECT 1 FROM json_each(ci.metadata_json, '$.performers') jp
                WHERE json_extract(jp.value, '$.id') IN ({})
            )",
            ph(f.performer_ids.len())
        ));
    }
    if !f.studio_ids.is_empty() {
        parts.push(format!(
            "json_extract(ci.metadata_json, '$.studio.id') IN ({})",
            ph(f.studio_ids.len())
        ));
    }
    if !f.tag_ids.is_empty() {
        parts.push(format!(
            "EXISTS (
                SELECT 1 FROM json_each(ci.metadata_json, '$.tags') jt
                WHERE json_extract(jt.value, '$.id') IN ({})
            )",
            ph(f.tag_ids.len())
        ));
    }
    if parts.is_empty() {
        "1=1".to_string()
    } else {
        parts.join(" AND ")
    }
}

fn bind_chip_scalar<'q, T>(
    mut q: sqlx::query::QueryScalar<'q, sqlx::Sqlite, T, sqlx::sqlite::SqliteArguments<'q>>,
    f: &'q FilterParams,
) -> sqlx::query::QueryScalar<'q, sqlx::Sqlite, T, sqlx::sqlite::SqliteArguments<'q>> {
    for id in &f.scene_content_item_ids { q = q.bind(id); }
    for id in &f.performer_ids { q = q.bind(id); }
    for id in &f.studio_ids { q = q.bind(id); }
    for id in &f.tag_ids { q = q.bind(id); }
    q
}

/// Cumshot count over the local-day range `[lo_day, hi_day]` inclusive,
/// chip-aware, INCLUDING session-less Stash-history imports (Phase 7).
///
/// Session-bound events are attributed by `assigned_day`; imports
/// (`session_id IS NULL`, no `assigned_day`) by the local date of `occurred_at`,
/// captured via the range's epoch bounds. When `chip_active`, imports still
/// match through `content_items` (they carry a `content_item_id`), so a
/// performer/scene/etc. chart matches the broadened browse stats from Stage A.
async fn cumshots_count(
    pool: &SqlitePool,
    f: &FilterParams,
    lo_day: &str,
    hi_day: &str,
    chip_active: bool,
    cm: &str,
) -> Result<i64> {
    let (ts_lo, ts_hi) = crate::db::local_day_bounds_ms(lo_day, hi_day)?;
    let count: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM o_events oe
                 LEFT JOIN sessions s ON s.id = oe.session_id
                 JOIN content_items ci ON ci.id = oe.content_item_id
                 WHERE ((oe.session_id IS NOT NULL AND s.assigned_day BETWEEN ? AND ?
                         AND s.status != 'discarded')
                     OR (oe.session_id IS NULL AND oe.occurred_at >= ? AND oe.occurred_at < ?))
                   AND {cm}"
            ))
            .bind(lo_day)
            .bind(hi_day)
            .bind(ts_lo)
            .bind(ts_hi),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM o_events oe
             LEFT JOIN sessions s ON s.id = oe.session_id
             WHERE (oe.session_id IS NOT NULL AND s.assigned_day BETWEEN ? AND ?
                    AND s.status != 'discarded')
                OR (oe.session_id IS NULL AND oe.occurred_at >= ? AND oe.occurred_at < ?)",
        )
        .bind(lo_day)
        .bind(hi_day)
        .bind(ts_lo)
        .bind(ts_hi)
        .fetch_one(pool)
        .await?
    };
    Ok(count)
}

/// Aggregate watch-time / cumshots / sessions / active days within the
/// filter range. Same shape as the existing "this month" hero sub-row
/// but with full filter awareness.
pub async fn range_stats(pool: &SqlitePool, f: &FilterParams) -> Result<RangeStats> {
    let chip_active = has_chip_filter(f);
    let cm = content_item_match_sql(f);

    // Semantics: when a chip filter is active, every metric narrows to
    // what's directly attributable to scenes matching the filter. Watch
    // time becomes "time spent on those scenes" (sum of seconds_tracked),
    // cumshots becomes "cumshots logged on those scenes", and
    // sessions / active_days count distinct sessions/days where any
    // matching scene was played. When no chip filter is set, all four
    // fall back to the original session-level definitions.
    //
    // Bare `?` placeholders only (no `?N`) — bind order matches SQL order.
    let watch_time_ms: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COALESCE(SUM(sp.seconds_tracked * 1000), 0)
                 FROM scene_plays sp
                 JOIN sessions s ON s.id = sp.session_id
                 JOIN content_items ci ON ci.id = sp.content_item_id
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded'
                   AND {cm}"
            ))
            .bind(&f.start_day)
            .bind(&f.end_day),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        let now_ms_val = crate::db::now_ms();
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(COALESCE(s.ended_at, ?) - s.started_at), 0)
             FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(now_ms_val)
        .bind(&f.start_day)
        .bind(&f.end_day)
        .fetch_one(pool)
        .await?
    };

    let cumshots = cumshots_count(pool, f, &f.start_day, &f.end_day, chip_active, &cm).await?;

    let sessions: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM sessions s
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded'
                   AND EXISTS (
                       SELECT 1 FROM scene_plays sp
                       JOIN content_items ci ON ci.id = sp.content_item_id
                       WHERE sp.session_id = s.id
                         AND {cm}
                   )"
            ))
            .bind(&f.start_day)
            .bind(&f.end_day),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(&f.start_day)
        .bind(&f.end_day)
        .fetch_one(pool)
        .await?
    };

    let active_days: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(DISTINCT s.assigned_day) FROM sessions s
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded'
                   AND EXISTS (
                       SELECT 1 FROM scene_plays sp
                       JOIN content_items ci ON ci.id = sp.content_item_id
                       WHERE sp.session_id = s.id
                         AND {cm}
                   )"
            ))
            .bind(&f.start_day)
            .bind(&f.end_day),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT s.assigned_day) FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(&f.start_day)
        .bind(&f.end_day)
        .fetch_one(pool)
        .await?
    };

    Ok(RangeStats { watch_time_ms, cumshots, sessions, active_days })
}

/// Bar chart data within the filter range. Granularity:
///   - "day"   → one bucket per assigned_day from start to end (inclusive)
///   - "month" → one bucket per YYYY-MM intersecting [start, end]
///
/// The caller picks based on the range duration. Empty days/months are
/// returned with zeros so the X-axis is contiguous.
pub async fn filtered_buckets(
    pool: &SqlitePool,
    f: &FilterParams,
    granularity: &str,
) -> Result<Vec<RangeBucket>> {
    match granularity {
        "day" => filtered_day_buckets(pool, f).await,
        "week" => filtered_week_buckets(pool, f).await,
        "month" => filtered_month_buckets(pool, f).await,
        "year" => filtered_year_buckets(pool, f).await,
        _ => Ok(vec![]),
    }
}

/// Earliest assigned_day across non-discarded sessions, or None if no
/// sessions have been recorded yet. Used by the "All time" preset to
/// dynamically clip its start to actual data instead of padding empty
/// leading bars back to year 2000.
pub async fn first_session_day(pool: &SqlitePool) -> Result<Option<String>> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT MIN(assigned_day) FROM sessions
         WHERE status != 'discarded' AND assigned_day IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(raw)
}

async fn filtered_day_buckets(pool: &SqlitePool, f: &FilterParams) -> Result<Vec<RangeBucket>> {
    let start = NaiveDate::parse_from_str(&f.start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(&f.end_day, "%Y-%m-%d")?;
    if end < start { return Ok(vec![]); }

    let chip_active = has_chip_filter(f);
    let cm = content_item_match_sql(f);
    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::new();
    let mut d = start;
    while d <= end {
        let day = d.format("%Y-%m-%d").to_string();

        let watch_time_ms: i64 = if chip_active {
            bind_chip_scalar(
                sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT COALESCE(SUM(sp.seconds_tracked * 1000), 0)
                     FROM scene_plays sp
                     JOIN sessions s ON s.id = sp.session_id
                     JOIN content_items ci ON ci.id = sp.content_item_id
                     WHERE s.assigned_day = ? AND s.status != 'discarded' AND {cm}"
                ))
                .bind(&day),
                f,
            )
            .fetch_one(pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(COALESCE(s.ended_at, ?) - s.started_at), 0)
                 FROM sessions s
                 WHERE s.assigned_day = ? AND s.status != 'discarded'",
            )
            .bind(now_ms_val)
            .bind(&day)
            .fetch_one(pool)
            .await?
        };

        let cumshots = cumshots_count(pool, f, &day, &day, chip_active, &cm).await?;

        let sessions: i64 = if chip_active {
            bind_chip_scalar(
                sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT COUNT(*) FROM sessions s
                     WHERE s.assigned_day = ? AND s.status != 'discarded'
                       AND EXISTS (
                           SELECT 1 FROM scene_plays sp
                           JOIN content_items ci ON ci.id = sp.content_item_id
                           WHERE sp.session_id = s.id AND {cm}
                       )"
                ))
                .bind(&day),
                f,
            )
            .fetch_one(pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sessions s
                 WHERE s.assigned_day = ? AND s.status != 'discarded'",
            )
            .bind(&day)
            .fetch_one(pool)
            .await?
        };

        out.push(RangeBucket {
            key: day,
            granularity: "day".to_string(),
            watch_time_ms,
            cumshots,
            sessions,
        });
        match d.succ_opt() {
            Some(next) => d = next,
            None => break,
        }
    }
    Ok(out)
}

async fn filtered_month_buckets(pool: &SqlitePool, f: &FilterParams) -> Result<Vec<RangeBucket>> {
    let start = NaiveDate::parse_from_str(&f.start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(&f.end_day, "%Y-%m-%d")?;
    if end < start { return Ok(vec![]); }

    let chip_active = has_chip_filter(f);
    let cm = content_item_match_sql(f);
    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::new();
    let mut year = start.year();
    let mut month = start.month() as i32;
    let end_year = end.year();
    let end_month = end.month() as i32;
    while (year, month) <= (end_year, end_month) {
        let label = format!("{:04}-{:02}", year, month);
        // For the first / last buckets the day range is clipped by start/end.
        let bucket_first = NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap();
        let bucket_last = {
            let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
            NaiveDate::from_ymd_opt(ny, nm as u32, 1).unwrap().pred_opt().unwrap()
        };
        let lo = bucket_first.max(start).format("%Y-%m-%d").to_string();
        let hi = bucket_last.min(end).format("%Y-%m-%d").to_string();

        let (watch_time_ms, cumshots, sessions) =
            range_bucket_metrics(pool, f, &lo, &hi, chip_active, &cm, now_ms_val).await?;

        out.push(RangeBucket {
            key: label,
            granularity: "month".to_string(),
            watch_time_ms,
            cumshots,
            sessions,
        });

        month += 1;
        if month > 12 { month = 1; year += 1; }
    }
    Ok(out)
}

/// Shared per-bucket fetcher for week/month/year buckets — all three iterate
/// over `[lo, hi]` ranges and need the same three metrics (watch_time,
/// cumshots, sessions) computed with the same chip-aware switching logic.
/// Returns `(watch_time_ms, cumshots, sessions)` for the bucket.
async fn range_bucket_metrics(
    pool: &SqlitePool,
    f: &FilterParams,
    lo: &str,
    hi: &str,
    chip_active: bool,
    cm: &str,
    now_ms_val: i64,
) -> Result<(i64, i64, i64)> {
    let watch_time_ms: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COALESCE(SUM(sp.seconds_tracked * 1000), 0)
                 FROM scene_plays sp
                 JOIN sessions s ON s.id = sp.session_id
                 JOIN content_items ci ON ci.id = sp.content_item_id
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded' AND {cm}"
            ))
            .bind(lo)
            .bind(hi),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(COALESCE(s.ended_at, ?) - s.started_at), 0)
             FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(now_ms_val)
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await?
    };

    let cumshots = cumshots_count(pool, f, lo, hi, chip_active, cm).await?;

    let sessions: i64 = if chip_active {
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM sessions s
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded'
                   AND EXISTS (
                       SELECT 1 FROM scene_plays sp
                       JOIN content_items ci ON ci.id = sp.content_item_id
                       WHERE sp.session_id = s.id AND {cm}
                   )"
            ))
            .bind(lo)
            .bind(hi),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await?
    };

    Ok((watch_time_ms, cumshots, sessions))
}

async fn filtered_week_buckets(pool: &SqlitePool, f: &FilterParams) -> Result<Vec<RangeBucket>> {
    let start = NaiveDate::parse_from_str(&f.start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(&f.end_day, "%Y-%m-%d")?;
    if end < start { return Ok(vec![]); }

    let chip_active = has_chip_filter(f);
    let cm = content_item_match_sql(f);
    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::new();

    // Snap to the Monday of the start week (Mon-Sun ISO convention).
    let days_from_monday = start.weekday().num_days_from_monday() as i64;
    let mut monday = start - Duration::days(days_from_monday);
    while monday <= end {
        let sunday = monday + Duration::days(6);
        // Clip the bucket's effective range to the filter [start, end] so
        // partial weeks at the edges only count their in-range days.
        let lo = monday.max(start).format("%Y-%m-%d").to_string();
        let hi = sunday.min(end).format("%Y-%m-%d").to_string();
        // Key is always the Monday (unclipped) — that's what the frontend
        // wants for label formatting + drill targeting.
        let key = monday.format("%Y-%m-%d").to_string();

        let (watch_time_ms, cumshots, sessions) =
            range_bucket_metrics(pool, f, &lo, &hi, chip_active, &cm, now_ms_val).await?;

        out.push(RangeBucket {
            key,
            granularity: "week".to_string(),
            watch_time_ms,
            cumshots,
            sessions,
        });

        monday += Duration::days(7);
    }
    Ok(out)
}

async fn filtered_year_buckets(pool: &SqlitePool, f: &FilterParams) -> Result<Vec<RangeBucket>> {
    let start = NaiveDate::parse_from_str(&f.start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(&f.end_day, "%Y-%m-%d")?;
    if end < start { return Ok(vec![]); }

    let chip_active = has_chip_filter(f);
    let cm = content_item_match_sql(f);
    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::new();
    for year in start.year()..=end.year() {
        let label = format!("{:04}", year);
        let year_start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
        let year_end = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
        let lo = year_start.max(start).format("%Y-%m-%d").to_string();
        let hi = year_end.min(end).format("%Y-%m-%d").to_string();

        let (watch_time_ms, cumshots, sessions) =
            range_bucket_metrics(pool, f, &lo, &hi, chip_active, &cm, now_ms_val).await?;

        out.push(RangeBucket {
            key: label,
            granularity: "year".to_string(),
            watch_time_ms,
            cumshots,
            sessions,
        });
    }
    Ok(out)
}

/// Count of distinct non-discarded sessions matching the filter. Used
/// by the "N sessions match" indicator below the chips. When the chip
/// filter is active, sessions must contain at least one matching
/// scene_play to count.
pub async fn filtered_session_count(pool: &SqlitePool, f: &FilterParams) -> Result<i64> {
    let count: i64 = if has_chip_filter(f) {
        let cm = content_item_match_sql(f);
        bind_chip_scalar(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM sessions s
                 WHERE s.assigned_day BETWEEN ? AND ?
                   AND s.status != 'discarded'
                   AND EXISTS (
                       SELECT 1 FROM scene_plays sp
                       JOIN content_items ci ON ci.id = sp.content_item_id
                       WHERE sp.session_id = s.id AND {cm}
                   )"
            ))
            .bind(&f.start_day)
            .bind(&f.end_day),
            f,
        )
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions s
             WHERE s.assigned_day BETWEEN ? AND ?
               AND s.status != 'discarded'",
        )
        .bind(&f.start_day)
        .bind(&f.end_day)
        .fetch_one(pool)
        .await?
    };
    Ok(count)
}

/// {id, name} pairs deduped across all content_items. Used by the
/// performer / studio / tag chip pickers. Stash-side IDs.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NamedEntity {
    pub id: String,
    pub name: String,
}

pub async fn list_performers(pool: &SqlitePool) -> Result<Vec<NamedEntity>> {
    let rows: Vec<NamedEntity> = sqlx::query_as(
        "SELECT DISTINCT
           CAST(json_extract(p.value, '$.id') AS TEXT) AS id,
           CAST(json_extract(p.value, '$.name') AS TEXT) AS name
         FROM content_items ci, json_each(ci.metadata_json, '$.performers') p
         WHERE ci.metadata_json IS NOT NULL
           AND json_extract(p.value, '$.id') IS NOT NULL
         ORDER BY name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_studios(pool: &SqlitePool) -> Result<Vec<NamedEntity>> {
    let rows: Vec<NamedEntity> = sqlx::query_as(
        "SELECT DISTINCT
           CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT) AS id,
           CAST(json_extract(ci.metadata_json, '$.studio.name') AS TEXT) AS name
         FROM content_items ci
         WHERE ci.metadata_json IS NOT NULL
           AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL
         ORDER BY name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_tags(pool: &SqlitePool) -> Result<Vec<NamedEntity>> {
    let rows: Vec<NamedEntity> = sqlx::query_as(
        "SELECT DISTINCT
           CAST(json_extract(t.value, '$.id') AS TEXT) AS id,
           CAST(json_extract(t.value, '$.name') AS TEXT) AS name
         FROM content_items ci, json_each(ci.metadata_json, '$.tags') t
         WHERE ci.metadata_json IS NOT NULL
           AND json_extract(t.value, '$.id') IS NOT NULL
         ORDER BY name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Compact scene row for the scene-picker typeahead.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SceneEntity {
    pub id: i64,                  // content_items.id (NOT Stash external_id)
    pub title: Option<String>,
    pub external_id: Option<String>,
    pub thumbnail_url: Option<String>,
}

/// Search the catalog by title (case-insensitive substring) or exact
/// external_id match. Ordered by recency (last_seen_at desc).
pub async fn search_scenes(pool: &SqlitePool, query: &str, limit: i64) -> Result<Vec<SceneEntity>> {
    let q = query.trim();
    let pattern = format!("%{}%", q);
    let rows: Vec<SceneEntity> = sqlx::query_as(
        "SELECT id, title, external_id, thumbnail_url
         FROM content_items
         WHERE (title LIKE ?1 COLLATE NOCASE) OR (external_id = ?2)
         ORDER BY last_seen_at DESC
         LIMIT ?3",
    )
    .bind(&pattern)
    .bind(q)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn hero_stats(pool: &SqlitePool) -> Result<HeroStats> {
    let today_local = Local::now().date_naive();
    let today_str = today_local.format("%Y-%m-%d").to_string();
    let month_prefix = today_local.format("%Y-%m").to_string();

    // --- Cumshots today (session-bound + session-less Stash imports) ---
    let (today_lo, today_hi) = crate::db::local_day_bounds_ms(&today_str, &today_str)?;
    let cumshots_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM o_events oe
         LEFT JOIN sessions s ON s.id = oe.session_id
         WHERE (oe.session_id IS NOT NULL AND s.assigned_day = ?1 AND s.status != 'discarded')
            OR (oe.session_id IS NULL AND oe.occurred_at >= ?2 AND oe.occurred_at < ?3)",
    )
    .bind(&today_str)
    .bind(today_lo)
    .bind(today_hi)
    .fetch_one(pool)
    .await?;

    // --- Record day (most cumshots in any single day, all-time) ---
    // Session-bound O's count by assigned_day; session-less Stash imports have
    // no assigned_day, so they count by the local date of occurred_at. Merge
    // both per-day tallies in Rust, then take the max (ties → latest day).
    let sess_day_counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.assigned_day, COUNT(*) AS c
         FROM o_events oe JOIN sessions s ON s.id = oe.session_id
         WHERE s.assigned_day IS NOT NULL AND s.status != 'discarded'
         GROUP BY s.assigned_day",
    )
    .fetch_all(pool)
    .await?;
    let import_ts: Vec<(i64,)> =
        sqlx::query_as("SELECT occurred_at FROM o_events WHERE session_id IS NULL")
            .fetch_all(pool)
            .await?;
    let mut day_counts: HashMap<String, i64> = sess_day_counts.into_iter().collect();
    for (occurred_at,) in import_ts {
        if let Some(day) = crate::db::local_day_of_ms(occurred_at) {
            *day_counts.entry(day).or_insert(0) += 1;
        }
    }
    let (record_day_date, record_day_count) = day_counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .map(|(d, c)| (Some(d), c))
        .unwrap_or((None, 0));

    // --- Streaks (consecutive days with any non-discarded session) ---
    // Every distinct assigned_day, walked in Rust. Deliberately UNLIMITED: the
    // current streak only ever needs recent days, but the record can sit
    // anywhere in history, and a 400-row cap would silently hide a long run
    // from two years ago. One row per day ever logged, so it stays small.
    let active_days: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT assigned_day FROM sessions
         WHERE status != 'discarded' AND assigned_day IS NOT NULL
         ORDER BY assigned_day DESC",
    )
    .fetch_all(pool)
    .await?;
    let (current_streak_days, streak_at_risk) = compute_streak(&active_days, today_local);
    let (record_streak_days, record_streak_start, record_streak_end) =
        longest_streak(&active_days);

    // --- Days since last cumshot (latest of session-bound + import) ---
    let last_sess_o_day: Option<String> = sqlx::query_scalar(
        "SELECT MAX(s.assigned_day) FROM o_events oe
         JOIN sessions s ON s.id = oe.session_id
         WHERE s.assigned_day IS NOT NULL AND s.status != 'discarded'",
    )
    .fetch_one(pool)
    .await?;
    let last_import_o_day: Option<String> =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(occurred_at) FROM o_events WHERE session_id IS NULL")
            .fetch_one(pool)
            .await?
            .and_then(crate::db::local_day_of_ms);
    // YYYY-MM-DD strings sort lexically == chronologically, so max() = latest.
    let last_o_day: Option<String> = [last_sess_o_day, last_import_o_day]
        .into_iter()
        .flatten()
        .max();
    let days_since_last_cumshot = last_o_day.as_deref().and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .map(|nd| (today_local - nd).num_days().max(0))
    });

    // --- Longest session (raw elapsed, ignoring pauses) ---
    let longest: Option<(i64, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT id, started_at, ended_at, assigned_day FROM sessions
         WHERE status = 'ended' AND ended_at IS NOT NULL
         ORDER BY (ended_at - started_at) DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let (longest_session_ms, longest_session_date, longest_session_id) = match longest {
        Some((id, started, Some(ended), day)) => ((ended - started).max(0), day, Some(id)),
        _ => (0, None, None),
    };

    // --- This-month aggregates (sub-row of hero) ---
    // assigned_day is YYYY-MM-DD; filter by prefix match on YYYY-MM.
    let month_pattern = format!("{}-%", month_prefix);
    // Epoch bounds of the current month for attributing session-less imports.
    let month_first = NaiveDate::from_ymd_opt(today_local.year(), today_local.month(), 1).unwrap();
    let month_last = {
        let (ny, nm) = if today_local.month() == 12 {
            (today_local.year() + 1, 1)
        } else {
            (today_local.year(), today_local.month() + 1)
        };
        NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().unwrap()
    };
    let (month_lo, month_hi) = crate::db::local_day_bounds_ms(
        &month_first.format("%Y-%m-%d").to_string(),
        &month_last.format("%Y-%m-%d").to_string(),
    )?;

    let month_sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions
         WHERE assigned_day LIKE ?1 AND status != 'discarded'",
    )
    .bind(&month_pattern)
    .fetch_one(pool)
    .await?;

    let month_cumshots: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM o_events oe
         LEFT JOIN sessions s ON s.id = oe.session_id
         WHERE (oe.session_id IS NOT NULL AND s.assigned_day LIKE ?1 AND s.status != 'discarded')
            OR (oe.session_id IS NULL AND oe.occurred_at >= ?2 AND oe.occurred_at < ?3)",
    )
    .bind(&month_pattern)
    .bind(month_lo)
    .bind(month_hi)
    .fetch_one(pool)
    .await?;

    let month_active_days: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT assigned_day) FROM sessions
         WHERE assigned_day LIKE ?1 AND status != 'discarded'",
    )
    .bind(&month_pattern)
    .fetch_one(pool)
    .await?;

    // Watch time this month — sum raw elapsed of ended sessions this month.
    // (Same simplification as longest_session: pauses are rare in normal use.)
    let month_watch_time_ms: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(COALESCE(ended_at, ?2) - started_at), 0)
         FROM sessions
         WHERE assigned_day LIKE ?1 AND status != 'discarded'",
    )
    .bind(&month_pattern)
    .bind(crate::db::now_ms())
    .fetch_one(pool)
    .await?;

    Ok(HeroStats {
        cumshots_today,
        record_day_count,
        record_day_date,
        current_streak_days,
        streak_at_risk,
        record_streak_days,
        record_streak_start,
        record_streak_end,
        days_since_last_cumshot,
        longest_session_ms,
        longest_session_date,
        longest_session_id,
        month_watch_time_ms,
        month_cumshots,
        month_sessions,
        month_active_days,
    })
}

/// The longest run of consecutive days in a DESCENDING list of distinct days.
/// Returns `(days, start, end)` with the run's first and last day inclusive, or
/// `(0, None, None)` when there is nothing logged.
///
/// Unanchored from today, unlike `compute_streak` - a record can sit anywhere in
/// history, and if the run still going happens to be the longest, this describes
/// that one with no special case.
///
/// Ties go to the MOST RECENT run. Equal-length runs are equally impressive, and
/// the recent one is the one you would want to look at.
fn longest_streak(descending_days: &[String]) -> (i64, Option<String>, Option<String>) {
    let parsed: Vec<NaiveDate> = descending_days
        .iter()
        .filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .collect();
    if parsed.is_empty() {
        return (0, None, None);
    }

    // Walking newest -> oldest, so within a run the FIRST day seen is its end.
    let (mut best_len, mut best_start, mut best_end) = (0i64, parsed[0], parsed[0]);
    let (mut run_len, mut run_end) = (0i64, parsed[0]);
    let mut previous: Option<NaiveDate> = None;

    for &day in &parsed {
        match previous {
            // The list is DISTINCT, but a duplicate would restart the run and
            // undercount, so skip rather than trust the query.
            Some(prev) if day == prev => continue,
            Some(prev) if prev.pred_opt() == Some(day) => run_len += 1,
            _ => {
                run_len = 1;
                run_end = day;
            }
        }
        // `>` not `>=`: runs are encountered newest first, so the first to reach
        // a given length is the most recent one, and it keeps the tie.
        if run_len > best_len {
            best_len = run_len;
            best_start = day;
            best_end = run_end;
        }
        previous = Some(day);
    }

    (
        best_len,
        Some(best_start.format("%Y-%m-%d").to_string()),
        Some(best_end.format("%Y-%m-%d").to_string()),
    )
}

/// Walk a descending list of distinct assigned_days and count the streak
/// up to today. Returns `(streak_days, at_risk)`. `at_risk` is true when
/// the streak is alive but today hasn't been logged yet.
fn compute_streak(descending_days: &[String], today: NaiveDate) -> (i64, bool) {
    if descending_days.is_empty() {
        return (0, false);
    }

    // Parse to NaiveDate, skip unparseable.
    let parsed: Vec<NaiveDate> = descending_days
        .iter()
        .filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .collect();
    if parsed.is_empty() {
        return (0, false);
    }

    let latest = parsed[0];
    let yesterday = today.pred_opt().unwrap_or(today);

    if latest < yesterday {
        // Streak already broken — last activity older than yesterday.
        return (0, false);
    }

    // If latest is today, streak starts at today (counting today).
    // If latest is yesterday, streak starts at yesterday and is at_risk.
    let mut cursor = latest;
    let mut count: i64 = 0;
    for d in parsed.iter() {
        if *d == cursor {
            count += 1;
            cursor = cursor.pred_opt().unwrap_or(cursor);
        } else if *d < cursor {
            // Gap — streak ends.
            break;
        }
        // *d > cursor means duplicate or out-of-order; skip
    }

    let at_risk = latest == yesterday && latest != today;
    (count, at_risk)
}

/// Last N calendar months including the current one, each with its
/// watch-time / cumshot / session totals. Returns oldest first so the
/// frontend renders left-to-right naturally.
pub async fn monthly_buckets(pool: &SqlitePool, months: u32) -> Result<Vec<MonthlyBucket>> {
    let today = Local::now().date_naive();
    let mut targets: Vec<(String, NaiveDate)> = Vec::with_capacity(months as usize);
    // Build a list of YYYY-MM strings going back `months` months.
    let mut year = today.year();
    let mut month = today.month() as i32;
    for _ in 0..months {
        let label = format!("{:04}-{:02}", year, month);
        let first_of = NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap_or(today);
        targets.push((label, first_of));
        month -= 1;
        if month == 0 {
            month = 12;
            year -= 1;
        }
    }
    targets.reverse(); // oldest first

    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::with_capacity(targets.len());
    for (label, _first_of) in targets {
        let pattern = format!("{}-%", label);

        let watch_time_ms: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(COALESCE(ended_at, ?2) - started_at), 0)
             FROM sessions
             WHERE assigned_day LIKE ?1 AND status != 'discarded'",
        )
        .bind(&pattern)
        .bind(now_ms_val)
        .fetch_one(pool)
        .await?;

        let cumshots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM o_events oe
             JOIN sessions s ON s.id = oe.session_id
             WHERE s.assigned_day LIKE ?1",
        )
        .bind(&pattern)
        .fetch_one(pool)
        .await?;

        let sessions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions
             WHERE assigned_day LIKE ?1 AND status != 'discarded'",
        )
        .bind(&pattern)
        .fetch_one(pool)
        .await?;

        out.push(MonthlyBucket {
            month: label,
            watch_time_ms,
            cumshots,
            sessions,
        });
    }
    Ok(out)
}

/// Every day in the given month, with its watch-time / cumshot / session
/// totals. Days with no activity are still returned with zeros so the
/// chart has a complete X-axis.
pub async fn daily_buckets(pool: &SqlitePool, year: i32, month: u32) -> Result<Vec<DailyBucket>> {
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if NaiveDate::from_ymd_opt(year, 2, 29).is_some() { 29 } else { 28 },
        _ => return Ok(vec![]),
    };

    let now_ms_val = crate::db::now_ms();
    let mut out = Vec::with_capacity(days_in_month);
    for d in 1..=days_in_month {
        let day = format!("{:04}-{:02}-{:02}", year, month, d);

        let watch_time_ms: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(COALESCE(ended_at, ?2) - started_at), 0)
             FROM sessions
             WHERE assigned_day = ?1 AND status != 'discarded'",
        )
        .bind(&day)
        .bind(now_ms_val)
        .fetch_one(pool)
        .await?;

        let cumshots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM o_events oe
             JOIN sessions s ON s.id = oe.session_id
             WHERE s.assigned_day = ?1",
        )
        .bind(&day)
        .fetch_one(pool)
        .await?;

        let sessions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions
             WHERE assigned_day = ?1 AND status != 'discarded'",
        )
        .bind(&day)
        .fetch_one(pool)
        .await?;

        out.push(DailyBucket {
            day,
            watch_time_ms,
            cumshots,
            sessions,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod streak_tests {
    use super::longest_streak;

    /// The function takes days newest-first, the order the query returns.
    fn desc(days: &[&str]) -> Vec<String> {
        days.iter().map(|d| d.to_string()).collect()
    }

    #[test]
    fn finds_the_longest_run_and_its_bounds() {
        // A 4-day run in June, a 2-day run in July.
        let days = desc(&[
            "2026-07-10", "2026-07-09", // 2
            "2026-06-04", "2026-06-03", "2026-06-02", "2026-06-01", // 4
        ]);
        let (len, start, end) = longest_streak(&days);
        assert_eq!(len, 4);
        assert_eq!(start.as_deref(), Some("2026-06-01"));
        assert_eq!(end.as_deref(), Some("2026-06-04"));
    }

    #[test]
    fn a_run_that_is_still_going_is_just_the_longest_run() {
        // The most recent run IS the record - no special case for "current".
        let days = desc(&[
            "2026-07-29", "2026-07-28", "2026-07-27", // still going, 3
            "2026-06-02", "2026-06-01", // 2
        ]);
        let (len, start, end) = longest_streak(&days);
        assert_eq!(len, 3);
        assert_eq!(start.as_deref(), Some("2026-07-27"));
        assert_eq!(end.as_deref(), Some("2026-07-29"));
    }

    #[test]
    fn one_missed_day_breaks_a_run() {
        // 07-27 absent, so this is 2 + 2 rather than one run of 4.
        let days = desc(&["2026-07-29", "2026-07-28", "2026-07-26", "2026-07-25"]);
        assert_eq!(longest_streak(&days).0, 2);
    }

    #[test]
    fn ties_go_to_the_more_recent_run() {
        let days = desc(&[
            "2026-07-11", "2026-07-10", // 2
            "2026-05-02", "2026-05-01", // 2
        ]);
        let (len, start, _) = longest_streak(&days);
        assert_eq!(len, 2);
        assert_eq!(start.as_deref(), Some("2026-07-10"));
    }

    #[test]
    fn runs_span_month_and_year_boundaries() {
        let days = desc(&["2027-01-01", "2026-12-31", "2026-12-30"]);
        let (len, start, end) = longest_streak(&days);
        assert_eq!(len, 3);
        assert_eq!(start.as_deref(), Some("2026-12-30"));
        assert_eq!(end.as_deref(), Some("2027-01-01"));
    }

    #[test]
    fn a_single_day_is_a_run_of_one() {
        let (len, start, end) = longest_streak(&desc(&["2026-07-29"]));
        assert_eq!(len, 1);
        assert_eq!(start.as_deref(), Some("2026-07-29"));
        assert_eq!(end.as_deref(), Some("2026-07-29"));
    }

    #[test]
    fn nothing_logged_yields_no_record() {
        let (len, start, end) = longest_streak(&[]);
        assert_eq!(len, 0);
        assert!(start.is_none() && end.is_none());
    }

    #[test]
    fn a_duplicate_day_does_not_break_a_run() {
        // The query is DISTINCT, but a duplicate must not undercount.
        let days = desc(&["2026-07-29", "2026-07-28", "2026-07-28", "2026-07-27"]);
        assert_eq!(longest_streak(&days).0, 3);
    }
}
