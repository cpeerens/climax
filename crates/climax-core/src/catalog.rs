// Catalog browse queries — Phase 5 (Scenes / Performers / Studios / Tags).
//
// Read-only aggregations powering the four dashboard "browse" sections. Every
// stat is either the per-scene Stash snapshot in content_items.metadata_json
// (play count / watch time / last-watched — mirrors Stash) or a Climax-side
// count (cumshots incl. session-less Stash-history imports; sessions +
// cumshots_tracked from scene_plays / o_events). Performer / studio / tag
// facets live inside metadata_json and are exploded with json_each — the same
// pattern dashboard.rs uses for the filter chips.
//
// Discarded sessions are excluded everywhere (status != 'discarded'), matching
// the rest of the app. An entity (scene/performer/studio/tag) only appears once
// it has real activity — watch time OR a cumshot in a non-discarded session.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::dashboard::NamedEntity;
use crate::db::now_ms;
use crate::{settings, stash};

/// One scene row for the Scenes grid. Base content fields + parsed metadata +
/// aggregated activity. `resolution` is populated from Stash enrichment when
/// present (see stash::fetch_scene); older rows omit it gracefully.
#[derive(Debug, Clone, Serialize)]
pub struct BrowseScene {
    pub id: i64,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_seconds: Option<i64>,
    pub resolution: Option<String>,
    pub studio: Option<NamedEntity>,
    pub performers: Vec<NamedEntity>,
    pub tags: Vec<NamedEntity>,
    pub watch_time_ms: i64,
    /// Mirror total: all O's incl. session-less Stash-history imports.
    pub cumshots: i64,
    /// Session-attributed subset of `cumshots` (O's logged inside Climax
    /// sessions) — the detail panel's "N tracked live" readout.
    pub cumshots_tracked: i64,
    pub sessions: i64,
    pub play_count: i64,
    pub last_watched_at: Option<i64>,
}

/// Aggregated performer row + enrichment metadata from `entity_meta` (populated
/// in the background from Stash; the enrichment fields are empty/None until
/// `enrich_entities` has run for them).
#[derive(Debug, Clone, Serialize)]
pub struct BrowsePerformer {
    pub id: String,
    pub name: String,
    pub scene_count: i64,
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub cumshots_tracked: i64,
    pub sessions: i64,
    pub play_count: i64,
    pub last_watched_at: Option<i64>,
    pub image_url: Option<String>,
    pub favorite: bool,
    /// Stash GenderEnum string; None until enriched / unset in Stash.
    pub gender: Option<String>,
    pub ethnicity: Option<String>,
    pub country: Option<String>,
    pub height_cm: Option<i64>,
    pub hair_color: Option<String>,
    pub aliases: Vec<String>,
}

/// Aggregated studio row + enrichment metadata (image, parent studio).
#[derive(Debug, Clone, Serialize)]
pub struct BrowseStudio {
    pub id: String,
    pub name: String,
    pub parent_studio: Option<String>,
    pub scene_count: i64,
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub cumshots_tracked: i64,
    pub sessions: i64,
    pub play_count: i64,
    pub last_watched_at: Option<i64>,
    pub image_url: Option<String>,
}

/// Aggregated tag row + enrichment metadata (image).
#[derive(Debug, Clone, Serialize)]
pub struct BrowseTag {
    pub id: String,
    pub name: String,
    pub scene_count: i64,
    pub watch_time_ms: i64,
    pub cumshots: i64,
    pub cumshots_tracked: i64,
    pub sessions: i64,
    pub play_count: i64,
    pub last_watched_at: Option<i64>,
    pub image_url: Option<String>,
}

// ---------- internal row shapes ----------

#[derive(sqlx::FromRow)]
struct SceneRow {
    id: i64,
    external_id: Option<String>,
    title: Option<String>,
    url: Option<String>,
    thumbnail_url: Option<String>,
    duration_seconds: Option<i64>,
    metadata_json: Option<String>,
    watch_time_ms: i64,
    cumshots: i64,
    cumshots_tracked: i64,
    sessions: i64,
    last_watched_at: Option<i64>,
}

