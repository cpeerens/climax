// Stash GraphQL client.
//
// Climax owns all catalog metadata about Stash scenes (title, thumbnail,
// performers, studio, tags). The bridge plugin only tells us "this scene_id
// is being watched right now"; everything else comes from this module via
// Stash's GraphQL API. The bridge plugin is therefore the only place that
// knows what's playing live; Stash GraphQL is the only place that knows
// what the scenes ARE.
//
// Reads connection info (URL + optional API key) from
// settings::StashConnectionSetting on every call. The user can change either
// from the Settings modal at runtime; no app restart needed.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::settings::get_stash_connection;

/// Master switch for ALL Stash network access, set once at boot from the active
/// profile (see lib.rs). The scratch/test profile sets this false so test
/// sessions can neither push to nor pull from the real Stash library. Every
/// public fn here funnels through `graphql_client` first, so gating that one
/// place blocks the entire client. Default true (the real profile).
static STASH_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable/disable all Stash network access for this process. Called once at boot.
pub fn set_enabled(enabled: bool) {
    STASH_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether Stash access is currently enabled (true unless the scratch profile
/// turned it off at boot).
pub fn is_enabled() -> bool {
    STASH_ENABLED.load(Ordering::Relaxed)
}

/// Compact subset of Stash scene fields Climax cares about.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SceneInfo {
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_seconds: Option<i64>,
    /// Vertical resolution label of the first file (e.g. "2160p"), if known.
    /// Stored in metadata_json for the Scenes grid; None when Stash doesn't
    /// report dimensions or the scene isn't found.
    pub resolution: Option<String>,
    /// Stash's `play_count` for the scene (number of registered plays). Mirrored
    /// into metadata_json for the "Play count" sort. 0 when unknown.
    pub play_count: i64,
    /// Stash's `o_counter` for the scene (Phase 7 mirror cross-check). 0 unknown.
    pub o_counter: i64,
    /// Stash's accumulated `play_duration` in whole seconds. 0 when unknown.
    pub play_duration_seconds: i64,
    /// Stash's `last_played_at` as unix epoch ms (UTC), if ever played.
    pub last_played_at: Option<i64>,
    /// The first file's basename, stored so a scene that later VANISHES from
    /// Stash can still be traced. A merge hands the source's file to the
    /// surviving scene, so the filename is the one thread that survives the
    /// merge - the id is gone and the title can change completely.
    pub file_basename: Option<String>,
    /// Stash's `o_history`: each O's timestamp as unix epoch ms (UTC). Empty when
    /// unknown / never. The mirror reconcile imports these into o_events.
    pub o_history: Vec<i64>,
    /// Stash's `play_history`: each play's timestamp as unix epoch ms (UTC).
    /// Empty when unknown / never. The mirror reconcile imports these into
    /// `play_imports` (the play analog of o_history -> o_events), feeding session
    /// reconstruction. NOT written into metadata_json (it's a table, not a
    /// scalar snapshot).
    pub play_history: Vec<i64>,
    pub performers: Vec<PerformerRef>,
    pub studio: Option<StudioRef>,
    pub date: Option<String>,
    pub tags: Vec<TagRef>,
}

impl SceneInfo {
    /// The metadata_json blob Climax stores on `content_items`: descriptive
    /// facets (performers / studio / date / tags / resolution) PLUS the
    /// Stash-history mirror counts (play_count / o_counter /
    /// play_duration_seconds / last_played_at). Single-sourced here so the live
    /// enrichment path (session.rs) and the mirror reconcile (mirror.rs) always
    /// write an identical shape — catalog.rs reads these keys back.
    pub fn to_metadata_json(&self) -> String {
        serde_json::json!({
            "performers": self.performers,
            "studio": self.studio,
            "date": self.date,
            "tags": self.tags,
            "resolution": self.resolution,
            "play_count": self.play_count,
            "o_counter": self.o_counter,
            "play_duration_seconds": self.play_duration_seconds,
            "last_played_at": self.last_played_at,
            "file_basename": self.file_basename,
        })
        .to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformerRef { pub id: String, pub name: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioRef { pub id: String, pub name: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRef { pub id: String, pub name: String }

/// Per-entity metadata fetched from Stash for the browse pages. `image_url` is
/// None when Stash has no real image (its URL carries `default=true`), so the
/// UI falls back to its placeholder card.
#[derive(Debug, Clone, Default)]
pub struct PerformerMeta {
    pub image_url: Option<String>,
    pub favorite: bool,
    pub ethnicity: Option<String>,
    pub country: Option<String>,
    pub height_cm: Option<i64>,
    pub hair_color: Option<String>,
    pub aliases: Vec<String>,
    /// Stash GenderEnum string (FEMALE / MALE / TRANSGENDER_FEMALE /
    /// TRANSGENDER_MALE / INTERSEX / NON_BINARY). None when unset in Stash.
    pub gender: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StudioMeta {
    pub image_url: Option<String>,
    pub parent_studio: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TagMeta {
    pub image_url: Option<String>,
}

/// Returned by `test_connection`. Surfaces enough info for the Settings UI
/// to confirm the connection worked.
#[derive(Debug, Clone, Serialize)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub version: Option<String>,
    pub scene_count: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct GqlRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct SceneAddOData {
    #[serde(rename = "sceneAddO")]
    scene_add_o: serde_json::Value,
}

// ---------- Error shaping ----------
//
// Errors from the GraphQL call sites below can surface directly in the UI
// (Settings sync status, onboarding, toasts), so the message each returns is a
// short plain-English sentence; the full technical detail is logged here
// instead of being carried up the chain.

/// Map a reqwest transport failure (send / read-body) to a user-readable error.
fn net_err(url: &str, e: reqwest::Error) -> anyhow::Error {
    tracing::warn!("stash request to {} failed: {:#}", url, e);
    if e.is_timeout() {
        anyhow!("Stash took too long to answer. Check that it's running.")
    } else if e.is_body() || e.is_decode() {
        // The request went through; the connection died reading the reply.
        anyhow!("The connection to Stash dropped before it finished answering. Try again.")
    } else {
        anyhow!("Couldn't reach Stash. Check that it's running and the URL is right.")
    }
}

/// Map a non-2xx Stash HTTP response to a user-readable error (body logged).
fn http_err(url: &str, status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    tracing::warn!("stash http {} from {}: {}", status, url, body);
    match status.as_u16() {
        401 | 403 => anyhow!(
            "Stash rejected the request (HTTP {}). Check your API key.",
            status.as_u16()
        ),
        s => anyhow!("Stash returned an error (HTTP {}).", s),
    }
}

/// Map a response that didn't parse as GraphQL JSON to a user-readable error
/// (parse detail + body logged).
fn gql_parse_err(url: &str, e: serde_json::Error, body: &str) -> anyhow::Error {
    tracing::warn!("parse gql response from {} failed: {} - body: {}", url, e, body);
    anyhow!("Stash sent a reply Climax couldn't read. Check that the URL points at Stash.")
}

/// Build a configured reqwest client + the resolved graphql URL for the
/// current stash connection. ApiKey header is set when the user has one
/// configured. 5-second timeout matches the bridge's heartbeat cadence —
/// if Stash takes longer than that to answer we'd rather fail and retry.
async fn graphql_client(
    pool: &SqlitePool,
) -> Result<(reqwest::Client, String)> {
    // Scratch/test profile: hard-off. Every public fn obtains its client here
    // first, so this single gate blocks all reads AND writes. Callers treat the
    // error as best-effort (writes no-op, the local record stands; reads skip),
    // and the Stash-touching background loops aren't even spawned in scratch.
    if !is_enabled() {
        return Err(anyhow!("Stash is turned off in this profile."));
    }
    let conn = get_stash_connection(pool).await?;
    let base = conn.url.trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(anyhow!("Enter your Stash address first."));
    }
    let url = format!("{}/graphql", base);

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5));
    if let Some(key) = &conn.api_key {
        if !key.is_empty() {
            // Stash auth: the `ApiKey` header carries the per-user key from
            // Settings -> Security -> API key in Stash.
            let mut headers = reqwest::header::HeaderMap::new();
            let val = reqwest::header::HeaderValue::from_str(key).map_err(|e| {
                tracing::warn!("stash api key is not header-safe: {:#}", e);
                anyhow!("The API key contains characters that can't be sent. Check it for stray spaces or line breaks.")
            })?;
            headers.insert("ApiKey", val);
            builder = builder.default_headers(headers);
        }
    }
    let client = builder.build().context("build reqwest client")?;
    Ok((client, url))
}

/// Tiny sanity-check GraphQL query: returns Stash's version + total scene
/// count. Used by the Settings modal's "Test connection" button.
pub async fn test_connection(pool: &SqlitePool) -> TestConnectionResult {
    let (client, url) = match graphql_client(pool).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("test_connection setup failed: {:#}", e);
            // Setup errors are designed sentences ("Enter your Stash address
            // first.") except a local settings-read failure, whose top-level
            // message is a dev fragment over raw sqlx - substitute that one.
            let msg = if e.chain().any(|c| c.downcast_ref::<sqlx::Error>().is_some()) {
                "Climax couldn't read its own settings. Try again.".to_string()
            } else {
                format!("{}", e)
            };
            return TestConnectionResult {
                ok: false,
                version: None,
                scene_count: None,
                error: Some(msg),
            };
        }
    };

    let query = r#"
        query ClimaxTestConnection {
            version { version }
            findScenes(filter: { per_page: 1 }) { count }
        }
    "#;
    let body = GqlRequest { query, variables: serde_json::json!({}) };

    #[derive(Deserialize)]
    struct Resp {
        version: Option<RawVersion>,
        #[serde(rename = "findScenes")]
        scenes: Option<RawScenes>,
    }
    #[derive(Deserialize)]
    struct RawVersion { version: Option<String> }
    #[derive(Deserialize)]
    struct RawScenes { count: i64 }

    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("test_connection: POST {} failed: {:#}", url, e);
            let msg = if e.is_timeout() {
                format!("Stash at {} took too long to answer. Check that it's running.", url)
            } else {
                format!("Couldn't reach Stash at {}. Check that it's running and the URL is right.", url)
            };
            return TestConnectionResult {
                ok: false,
                version: None,
                scene_count: None,
                error: Some(msg),
            };
        }
    };
    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("test_connection: read body from {} failed: {:#}", url, e);
            return TestConnectionResult {
                ok: false,
                version: None,
                scene_count: None,
                error: Some("The connection to Stash dropped before it finished answering. Try again.".to_string()),
            };
        }
    };
    if !status.is_success() {
        tracing::warn!("test_connection: http {} from {}: {}", status, url, text);
        let msg = match status.as_u16() {
            401 | 403 => format!("Stash answered with HTTP {}. Check your API key.", status.as_u16()),
            s => format!("Stash answered with HTTP {}. Check that the URL points at Stash.", s),
        };
        return TestConnectionResult {
            ok: false,
            version: None,
            scene_count: None,
            error: Some(msg),
        };
    }
    let parsed: GqlResponse<Resp> = match serde_json::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("test_connection: parse gql response from {} failed: {} - body: {}", url, e, text);
            return TestConnectionResult {
                ok: false,
                version: None,
                scene_count: None,
                error: Some("That address answered, but not like a Stash server. Check the URL.".to_string()),
            };
        }
    };
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return TestConnectionResult {
            ok: false,
            version: None,
            scene_count: None,
            error: Some(format!("Stash returned an error: {}", msg)),
        };
    }
    let data = parsed.data;
    TestConnectionResult {
        ok: true,
        version: data.as_ref().and_then(|d| d.version.as_ref().and_then(|v| v.version.clone())),
        scene_count: data.and_then(|d| d.scenes.map(|s| s.count)),
        error: None,
    }
}