/// Stats for one performer / studio / tag, BEFORE the distinct-session count is
/// merged in (sessions needs its own query — see below).
#[derive(sqlx::FromRow)]
struct EntityStatRow {
    id: String,
    name: String,
    scene_count: i64,
    watch_time_ms: i64,
    cumshots: i64,
    cumshots_tracked: i64,
    play_count: i64,
    last_watched_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct SessionCountRow {
    id: String,
    sessions: i64,
}

/// One `entity_meta` row (background-fetched Stash metadata), merged into the
/// Browse rows by external_id.
#[derive(sqlx::FromRow)]
struct MetaRow {
    external_id: String,
    image_url: Option<String>,
    favorite: i64,
    ethnicity: Option<String>,
    country: Option<String>,
    height_cm: Option<i64>,
    hair_color: Option<String>,
    aliases_json: Option<String>,
    parent_studio: Option<String>,
    gender: Option<String>,
}

async fn load_meta(pool: &SqlitePool, kind: &str) -> Result<HashMap<String, MetaRow>> {
    let rows: Vec<MetaRow> = sqlx::query_as(
        "SELECT external_id, image_url, favorite, ethnicity, country, height_cm,
                hair_color, aliases_json, parent_studio, gender
         FROM entity_meta WHERE kind = ?",
    )
    .bind(kind)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.external_id.clone(), r)).collect())
}

fn parse_aliases(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

/// Per-scene activity rollup. Reused (via json_each) to attribute a scene's
/// totals to each of its performers / studio / tags without the cross-join
/// double-counting a direct scene_plays join would cause. `watch_s` is seconds;
/// callers multiply by 1000 for ms.
// `cumshots` is the MIRROR total — every o_event for the scene, including
// session-less Stash-history imports (Phase 7), so per-entity O-counts equal
// Stash. `cumshots_tracked` is the session-attributed subset (the detail panel's
// "N tracked live" readout). `plays` is the Stash play_count snapshot so
// played-but-not-cum'd scenes still surface in the browse grids.
const SCENE_STATS_CTE: &str = "WITH scene_stats AS (
    SELECT
        ci.id AS cid,
        ci.metadata_json AS meta,
        COALESCE(CAST(json_extract(ci.metadata_json, '$.play_duration_seconds') AS INTEGER), 0) AS watch_s,
        COALESCE((SELECT COUNT(*) FROM o_events oe
                  LEFT JOIN sessions s ON s.id = oe.session_id
                  WHERE oe.content_item_id = ci.id
                    AND (oe.session_id IS NULL OR s.status != 'discarded')), 0) AS cumshots,
        COALESCE((SELECT COUNT(*) FROM o_events oe
                  JOIN sessions s ON s.id = oe.session_id
                  WHERE oe.content_item_id = ci.id AND s.status != 'discarded'), 0) AS cumshots_tracked,
        COALESCE(CAST(json_extract(ci.metadata_json, '$.play_count') AS INTEGER), 0) AS plays,
        CAST(json_extract(ci.metadata_json, '$.last_played_at') AS INTEGER) AS last_watched
    FROM content_items ci
    WHERE ci.metadata_json IS NOT NULL
)";

// ---------- scenes ----------