// ---------- Bridge plugin install / detection (onboarding) ----------

/// The Climax bridge's Stash plugin id (matches its YAML `id` and the
/// package_id in the public index). Used to find it in `plugins{}` /
/// `installedPackages` and as the install target.
pub const BRIDGE_PLUGIN_ID: &str = "climax-bridge";

/// The public plugin index the bridge installs from (step 1 of the release
/// arc published this). `installPackages` pulls `climax-bridge` from here.
pub const BRIDGE_SOURCE_URL: &str =
    "https://pineapplestorm.github.io/pineapplestorm-stash-plugins/main/index.yml";

/// What the source is called in Stash's Available Plugins list. Only used when
/// Climax adds the source itself; an existing entry keeps whatever name the
/// user gave it.
pub const BRIDGE_SOURCE_NAME: &str = "Pineapplestorm Plugins";

/// Whether the bridge plugin is present in Stash, and if so whether it's enabled
/// and what version. Returned by the onboarding bridge step + reusable in
/// Settings. `connected` (live WS) is layered on by the command, not here.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BridgePluginInfo {
    pub installed: bool,
    pub enabled: bool,
    pub version: Option<String>,
}

/// Look the bridge up in Stash's loaded plugin list. Uses `plugins{}` (NOT
/// `installedPackages`) on purpose: `plugins{}` lists EVERY loaded plugin
/// including manually-copied folders, whereas `installedPackages` only tracks
/// packages installed through Stash's package manager — so a hand-installed
/// bridge would read as "missing" there.
pub async fn bridge_plugin(pool: &SqlitePool) -> Result<BridgePluginInfo> {
    #[derive(Deserialize)]
    struct Resp {
        plugins: Vec<RawPlugin>,
    }
    #[derive(Deserialize)]
    struct RawPlugin {
        id: String,
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        version: Option<String>,
    }
    let query = r#"query ClimaxBridgePlugin { plugins { id enabled version } }"#;
    let data: Option<Resp> = gql_query(pool, query, serde_json::json!({})).await?;
    let info = data
        .and_then(|d| d.plugins.into_iter().find(|p| p.id == BRIDGE_PLUGIN_ID))
        .map(|p| BridgePluginInfo {
            installed: true,
            enabled: p.enabled,
            version: non_empty(p.version),
        })
        .unwrap_or_default();
    Ok(info)
}