/// Every scene with watch time or a cumshot in a non-discarded session.
pub async fn list_scenes(pool: &SqlitePool) -> Result<Vec<BrowseScene>> {
    let rows: Vec<SceneRow> = sqlx::query_as(
        "SELECT
            ci.id AS id,
            ci.external_id AS external_id,
            ci.title AS title,
            ci.url AS url,
            ci.thumbnail_url AS thumbnail_url,
            ci.duration_seconds AS duration_seconds,
            ci.metadata_json AS metadata_json,
            COALESCE(CAST(json_extract(ci.metadata_json, '$.play_duration_seconds') AS INTEGER), 0) * 1000 AS watch_time_ms,
            COALESCE((SELECT COUNT(*) FROM o_events oe
                      LEFT JOIN sessions s ON s.id = oe.session_id
                      WHERE oe.content_item_id = ci.id
                        AND (oe.session_id IS NULL OR s.status != 'discarded')), 0) AS cumshots,
            COALESCE((SELECT COUNT(*) FROM o_events oe
                      JOIN sessions s ON s.id = oe.session_id
                      WHERE oe.content_item_id = ci.id AND s.status != 'discarded'), 0) AS cumshots_tracked,
            COALESCE((SELECT COUNT(*) FROM sessions s
                      WHERE s.status != 'discarded'
                        AND (EXISTS (SELECT 1 FROM scene_plays sp
                                     WHERE sp.session_id = s.id AND sp.content_item_id = ci.id)
                          OR EXISTS (SELECT 1 FROM o_events oe
                                     WHERE oe.session_id = s.id AND oe.content_item_id = ci.id))), 0) AS sessions,
            CAST(json_extract(ci.metadata_json, '$.last_played_at') AS INTEGER) AS last_watched_at
         FROM content_items ci
         -- Scenes Stash no longer has (deleted, or merged and untraceable) are
         -- hidden here: with no thumbnail, no title refresh and no live numbers
         -- they are only an eyesore in the grid. The row survives so the sessions
         -- that reference it still show what was watched.
         WHERE ci.stash_missing_at IS NULL
           AND (EXISTS (SELECT 1 FROM scene_plays sp JOIN sessions s ON s.id = sp.session_id
                       WHERE sp.content_item_id = ci.id AND s.status != 'discarded')
            OR EXISTS (SELECT 1 FROM o_events oe LEFT JOIN sessions s ON s.id = oe.session_id
                       WHERE oe.content_item_id = ci.id
                         AND (oe.session_id IS NULL OR s.status != 'discarded'))
            OR COALESCE(CAST(json_extract(ci.metadata_json, '$.play_count') AS INTEGER), 0) > 0
            OR COALESCE(CAST(json_extract(ci.metadata_json, '$.o_counter') AS INTEGER), 0) > 0)
         ORDER BY cumshots DESC, watch_time_ms DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let (performers, studio, tags, resolution, play_count) = parse_meta(r.metadata_json.as_deref());
            BrowseScene {
                id: r.id,
                external_id: r.external_id,
                title: r.title,
                url: r.url,
                thumbnail_url: r.thumbnail_url,
                duration_seconds: r.duration_seconds,
                resolution,
                studio,
                performers,
                tags,
                watch_time_ms: r.watch_time_ms,
                cumshots: r.cumshots,
                cumshots_tracked: r.cumshots_tracked,
                sessions: r.sessions,
                play_count,
                last_watched_at: r.last_watched_at,
            }
        })
        .collect())
}

// ---------- performers / tags (array facets) ----------

pub async fn list_performers(pool: &SqlitePool) -> Result<Vec<BrowsePerformer>> {
    let stats = array_facet_stats(pool, "$.performers").await?;
    let sessions = array_facet_sessions(pool, "$.performers").await?;
    let meta = load_meta(pool, "performer").await?;
    Ok(stats
        .into_iter()
        .map(|r| {
            let m = meta.get(&r.id);
            let aliases = m
                .and_then(|x| x.aliases_json.as_deref())
                .map(parse_aliases)
                .unwrap_or_default();
            BrowsePerformer {
                sessions: sessions.get(&r.id).copied().unwrap_or(0),
                image_url: m.and_then(|x| x.image_url.clone()),
                favorite: m.map(|x| x.favorite != 0).unwrap_or(false),
                gender: m.and_then(|x| x.gender.clone()).filter(|g| !g.is_empty()),
                ethnicity: m.and_then(|x| x.ethnicity.clone()),
                country: m.and_then(|x| x.country.clone()),
                height_cm: m.and_then(|x| x.height_cm),
                hair_color: m.and_then(|x| x.hair_color.clone()),
                aliases,
                id: r.id,
                name: r.name,
                scene_count: r.scene_count,
                watch_time_ms: r.watch_time_ms,
                cumshots: r.cumshots,
                cumshots_tracked: r.cumshots_tracked,
                play_count: r.play_count,
                last_watched_at: r.last_watched_at,
            }
        })
        .collect())
}

pub async fn list_tags(pool: &SqlitePool) -> Result<Vec<BrowseTag>> {
    let stats = array_facet_stats(pool, "$.tags").await?;
    let sessions = array_facet_sessions(pool, "$.tags").await?;
    let meta = load_meta(pool, "tag").await?;
    Ok(stats
        .into_iter()
        .map(|r| BrowseTag {
            sessions: sessions.get(&r.id).copied().unwrap_or(0),
            image_url: meta.get(&r.id).and_then(|m| m.image_url.clone()),
            id: r.id,
            name: r.name,
            scene_count: r.scene_count,
            watch_time_ms: r.watch_time_ms,
            cumshots: r.cumshots,
            cumshots_tracked: r.cumshots_tracked,
            play_count: r.play_count,
            last_watched_at: r.last_watched_at,
        })
        .collect())
}