/// Whether the bridge is registered with Stash's PACKAGE MANAGER (the signal
/// that an `installPackages` job has finished writing it to disk). Distinct from
/// `bridge_plugin`, which reads the loaded-plugin list.
async fn bridge_in_installed_packages(pool: &SqlitePool) -> Result<bool> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(rename = "installedPackages")]
        pkgs: Vec<RawPkg>,
    }
    #[derive(Deserialize)]
    struct RawPkg {
        package_id: String,
    }
    let query = r#"query ClimaxInstalledPkgs { installedPackages(type: Plugin) { package_id } }"#;
    let data: Option<Resp> = gql_query(pool, query, serde_json::json!({})).await?;
    Ok(data
        .map(|d| d.pkgs.iter().any(|p| p.package_id == BRIDGE_PLUGIN_ID))
        .unwrap_or(false))
}

/// Register the plugin index in Stash's Available Plugins sources, unless it is
/// already there.
///
/// Installing a package by `sourceURL` alone records that URL against the
/// PACKAGE, and Stash will happily install it, but the source never joins the
/// list Stash checks for newer versions. The bridge then sits frozen at whatever
/// version shipped the day it was installed, and every later update needs a
/// manual reinstall. Adding the source is what puts the bridge in "Installed
/// Plugins -> Check for Updates" like any other plugin.
///
/// `configureGeneral` REPLACES the whole source list, so this is a
/// read-modify-write: every existing source is passed back untouched, including
/// its `local_path`. Send only the new one and the user loses their Community
/// source. (Same trap as `configurePlugin` replacing a plugin's whole settings
/// map.)
///
/// Matched on URL, not name, so a user who already has this index under a name
/// of their own doesn't end up with a duplicate.
async fn ensure_bridge_source(pool: &SqlitePool) -> Result<()> {
    #[derive(Deserialize)]
    struct Resp {
        configuration: Config,
    }
    #[derive(Deserialize)]
    struct Config {
        general: General,
    }
    #[derive(Deserialize)]
    struct General {
        #[serde(rename = "pluginPackageSources", default)]
        sources: Vec<PackageSource>,
    }
    #[derive(Deserialize, Serialize, Clone)]
    struct PackageSource {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        local_path: Option<String>,
    }

    let query = r#"query ClimaxPluginSources {
        configuration { general { pluginPackageSources { name url local_path } } }
    }"#;
    let data: Option<Resp> = gql_query(pool, query, serde_json::json!({})).await?;
    let mut sources = data.map(|d| d.configuration.general.sources).unwrap_or_default();

    if sources
        .iter()
        .any(|s| s.url.trim().eq_ignore_ascii_case(BRIDGE_SOURCE_URL))
    {
        return Ok(());
    }

    sources.push(PackageSource {
        name: Some(BRIDGE_SOURCE_NAME.to_string()),
        url: BRIDGE_SOURCE_URL.to_string(),
        // Omitted on purpose: Stash derives the folder itself.
        local_path: None,
    });

    let mutation = r#"
        mutation ClimaxAddPluginSource($sources: [PackageSourceInput!]) {
            configureGeneral(input: { pluginPackageSources: $sources }) {
                pluginPackageSources { url }
            }
        }
    "#;
    let vars = serde_json::json!({ "sources": sources });
    let _: Option<serde_json::Value> = gql_query(pool, mutation, vars).await?;
    tracing::info!("added plugin source {} to Stash", BRIDGE_SOURCE_URL);
    Ok(())
}

/// Install (or repair) the bridge via Stash's package manager, then load it.
///
/// The source is registered first (see `ensure_bridge_source`) so the bridge
/// updates like any other plugin afterwards.
///
/// `installPackages` returns an `ID!` — the install runs as a background JOB, so
/// the files aren't on disk the instant the mutation returns. We kick it off,
/// poll `installedPackages` until the package registers (job done; ~30s cap),
/// then `reloadPlugins` so Stash loads it into the running plugin set. The user
/// still has to refresh their open Stash tab for the bridge JS + its WebSocket
/// to start — the onboarding copy says so.
pub async fn install_bridge(pool: &SqlitePool) -> Result<()> {
    // Best-effort: a failure here costs update checks, not the install, so it
    // must not block someone getting a working bridge.
    if let Err(e) = ensure_bridge_source(pool).await {
        tracing::warn!("couldn't add the plugin source to Stash: {:#}", e);
    }

    let install = r#"
        mutation ClimaxInstallBridge($type: PackageType!, $packages: [PackageSpecInput!]!) {
            installPackages(type: $type, packages: $packages)
        }
    "#;
    let vars = serde_json::json!({
        "type": "Plugin",
        "packages": [{ "id": BRIDGE_PLUGIN_ID, "sourceURL": BRIDGE_SOURCE_URL }],
    });
    let _: Option<serde_json::Value> = gql_query(pool, install, vars).await?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        if bridge_in_installed_packages(pool).await.unwrap_or(false) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "Stash didn't report the bridge as installed within 30s - the package job may have failed. You can install \"Climax Bridge\" manually from Stash Settings > Plugins."
            ));
        }
    }

    let reload = r#"mutation ClimaxReloadPlugins { reloadPlugins }"#;
    let _: Option<serde_json::Value> = gql_query(pool, reload, serde_json::json!({})).await?;

    // A package-manager install lands enabled by default, but if a prior copy
    // was left disabled, flip it on so the bridge actually runs.
    let _ = enable_bridge(pool).await;
    Ok(())
}

/// Enable the bridge plugin in Stash (`setPluginsEnabled` merges a single key).
/// Best-effort; called after install in case an old copy was disabled.
pub async fn enable_bridge(pool: &SqlitePool) -> Result<()> {
    let mutation = r#"
        mutation ClimaxEnableBridge($map: BoolMap!) {
            setPluginsEnabled(enabledMap: $map)
        }
    "#;
    let vars = serde_json::json!({ "map": { BRIDGE_PLUGIN_ID: true } });
    let _: Option<serde_json::Value> = gql_query(pool, mutation, vars).await?;
    Ok(())
}