// ---------- studios (object facet) ----------

pub async fn list_studios(pool: &SqlitePool) -> Result<Vec<BrowseStudio>> {
    let stats: Vec<EntityStatRow> = sqlx::query_as(&format!(
        "{cte}
         SELECT
            CAST(json_extract(ss.meta, '$.studio.id') AS TEXT) AS id,
            CAST(json_extract(ss.meta, '$.studio.name') AS TEXT) AS name,
            COUNT(*) AS scene_count,
            SUM(ss.watch_s) * 1000 AS watch_time_ms,
            SUM(ss.cumshots) AS cumshots,
            SUM(ss.cumshots_tracked) AS cumshots_tracked,
            SUM(ss.plays) AS play_count,
            MAX(ss.last_watched) AS last_watched_at
         FROM scene_stats ss
         WHERE (ss.watch_s > 0 OR ss.cumshots > 0 OR ss.plays > 0)
           AND json_extract(ss.meta, '$.studio.id') IS NOT NULL
         GROUP BY json_extract(ss.meta, '$.studio.id')
         ORDER BY cumshots DESC",
        cte = SCENE_STATS_CTE,
    ))
    .fetch_all(pool)
    .await?;

    // Sessions that contributed EITHER watch time (scene_plays) OR a cumshot
    // (o_events) for a studio's scenes, so the count stays consistent with the
    // cumshots stat (which counts o_events).
    let session_rows: Vec<SessionCountRow> = sqlx::query_as(
        "SELECT u.id, COUNT(DISTINCT u.session_id) AS sessions FROM (
            SELECT CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT) AS id,
                   sp.session_id AS session_id
            FROM content_items ci
              JOIN scene_plays sp ON sp.content_item_id = ci.id
              JOIN sessions s ON s.id = sp.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL
              AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL
            UNION
            SELECT CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT) AS id,
                   oe.session_id AS session_id
            FROM content_items ci
              JOIN o_events oe ON oe.content_item_id = ci.id
              JOIN sessions s ON s.id = oe.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL
              AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL
         ) AS u GROUP BY u.id",
    )
    .fetch_all(pool)
    .await?;
    let sessions: HashMap<String, i64> =
        session_rows.into_iter().map(|r| (r.id, r.sessions)).collect();
    let meta = load_meta(pool, "studio").await?;

    Ok(stats
        .into_iter()
        .map(|r| {
            let m = meta.get(&r.id);
            BrowseStudio {
                sessions: sessions.get(&r.id).copied().unwrap_or(0),
                parent_studio: m.and_then(|x| x.parent_studio.clone()),
                image_url: m.and_then(|x| x.image_url.clone()),
                id: r.id,
                name: r.name,
                scene_count: r.scene_count,
                watch_time_ms: r.watch_time_ms,
                cumshots: r.cumshots,
                cumshots_tracked: r.cumshots_tracked,
                play_count: r.play_count,
                last_watched_at: r.last_watched_at,
            }
        })
        .collect())
}

// ---------- shared array-facet helpers ----------