/// Trim a string and return None for empty results. Stash GraphQL often
/// returns "" (not null) for unset string fields — without this normalisation,
/// the empty string flows through `COALESCE(?, title)` in the UPDATE and
/// wipes out any existing fallback title in Climax's catalog.
fn non_empty(s: Option<String>) -> Option<String> {
    s.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Stash serves a generated default image (URL carries `default=true`) when an
/// entity has no real image. Treat that as "no image" so the browse UI shows
/// its placeholder (initials / glyph) rather than a generic silhouette.
fn non_default_image(url: Option<String>) -> Option<String> {
    url.and_then(|raw| {
        let t = raw.trim();
        if t.is_empty() || t.contains("default=true") {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Minimal generic GraphQL POST + parse, returning the `data` payload. Shared by
/// the per-entity fetches below; mirrors the inline pattern the other queries
/// use (HTTP error → Err, GraphQL errors → Err, missing data → Ok(None)).
async fn gql_query<T: serde::de::DeserializeOwned>(
    pool: &SqlitePool,
    query: &str,
    variables: serde_json::Value,
) -> Result<Option<T>> {
    let (client, url) = graphql_client(pool).await?;
    let body = GqlRequest { query, variables };
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<T> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(parsed.data)
}

/// Map a video's pixel height to a familiar resolution label. Bands chosen so
/// near-standard heights (e.g. 1078, 2156) still read as their canonical name.
fn res_label(height: i64) -> String {
    match height {
        h if h >= 2160 => "2160p".to_string(),
        h if h >= 1440 => "1440p".to_string(),
        h if h >= 1080 => "1080p".to_string(),
        h if h >= 720 => "720p".to_string(),
        h if h >= 480 => "480p".to_string(),
        h => format!("{}p", h),
    }
}

/// Shared raw GraphQL scene shape, used by both `fetch_scene` (single, by id)
/// and `find_scenes_page` (bulk catalog walk for the mirror import). Carries the
/// descriptive metadata AND the Stash-history mirror fields (o_counter /
/// play_duration / last_played_at / o_history).
#[derive(Deserialize)]
struct RawScene {
    #[serde(default)]
    id: Option<String>,
    title: Option<String>,
    paths: Option<RawPaths>,
    files: Option<Vec<RawFile>>,
    performers: Option<Vec<RawNamedRef>>,
    tags: Option<Vec<RawNamedRef>>,
    studio: Option<RawNamedRef>,
    date: Option<String>,
    play_count: Option<i64>,
    o_counter: Option<i64>,
    play_duration: Option<f64>,
    last_played_at: Option<String>,
    o_history: Option<Vec<String>>,
    play_history: Option<Vec<String>>,
}
#[derive(Deserialize)]
struct RawPaths { screenshot: Option<String>, #[allow(dead_code)] preview: Option<String> }
#[derive(Deserialize)]
struct RawFile {
    duration: Option<f64>,
    basename: Option<String>,
    height: Option<i64>,
}
#[derive(Deserialize)]
struct RawNamedRef { id: String, name: String }

/// The scene selection set Climax requests, shared by the single + bulk queries
/// so they can never drift. Note: substituted into a `format!` template as a
/// runtime value, so its `{ }` are NOT processed by the formatter.
const SCENE_FIELDS: &str = "
    id
    title
    paths { screenshot preview }
    files { duration basename height }
    performers { id name }
    tags { id name }
    studio { id name }
    date
    play_count
    o_counter
    play_duration
    last_played_at
    o_history
    play_history
";

/// A scene that currently owns a given file, for tracing a merged-away scene.
#[derive(Debug, Clone)]
pub struct SceneFileMatch {
    pub id: String,
    pub title: Option<String>,
    pub basenames: Vec<String>,
}

/// Escape a literal string for use inside Stash's `path` regex filter.
///
/// Filenames are full of regex metacharacters - `[UPSCALE]`, `#01`, `.mp4` - and
/// an unescaped `.` or `[` either matches the wrong thing or fails outright.
/// Only real metacharacters are escaped: escaping ordinary characters (a space,
/// say) is itself a syntax error in Go's regexp, which is what Stash uses.
pub fn escape_regex_literal(s: &str) -> String {
    const META: &str = r"\.+*?()|[]{}^$";
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        if META.contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Find the scene(s) whose file path matches `pattern` (a regex).
///
/// This is how a merge is traced. Stash's `sceneMerge` DELETES the source and
/// keeps no forwarding record, but it reassigns the source's FILES to the
/// survivor - so the filename outlives the scene id. Verified against three real
/// merges: in one the survivor's title had changed completely and the file was
/// still the only thing linking them.
///
/// MATCHES_REGEX, deliberately, and this was measured rather than assumed:
/// `EQUALS` wants the whole absolute path (we only keep the basename) and
/// `INCLUDES` TOKENISES - a two-word fragment returned 64 unrelated scenes.
/// Regex was the only modifier that answered with the one right scene.
/// Callers must still confirm the match themselves; see `resolve_merged_scene`.
pub async fn find_scenes_by_path(pool: &SqlitePool, pattern: &str) -> Result<Vec<SceneFileMatch>> {
    let (client, url) = graphql_client(pool).await?;
    let query = "query ClimaxFindByPath($p: String!) {
        findScenes(scene_filter: { path: { value: $p, modifier: MATCHES_REGEX } },
                   filter: { per_page: 25 }) {
            scenes { id title files { basename } }
        }
    }";

    #[derive(Deserialize)]
    struct RawMatch { id: String, title: Option<String>, files: Option<Vec<RawFile>> }
    #[derive(Deserialize)]
    struct RawFound { scenes: Vec<RawMatch> }
    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findScenes")] found: Option<RawFound> }

    let body = GqlRequest { query, variables: serde_json::json!({ "p": pattern }) };
    let resp = client.post(&url).json(&body).send().await.map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<Resp> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(parsed
        .data
        .and_then(|d| d.found)
        .map(|f| f.scenes)
        .unwrap_or_default()
        .into_iter()
        .map(|s| SceneFileMatch {
            id: s.id,
            title: non_empty(s.title),
            basenames: s
                .files
                .unwrap_or_default()
                .into_iter()
                .filter_map(|f| f.basename)
                .collect(),
        })
        .collect())
}

/// Parse an RFC3339 timestamp (Stash returns UTC \"…Z\") to unix epoch ms.
fn rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Map a raw GraphQL scene into Climax's `SceneInfo` (descriptive + mirror
/// fields). Title falls back to the first file's basename when Stash's title is
/// empty (common for untagged collections); None only if both are empty so the
/// caller's `COALESCE(?, title)` preserves whatever was already stored.
fn scene_info_from_raw(scene: RawScene) -> SceneInfo {
    let first_file = scene.files.as_ref().and_then(|fs| fs.first());
    let duration_seconds = first_file.and_then(|f| f.duration).map(|d| d.round() as i64);
    let basename = first_file.and_then(|f| f.basename.clone());
    let resolution = first_file
        .and_then(|f| f.height)
        .filter(|h| *h > 0)
        .map(res_label);
    let title = non_empty(scene.title).or_else(|| non_empty(basename.clone()));

    SceneInfo {
        title,
        file_basename: non_empty(basename),
        thumbnail_url: non_empty(scene.paths.and_then(|p| p.screenshot)),
        duration_seconds,
        resolution,
        play_count: scene.play_count.unwrap_or(0),
        o_counter: scene.o_counter.unwrap_or(0),
        play_duration_seconds: scene.play_duration.map(|d| d.round() as i64).unwrap_or(0),
        last_played_at: scene.last_played_at.and_then(|s| rfc3339_to_ms(&s)),
        o_history: scene
            .o_history
            .unwrap_or_default()
            .iter()
            .filter_map(|s| rfc3339_to_ms(s))
            .collect(),
        play_history: scene
            .play_history
            .unwrap_or_default()
            .iter()
            .filter_map(|s| rfc3339_to_ms(s))
            .collect(),
        performers: scene.performers.unwrap_or_default()
            .into_iter().map(|p| PerformerRef { id: p.id, name: p.name }).collect(),
        studio: scene.studio.map(|s| StudioRef { id: s.id, name: s.name }),
        date: non_empty(scene.date),
        tags: scene.tags.unwrap_or_default()
            .into_iter().map(|t| TagRef { id: t.id, name: t.name }).collect(),
    }
}

/// Fetch a Stash scene's metadata + history mirror fields by id. Returns
/// Ok(SceneInfo::default()) if the scene isn't found in Stash (rare but possible
/// if it was deleted since Climax saw it); errors only on transport / parse /
/// auth issues.
pub async fn fetch_scene(pool: &SqlitePool, scene_id: &str) -> Result<Option<SceneInfo>> {
    let (client, url) = graphql_client(pool).await?;

    let query = format!(
        "query ClimaxFetchScene($id: ID!) {{ findScene(id: $id) {{ {SCENE_FIELDS} }} }}"
    );

    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findScene")] scene: Option<RawScene> }

    let body = GqlRequest {
        query: query.as_str(),
        variables: serde_json::json!({ "id": scene_id }),
    };
    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<Resp> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    // None means Stash genuinely has no such scene - deleted, or MERGED into
    // another one. It used to return SceneInfo::default() here, which callers
    // could not tell apart from a real scene with no activity, so the metadata
    // sync wrote those zeros straight over the stored snapshot on every run.
    // That is why a merged-away scene showed "0 plays / never" instead of what
    // it last knew. Never conflate the two again.
    let Some(scene) = parsed.data.and_then(|d| d.scene) else {
        return Ok(None);
    };
    Ok(Some(scene_info_from_raw(scene)))
}

/// One page of Stash scenes for the mirror sync. Returns this page's scenes as
/// (scene_id, SceneInfo) pairs plus the TOTAL matching count (for progress +
/// paging). Sorted by id ASC so paging is stable across the walk. `page` is
/// 1-based (Stash convention).
///
/// When `active_only` is true, the query is scoped to scenes WITH history
/// (`play_count > 0` OR `o_counter > 0`) — the set Climax mirrors — so a sync
/// discovers everything you've watched/O'd (incl. while Climax was closed) and
/// skips the thousands of never-touched catalog scenes. Verified empirically:
/// Stash's `scene_filter` supports the nested `OR`.
pub async fn find_scenes_page(
    pool: &SqlitePool,
    page: i64,
    per_page: i64,
    active_only: bool,
) -> Result<(Vec<(String, SceneInfo)>, i64)> {
    let (client, url) = graphql_client(pool).await?;

    // Substituted as a runtime value, so its braces aren't seen by format!.
    let scene_filter = if active_only {
        ", scene_filter: { play_count: { value: 0, modifier: GREATER_THAN }, \
         OR: { o_counter: { value: 0, modifier: GREATER_THAN } } }"
    } else {
        ""
    };
    let query = format!(
        "query ClimaxFindScenesPage($page: Int!, $per: Int!) {{
            findScenes(filter: {{ per_page: $per, page: $page, sort: \"id\", direction: ASC }}{scene_filter}) {{
                count
                scenes {{ {SCENE_FIELDS} }}
            }}
        }}"
    );

    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findScenes")] scenes: Option<RawFindScenes> }
    #[derive(Deserialize)]
    struct RawFindScenes { count: i64, scenes: Vec<RawScene> }

    let body = GqlRequest {
        query: query.as_str(),
        variables: serde_json::json!({ "page": page, "per": per_page }),
    };
    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<Resp> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    let Some(fs) = parsed.data.and_then(|d| d.scenes) else {
        return Ok((vec![], 0));
    };
    let total = fs.count;
    let out = fs
        .scenes
        .into_iter()
        .filter_map(|raw| {
            let id = raw.id.clone()?;
            Some((id, scene_info_from_raw(raw)))
        })
        .collect();
    Ok((out, total))
}

/// Lean poll for the live remote-playback detector (remote.rs). Returns every
/// scene with any accumulated watch time as (scene_id, play_duration_seconds).
/// We request ONLY id + play_duration (NOT the full SCENE_FIELDS) so this can
/// run on a tight cadence cheaply. `play_duration` is Stash's cumulative,
/// monotonic per-scene watch time; the poller diffs it between calls to detect
/// scenes being actively watched on ANY device (incl. the Android TV app, which
/// the local bridge can't see). `per_page: -1` returns all matching scenes;
/// `play_duration > 0` skips the never-watched catalog. Sorted by last_played_at
/// DESC purely so the most-recently-active scenes lead the list in logs.
pub async fn find_active_play_durations(pool: &SqlitePool) -> Result<Vec<(String, f64)>> {
    let query = r#"
        query ClimaxRemotePoll {
            findScenes(
                filter: { per_page: -1, sort: "last_played_at", direction: DESC }
                scene_filter: { play_duration: { value: 0, modifier: GREATER_THAN } }
            ) {
                scenes { id play_duration }
            }
        }
    "#;

    #[derive(Deserialize)]
    struct Resp {
        #[serde(rename = "findScenes")]
        scenes: Option<RawFind>,
    }
    #[derive(Deserialize)]
    struct RawFind {
        scenes: Vec<RawRow>,
    }
    #[derive(Deserialize)]
    struct RawRow {
        id: Option<String>,
        play_duration: Option<f64>,
    }

    let data: Option<Resp> = gql_query(pool, query, serde_json::json!({})).await?;
    let Some(fs) = data.and_then(|d| d.scenes) else {
        return Ok(vec![]);
    };
    Ok(fs
        .scenes
        .into_iter()
        .filter_map(|r| Some((r.id?, r.play_duration.unwrap_or(0.0))))
        .collect())
}

/// Fetch `play_history` (epoch-ms per play) for a handful of scenes in ONE
/// request, as (scene_id, timestamps).
///
/// Aliased `findScene(id:)` calls rather than a bulk filter: the input here is a
/// single session's own scenes - typically one to ten - and `findScene(id:)` is
/// the narrowest and most portable shape Stash offers, with the ids passed as
/// variables so nothing is interpolated into the query text. Only
/// `play_history` is requested, so this stays far cheaper than SCENE_FIELDS.
///
/// Used at session end to learn how many separate PLAYS Stash logged for each
/// scene while the session ran. That is what decides whether the session spine
/// draws a scene once or twice: the mirror only refreshes on its own cadence
/// (12 hours by default), so waiting for it would leave a just-finished session
/// showing one card and silently splitting into two the next day.
pub async fn find_play_histories(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<Vec<(String, Vec<i64>)>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let decls = (0..ids.len())
        .map(|i| format!("$id{i}: ID!"))
        .collect::<Vec<_>>()
        .join(", ");
    let body = (0..ids.len())
        .map(|i| format!("s{i}: findScene(id: $id{i}) {{ play_history }}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = format!("query ClimaxPlayHistories({decls}) {{ {body} }}");

    let mut vars = serde_json::Map::new();
    for (i, id) in ids.iter().enumerate() {
        vars.insert(format!("id{i}"), serde_json::Value::String(id.clone()));
    }

    let data: Option<serde_json::Value> =
        gql_query(pool, &query, serde_json::Value::Object(vars)).await?;
    let Some(data) = data else {
        return Ok(vec![]);
    };

    let mut out = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        // A missing alias means Stash no longer has that scene (merged away or
        // deleted). Skipped, not reported as an empty history, so the caller
        // can't mistake "gone" for "never played".
        let Some(node) = data.get(format!("s{i}")).filter(|v| !v.is_null()) else {
            continue;
        };
        let plays = node
            .get("play_history")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(rfc3339_to_ms)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.push((id.clone(), plays));
    }
    Ok(out)
}

/// Fetch a performer's browse metadata (image, favorite, demographics, aliases)
/// from Stash. Returns PerformerMeta::default() if the performer isn't found.
pub async fn fetch_performer(pool: &SqlitePool, id: &str) -> Result<PerformerMeta> {
    let query = r#"
        query ClimaxFetchPerformer($id: ID!) {
            findPerformer(id: $id) {
                image_path
                favorite
                ethnicity
                country
                height_cm
                hair_color
                alias_list
                gender
            }
        }
    "#;
    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findPerformer")] p: Option<RawP> }
    #[derive(Deserialize)]
    struct RawP {
        image_path: Option<String>,
        favorite: Option<bool>,
        ethnicity: Option<String>,
        country: Option<String>,
        height_cm: Option<i64>,
        hair_color: Option<String>,
        alias_list: Option<Vec<String>>,
        gender: Option<String>,
    }
    let resp: Option<Resp> = gql_query(pool, query, serde_json::json!({ "id": id })).await?;
    let Some(p) = resp.and_then(|r| r.p) else {
        return Ok(PerformerMeta::default());
    };
    Ok(PerformerMeta {
        image_url: non_default_image(p.image_path),
        favorite: p.favorite.unwrap_or(false),
        ethnicity: non_empty(p.ethnicity),
        country: non_empty(p.country),
        height_cm: p.height_cm.filter(|h| *h > 0),
        hair_color: non_empty(p.hair_color),
        aliases: p
            .alias_list
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| non_empty(Some(a)))
            .collect(),
        gender: non_empty(p.gender),
    })
}

/// Fetch a studio's browse metadata (image + parent studio name) from Stash.
pub async fn fetch_studio(pool: &SqlitePool, id: &str) -> Result<StudioMeta> {
    let query = r#"
        query ClimaxFetchStudio($id: ID!) {
            findStudio(id: $id) {
                image_path
                parent_studio { name }
            }
        }
    "#;
    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findStudio")] s: Option<RawS> }
    #[derive(Deserialize)]
    struct RawS { image_path: Option<String>, parent_studio: Option<RawParent> }
    #[derive(Deserialize)]
    struct RawParent { name: Option<String> }
    let resp: Option<Resp> = gql_query(pool, query, serde_json::json!({ "id": id })).await?;
    let Some(s) = resp.and_then(|r| r.s) else {
        return Ok(StudioMeta::default());
    };
    Ok(StudioMeta {
        image_url: non_default_image(s.image_path),
        parent_studio: s.parent_studio.and_then(|p| non_empty(p.name)),
    })
}

/// Fetch a tag's browse metadata (image) from Stash.
pub async fn fetch_tag(pool: &SqlitePool, id: &str) -> Result<TagMeta> {
    let query = r#"
        query ClimaxFetchTag($id: ID!) {
            findTag(id: $id) { image_path }
        }
    "#;
    #[derive(Deserialize)]
    struct Resp { #[serde(rename = "findTag")] t: Option<RawT> }
    #[derive(Deserialize)]
    struct RawT { image_path: Option<String> }
    let resp: Option<Resp> = gql_query(pool, query, serde_json::json!({ "id": id })).await?;
    let Some(t) = resp.and_then(|r| r.t) else {
        return Ok(TagMeta::default());
    };
    Ok(TagMeta { image_url: non_default_image(t.image_path) })
}

/// Fetch the user's Stash "minimum play percent" setting — the threshold
/// at which Stash itself bumps `play_count`. Climax mirrors this for its
/// own scene-play counting so the two stay in lockstep.
///
/// Stash stores this under `configuration.ui` (a free-form Map scalar, NOT
/// on the typed ConfigInterfaceResult — confirmed by introspecting a live
/// Stash instance). We fetch the whole `ui` blob, then pull
/// `minimumPlayPercent` out of it.
///
/// Returns Some(pct) on success; None if the field isn't present (history
/// off, or older Stash builds that don't expose this key). Network / auth
/// errors propagate as Err.
pub async fn fetch_minimum_play_percent(pool: &SqlitePool) -> Result<Option<f32>> {
    let (client, url) = graphql_client(pool).await?;

    let query = r#"
        query ClimaxFetchPlayThreshold {
            configuration { ui }
        }
    "#;

    #[derive(Deserialize)]
    struct Resp { configuration: Option<RawConfig> }
    #[derive(Deserialize)]
    struct RawConfig { ui: Option<serde_json::Value> }

    let body = GqlRequest { query, variables: serde_json::json!({}) };
    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<Resp> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(parsed.data
        .and_then(|d| d.configuration)
        .and_then(|c| c.ui)
        .and_then(|ui| ui.get("minimumPlayPercent").cloned())
        .and_then(|v| v.as_f64().map(|f| f as f32)))
}

/// Sync Stash's `minimumPlayPercent` into Climax's play-counting threshold, but
/// ONLY if the user has never configured one (`threshold_pct` is `None`) - never
/// clobber a value the user set by hand. Best-effort: `Ok(Some(pct))` = synced,
/// `Ok(None)` = skipped (already set, or Stash returned no value).
///
/// Called at boot AND whenever a Stash connection is (re)configured. The boot
/// sync alone isn't enough: a fresh / headless server (Docker, NAS) has NO Stash
/// reachable at boot - it's connected later, during onboarding - so without the
/// on-connect call the threshold would sit at the 0% fallback until the user
/// synced it by hand. Living in core, it's the single implementation both the
/// desktop `stash_connection_set` command and the `/rpc` arm call, so every
/// edition (Windows host, desktop-as-client, headless server) behaves identically.
pub async fn sync_play_threshold_if_unset(pool: &SqlitePool) -> Result<Option<f32>> {
    let current = crate::settings::get_play_counting(pool).await?;
    if current.threshold_pct.is_some() {
        return Ok(None); // user already has a value - don't overwrite it
    }
    match fetch_minimum_play_percent(pool).await? {
        Some(pct) => {
            crate::settings::set_play_counting(
                pool,
                &crate::settings::PlayCountingSetting { threshold_pct: Some(pct) },
            )
            .await?;
            Ok(Some(pct))
        }
        None => Ok(None),
    }
}

/// Read Stash's "Enable scene play history" toggle (Settings -> Interface ->
/// Scene Player). Stored as `trackActivity` in the free-form `configuration.ui`
/// map (the same place as `minimumPlayPercent`; confirmed by introspecting a
/// live Stash). Returns Some(bool) when present, None when the key is absent
/// (older builds). Network / auth errors propagate.
pub async fn get_track_activity(pool: &SqlitePool) -> Result<Option<bool>> {
    let query = r#"
        query ClimaxGetTrackActivity {
            configuration { ui }
        }
    "#;
    #[derive(Deserialize)]
    struct Resp { configuration: Option<RawConfig> }
    #[derive(Deserialize)]
    struct RawConfig { ui: Option<serde_json::Value> }
    let data: Option<Resp> = gql_query(pool, query, serde_json::json!({})).await?;
    Ok(data
        .and_then(|d| d.configuration)
        .and_then(|c| c.ui)
        .and_then(|ui| ui.get("trackActivity").and_then(|v| v.as_bool())))
}

/// Set Stash's "Enable scene play history" (`trackActivity`) on or off.
///
/// Uses `configureUISetting(key, value)`, which merges a SINGLE key into the UI
/// config without clobbering the rest of it (verified against live Stash — the
/// mutation returns the whole merged map, which we discard). Best-effort at the
/// call sites: the organising-mode toggle + the session-start / launch
/// self-heal. A no-op (Err) in the scratch profile, where Stash is disabled.
pub async fn set_track_activity(pool: &SqlitePool, enabled: bool) -> Result<()> {
    let query = r#"
        mutation ClimaxSetTrackActivity($key: String!, $value: Any) {
            configureUISetting(key: $key, value: $value)
        }
    "#;
    // The mutation returns the entire (large) UI map; we don't need it, so parse
    // it into a throwaway Value just to surface any GraphQL errors.
    let _: Option<serde_json::Value> = gql_query(
        pool,
        query,
        serde_json::json!({ "key": "trackActivity", "value": enabled }),
    )
    .await?;
    Ok(())
}

/// Ensure Stash's play-history tracking is ON, flipping it back if a previous
/// organising session (or an unclean exit during one) left it off. Reads first
/// so the common already-on case writes nothing. Returns true if it had to heal
/// the setting. The invariant Climax enforces: it must never sit in 'no session'
/// or 'tracking' with this off — only organising mode allows off.
pub async fn ensure_track_activity_on(pool: &SqlitePool) -> Result<bool> {
    match get_track_activity(pool).await? {
        Some(false) => {
            set_track_activity(pool, true).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Decrement Stash's o_counter for a scene by one (removes the most recent
/// O entry from its history). Used when the user deletes a cumshot in Climax
/// that was previously synced to Stash.
///
/// Best-effort: failures are logged but not surfaced; the local delete
/// happens regardless so Climax + Stash can be reconciled later if needed.
pub async fn remove_o(pool: &SqlitePool, scene_id: &str) -> Result<i64> {
    let (client, url) = graphql_client(pool).await?;

    // Stash's actual signature: sceneDecrementO(id: ID!) -> Int!
    // (Decrements counter, removes most recent entry from o_history.)
    let query = r#"
        mutation ClimaxSceneDecrementO($id: ID!) {
            sceneDecrementO(id: $id)
        }
    "#;

    let body = GqlRequest {
        query,
        variables: serde_json::json!({ "id": scene_id }),
    };

    let resp = client.post(&url).json(&body).send().await
        .map_err(|e| net_err(&url, e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }

    #[derive(Deserialize)]
    struct DecOData { #[serde(rename = "sceneDecrementO")] dec: i64 }
    let parsed: GqlResponse<DecOData> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    let data = parsed.data.ok_or_else(|| anyhow!("no data in gql response"))?;
    Ok(data.dec)
}

/// Remove the SINGLE O-history entry that corresponds to a specific cumshot,
/// identified by its `occurred_at` time, rather than always popping the most
/// recent (LIFO). Reads o_history, deletes the entry nearest `occurred_at_ms`
/// within a tolerance; if none is close (e.g. a stash-origin O whose stored
/// time drifted a little), falls back to the most-recent entry so the counter
/// still decrements. Best-effort. Returns true if a delete was issued.
pub async fn remove_o_at(pool: &SqlitePool, scene_id: &str, occurred_at_ms: i64) -> Result<bool> {
    // A cumshot logged in Climax is pushed with its exact occurred_at, so the
    // o_history entry matches exactly; the tolerance only matters for O's that
    // originated in Stash and were mirrored in with slight clock skew.
    const TOL_MS: i64 = 120_000;
    let (client, url) = graphql_client(pool).await?;

    // Read the scene's O history.
    let read_query = r#"
        query ClimaxReadOHistory($id: ID!) {
            findScene(id: $id) { o_history }
        }
    "#;
    let read_body = GqlRequest {
        query: read_query,
        variables: serde_json::json!({ "id": scene_id }),
    };
    let resp = client.post(&url).json(&read_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    let history: Vec<String> = parsed.data
        .as_ref()
        .and_then(|d| d.get("findScene"))
        .and_then(|s| s.get("o_history"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    if history.is_empty() {
        return Ok(false);
    }

    // Pick the entry nearest occurred_at; track the newest as a fallback.
    let mut nearest: Option<(i64, String)> = None; // (abs diff, ts)
    let mut newest: Option<(i64, String)> = None; // (ms, ts)
    for ts in &history {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
            let ms = dt.timestamp_millis();
            let diff = (ms - occurred_at_ms).abs();
            if nearest.as_ref().is_none_or(|(d, _)| diff < *d) {
                nearest = Some((diff, ts.clone()));
            }
            if newest.as_ref().is_none_or(|(n, _)| ms > *n) {
                newest = Some((ms, ts.clone()));
            }
        }
    }
    let chosen = match nearest {
        Some((diff, ts)) if diff <= TOL_MS => ts,
        _ => match newest {
            Some((_, ts)) => ts,
            None => return Ok(false),
        },
    };

    // Delete that exact timestamp.
    let del_query = r#"
        mutation ClimaxSceneDeleteO($id: ID!, $times: [Timestamp!]) {
            sceneDeleteO(id: $id, times: $times) { count }
        }
    "#;
    let del_body = GqlRequest {
        query: del_query,
        variables: serde_json::json!({ "id": scene_id, "times": [chosen] }),
    };
    let resp = client.post(&url).json(&del_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(true)
}

/// Remove the play-history entries for a scene whose timestamps fall within a
/// session's time window — the plays THAT session contributed — rather than
/// just popping the most recent one (LIFO). Returns how many were removed.
/// Best-effort: callers run this in the background and only log failures.
pub async fn remove_plays_in_window(
    pool: &SqlitePool,
    scene_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<usize> {
    delete_history_in_window(pool, scene_id, "play_history", "sceneDeletePlay", start_ms, end_ms).await
}

/// Remove the O-history entries for a scene whose timestamps fall within a
/// session's time window — the cumshots THAT session contributed — rather than
/// the most recent one. Returns how many were removed.
pub async fn remove_os_in_window(
    pool: &SqlitePool,
    scene_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<usize> {
    delete_history_in_window(pool, scene_id, "o_history", "sceneDeleteO", start_ms, end_ms).await
}

/// Shared implementation for the windowed history deletions above. Reads the
/// scene's history field (`o_history` | `play_history`) — a list of RFC3339
/// timestamps — keeps the ones inside [start_ms, end_ms] (padded slightly to
/// absorb Stash's second-precision truncation + minor clock skew), and deletes
/// exactly those via the given removal mutation (`sceneDeleteO` |
/// `sceneDeletePlay`), passing the original timestamp strings so Stash matches
/// the precise entries. This makes a session/scene delete remove the specific
/// plays/O's it logged, not whatever happens to be newest.
async fn delete_history_in_window(
    pool: &SqlitePool,
    scene_id: &str,
    field: &str,
    mutation: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<usize> {
    // Pad the window so a play/O recorded at the very edge isn't missed when
    // Stash floors the timestamp to whole seconds. 1.5s is far smaller than the
    // gap between real sessions, so it won't catch a neighbour's entries.
    const PAD_MS: i64 = 1500;
    let (client, url) = graphql_client(pool).await?;

    // 1. Read the history list.
    let read_query =
        format!("query ClimaxReadHistory($id: ID!) {{ findScene(id: $id) {{ {field} }} }}");
    let read_body = GqlRequest {
        query: read_query.as_str(),
        variables: serde_json::json!({ "id": scene_id }),
    };
    let resp = client.post(&url).json(&read_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    let history = parsed.data
        .as_ref()
        .and_then(|d| d.get("findScene"))
        .and_then(|s| s.get(field))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();

    let lo = start_ms - PAD_MS;
    let hi = end_ms + PAD_MS;
    let in_window: Vec<String> = history
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .filter(|ts| {
            chrono::DateTime::parse_from_rfc3339(ts)
                .map(|d| {
                    let ms = d.timestamp_millis();
                    ms >= lo && ms <= hi
                })
                .unwrap_or(false)
        })
        .collect();

    if in_window.is_empty() {
        return Ok(0);
    }

    // 2. Delete exactly those timestamps.
    let del_query = format!(
        "mutation ClimaxDeleteHistory($id: ID!, $times: [Timestamp!]) {{ {mutation}(id: $id, times: $times) {{ count }} }}"
    );
    let del_body = GqlRequest {
        query: del_query.as_str(),
        variables: serde_json::json!({ "id": scene_id, "times": in_window }),
    };
    let resp = client.post(&url).json(&del_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(in_window.len())
}

/// Reduce a Stash scene's accumulated watch time (`play_duration`) by
/// `subtract_secs` seconds, clamped at zero. Used to roll back the watch time a
/// deleted Climax session/scene contributed to Stash.
///
/// Stash has no "subtract" verb (`sceneSaveActivity` only ADDS), but
/// `play_duration` is a plain Float settable via `sceneUpdate`. So this is a
/// read-modify-write: read the current value, write `max(0, current - delta)`.
/// Single-user + best-effort, so the read/write race is immaterial.
pub async fn reduce_play_duration(
    pool: &SqlitePool,
    scene_id: &str,
    subtract_secs: i64,
) -> Result<f64> {
    let (client, url) = graphql_client(pool).await?;

    // Read current play_duration.
    let read_query = r#"
        query ClimaxReadPlayDuration($id: ID!) {
            findScene(id: $id) { play_duration }
        }
    "#;
    let read_body = GqlRequest {
        query: read_query,
        variables: serde_json::json!({ "id": scene_id }),
    };
    let resp = client.post(&url).json(&read_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }

    #[derive(Deserialize)]
    struct ReadResp {
        #[serde(rename = "findScene")]
        scene: Option<ReadScene>,
    }
    #[derive(Deserialize)]
    struct ReadScene { play_duration: Option<f64> }

    let parsed: GqlResponse<ReadResp> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    let current = parsed.data
        .and_then(|d| d.scene)
        .and_then(|s| s.play_duration)
        .ok_or_else(|| anyhow!("scene {} has no play_duration (not found?)", scene_id))?;

    let new_duration = (current - subtract_secs as f64).max(0.0);

    // Write the reduced value.
    let write_query = r#"
        mutation ClimaxSetPlayDuration($id: ID!, $pd: Float!) {
            sceneUpdate(input: { id: $id, play_duration: $pd }) { id }
        }
    "#;
    let write_body = GqlRequest {
        query: write_query,
        variables: serde_json::json!({ "id": scene_id, "pd": new_duration }),
    };
    let resp = client.post(&url).json(&write_body).send().await
        .map_err(|e| net_err(&url, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }
    let parsed: GqlResponse<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;
    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }
    Ok(new_duration)
}

/// Add an O event to a Stash scene by ID. `at_ms` is unix epoch milliseconds.
/// Returns the new o_counter value on success.
pub async fn add_o(pool: &SqlitePool, scene_id: &str, at_ms: i64) -> Result<i64> {
    let (client, url) = graphql_client(pool).await?;

    // Convert ms -> ISO-8601 RFC3339 (Stash accepts this for Timestamp scalars).
    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms)
        .ok_or_else(|| anyhow!("bad timestamp"))?
        .to_rfc3339();

    // Stash's actual signature: sceneAddO(id: ID!, times: [Timestamp!])
    // (Singular id, NOT input-wrapped. Verified empirically.)
    let query = r#"
        mutation ClimaxSceneAddO($id: ID!, $times: [Timestamp!]) {
            sceneAddO(id: $id, times: $times) {
                count
                history
            }
        }
    "#;

    let body = GqlRequest {
        query,
        variables: serde_json::json!({ "id": scene_id, "times": [ts] }),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| net_err(&url, e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| net_err(&url, e))?;
    if !status.is_success() {
        return Err(http_err(&url, status, &text));
    }

    let parsed: GqlResponse<SceneAddOData> = serde_json::from_str(&text)
        .map_err(|e| gql_parse_err(&url, e, &text))?;

    if let Some(errs) = parsed.errors {
        let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
        return Err(anyhow!("Stash returned an error: {}", msg));
    }

    let data = parsed.data.ok_or_else(|| anyhow!("no data in gql response"))?;
    let count = data
        .scene_add_o
        .get("count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Ok(count)
}

#[cfg(test)]
mod regex_escape_tests {
    use super::escape_regex_literal;

    /// Real filenames from a live library, which is where this gets exercised.
    #[test]
    fn escapes_the_metacharacters_filenames_actually_contain() {
        let name = "American Anal Sluts, Scene #01 - Jennifer White [UPSCALE].mp4";
        let escaped = escape_regex_literal(name);
        assert!(escaped.contains(r"\[UPSCALE\]"), "brackets must be escaped: {escaped}");
        assert!(escaped.contains(r"\.mp4"), "the dot must be escaped: {escaped}");
        // '#', ',' and spaces are NOT regex metacharacters, and escaping a space
        // is a syntax error in Go's regexp - which is what Stash runs.
        assert!(!escaped.contains(r"\ "), "must not escape spaces: {escaped}");
        assert!(!escaped.contains(r"\#"), "must not escape hashes: {escaped}");
    }

    #[test]
    fn leaves_a_plain_name_untouched() {
        assert_eq!(escape_regex_literal("Plain Name 01"), "Plain Name 01");
    }

    #[test]
    fn escapes_a_literal_backslash() {
        // Built from chars so the assertion can't be misread through two layers
        // of string escaping: one backslash in, two out.
        let input: String = ['a', '\\', 'b'].iter().collect();
        let out = escape_regex_literal(&input);
        assert_eq!(out.chars().filter(|c| *c == '\\').count(), 2, "got {out}");
    }
}