/// Aggregate per-entity stats over a json array facet (`$.performers` / `$.tags`)
/// via the scene_stats CTE, so a scene's totals attribute once per facet member
/// (no scene_plays cross-join). `path` is a fixed literal — never user input.
async fn array_facet_stats(pool: &SqlitePool, path: &str) -> Result<Vec<EntityStatRow>> {
    let sql = format!(
        "{cte}
         SELECT
            CAST(json_extract(j.value, '$.id') AS TEXT) AS id,
            CAST(json_extract(j.value, '$.name') AS TEXT) AS name,
            COUNT(*) AS scene_count,
            SUM(ss.watch_s) * 1000 AS watch_time_ms,
            SUM(ss.cumshots) AS cumshots,
            SUM(ss.cumshots_tracked) AS cumshots_tracked,
            SUM(ss.plays) AS play_count,
            MAX(ss.last_watched) AS last_watched_at
         FROM scene_stats ss, json_each(ss.meta, '{path}') j
         WHERE (ss.watch_s > 0 OR ss.cumshots > 0 OR ss.plays > 0)
           AND json_extract(j.value, '$.id') IS NOT NULL
         GROUP BY json_extract(j.value, '$.id')
         ORDER BY cumshots DESC",
        cte = SCENE_STATS_CTE,
        path = path,
    );
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Distinct non-discarded sessions in which a scene carrying each facet member
/// either was played (scene_plays) OR had a cumshot logged (o_events) — so the
/// count stays consistent with the cumshots stat.
async fn array_facet_sessions(pool: &SqlitePool, path: &str) -> Result<HashMap<String, i64>> {
    // The derived table is aliased + columns qualified (u.id / u.session_id):
    // an unaliased subquery here triggers SQLite's flattening, which surfaces
    // the inner tables' own `id` columns and errors with "ambiguous column id".
    let sql = format!(
        "SELECT u.id, COUNT(DISTINCT u.session_id) AS sessions FROM (
            SELECT CAST(json_extract(j.value, '$.id') AS TEXT) AS id, sp.session_id AS session_id
            FROM content_items ci
              JOIN json_each(ci.metadata_json, '{path}') j
              JOIN scene_plays sp ON sp.content_item_id = ci.id
              JOIN sessions s ON s.id = sp.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL AND json_extract(j.value, '$.id') IS NOT NULL
            UNION
            SELECT CAST(json_extract(j.value, '$.id') AS TEXT) AS id, oe.session_id AS session_id
            FROM content_items ci
              JOIN json_each(ci.metadata_json, '{path}') j
              JOIN o_events oe ON oe.content_item_id = ci.id
              JOIN sessions s ON s.id = oe.session_id AND s.status != 'discarded'
            WHERE ci.metadata_json IS NOT NULL AND json_extract(j.value, '$.id') IS NOT NULL
         ) AS u GROUP BY u.id",
        path = path,
    );
    let rows: Vec<SessionCountRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| (r.id, r.sessions)).collect())
}

// ---------- background entity enrichment ----------

/// Fetch + cache Stash metadata (image, favorite, demographics, parent studio)
/// for every entity of `kind` referenced in scene metadata that is missing or
/// stale in `entity_meta`. Sequential + best-effort; per-entity failures are
/// logged and skipped. Returns the number enriched. Intended to be spawned in
/// the background — the browse grids poll and pick up results as they land.
pub async fn enrich_entities(pool: &SqlitePool, kind: &str, force: bool) -> Result<usize> {
    let ids: Vec<String> = match kind {
        "performer" => distinct_array_ids(pool, "$.performers").await?,
        "tag" => distinct_array_ids(pool, "$.tags").await?,
        "studio" => distinct_studio_ids(pool).await?,
        _ => return Ok(0),
    };

    let existing: HashMap<String, (Option<i64>, Option<String>)> =
        sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
            "SELECT external_id, fetched_at, gender FROM entity_meta WHERE kind = ?",
        )
        .bind(kind)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, fetched_at, gender)| (id, (fetched_at, gender)))
        .collect();

    let cfg = settings::get_metadata_refresh(pool).await.unwrap_or_default();
    // Auto-refresh off → only never-fetched entities enrich (ttl 0 makes is_stale
    // false for already-fetched ones). `force` (manual sync) overrides entirely.
    let ttl_seconds = if cfg.enabled { cfg.ttl_seconds() } else { 0 };
    let now = now_ms();

    let mut enriched = 0usize;
    for id in ids {
        let (fetched_at, gender) = match existing.get(&id) {
            Some((f, g)) => (*f, g.clone()),
            None => (None, None),
        };
        // Performer rows enriched BEFORE the gender column existed have NULL
        // gender — treat them as stale so gender backfills automatically
        // without waiting out the TTL (or a manual refresh).
        let gender_missing = kind == "performer" && fetched_at.is_some() && gender.is_none();
        if !force && !is_stale(fetched_at, ttl_seconds, now) && !gender_missing {
            continue;
        }
        if let Err(e) = enrich_one(pool, kind, &id, now).await {
            tracing::warn!("enrich {} {} failed: {:#}", kind, id, e);
            continue;
        }
        enriched += 1;
    }
    Ok(enriched)
}

/// Mirror of the staleness rule in session.rs::refresh_stash_metadata: never
/// fetched → stale; ttl<=0 disables auto-refresh; otherwise stale past the TTL.
fn is_stale(fetched_at: Option<i64>, ttl_seconds: i64, now: i64) -> bool {
    match fetched_at {
        None => true,
        Some(_) if ttl_seconds <= 0 => false,
        Some(at) => (now - at) > ttl_seconds * 1000,
    }
}

async fn enrich_one(pool: &SqlitePool, kind: &str, id: &str, now: i64) -> Result<()> {
    // (image_url, favorite, ethnicity, country, height_cm, hair_color, aliases_json, parent_studio, gender)
    let (image_url, favorite, ethnicity, country, height_cm, hair_color, aliases_json, parent_studio, gender) =
        match kind {
            "performer" => {
                let m = stash::fetch_performer(pool, id).await?;
                (
                    m.image_url,
                    m.favorite as i64,
                    m.ethnicity,
                    m.country,
                    m.height_cm,
                    m.hair_color,
                    serde_json::to_string(&m.aliases).ok(),
                    None,
                    // Empty string = "fetched, but unset in Stash" — distinct
                    // from NULL (= never fetched), so the gender-missing
                    // staleness override doesn't re-fetch unset performers
                    // forever.
                    Some(m.gender.unwrap_or_default()),
                )
            }
            "studio" => {
                let m = stash::fetch_studio(pool, id).await?;
                (m.image_url, 0i64, None, None, None, None, None, m.parent_studio, None)
            }
            "tag" => {
                let m = stash::fetch_tag(pool, id).await?;
                (m.image_url, 0i64, None, None, None, None, None, None, None)
            }
            _ => return Ok(()),
        };

    sqlx::query(
        "INSERT INTO entity_meta
            (kind, external_id, image_url, favorite, ethnicity, country, height_cm,
             hair_color, aliases_json, parent_studio, gender, fetched_at)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
         ON CONFLICT(kind, external_id) DO UPDATE SET
            image_url=excluded.image_url, favorite=excluded.favorite,
            ethnicity=excluded.ethnicity, country=excluded.country,
            height_cm=excluded.height_cm, hair_color=excluded.hair_color,
            aliases_json=excluded.aliases_json, parent_studio=excluded.parent_studio,
            gender=excluded.gender, fetched_at=excluded.fetched_at",
    )
    .bind(kind)
    .bind(id)
    .bind(image_url)
    .bind(favorite)
    .bind(ethnicity)
    .bind(country)
    .bind(height_cm)
    .bind(hair_color)
    .bind(aliases_json)
    .bind(parent_studio)
    .bind(gender)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

async fn distinct_array_ids(pool: &SqlitePool, path: &str) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT DISTINCT CAST(json_extract(j.value, '$.id') AS TEXT)
         FROM content_items ci, json_each(ci.metadata_json, '{path}') j
         WHERE ci.metadata_json IS NOT NULL AND json_extract(j.value, '$.id') IS NOT NULL",
        path = path,
    );
    Ok(sqlx::query_scalar(&sql).fetch_all(pool).await?)
}

async fn distinct_studio_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT CAST(json_extract(ci.metadata_json, '$.studio.id') AS TEXT)
         FROM content_items ci
         WHERE ci.metadata_json IS NOT NULL
           AND json_extract(ci.metadata_json, '$.studio.id') IS NOT NULL",
    )
    .fetch_all(pool)
    .await?)
}

// ---------- metadata parsing ----------

/// Pull performers / studio / tags / resolution out of a scene's metadata_json.
/// Returns empty/None for anything missing or unparseable.
fn parse_meta(
    meta: Option<&str>,
) -> (Vec<NamedEntity>, Option<NamedEntity>, Vec<NamedEntity>, Option<String>, i64) {
    let Some(raw) = meta else {
        return (vec![], None, vec![], None, 0);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (vec![], None, vec![], None, 0);
    };
    let performers = named_array(v.get("performers"));
    let tags = named_array(v.get("tags"));
    let studio = v.get("studio").and_then(named_entity);
    let resolution = v
        .get("resolution")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string());
    let play_count = v.get("play_count").and_then(|p| p.as_i64()).unwrap_or(0);
    (performers, studio, tags, resolution, play_count)
}

fn named_array(v: Option<&serde_json::Value>) -> Vec<NamedEntity> {
    v.and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(named_entity).collect())
        .unwrap_or_default()
}

fn named_entity(v: &serde_json::Value) -> Option<NamedEntity> {
    let id = v.get("id")?.as_str()?.to_string();
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    Some(NamedEntity { id, name })
}
