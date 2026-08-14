//! Update check - asks GitHub whether a newer Climax has been released.
//!
//! Climax never updates itself. This module only ever READS a public endpoint
//! and caches what it finds; downloading and installing stays the user's job,
//! which is the same shape Stash uses and the reason no signing keys or updater
//! plugin are involved.
//!
//! Where it runs: the BACKEND, exactly like Stash resolves its `latestversion`
//! query in Go rather than from the browser. One implementation then serves the
//! desktop app in host mode, the desktop app in client mode, and the web UI, and
//! no client ever has to make a cross-origin request.
//!
//! What it does NOT do: decide whether YOU are out of date. It reports the
//! newest release and nothing more, because "current version" depends on which
//! component is asking - in client mode a desktop app and the server it talks to
//! are separately versioned. Each client compares against its own version.
//!
//! Privacy note, since this is the only outbound call Climax makes to anything
//! other than the user's own Stash or Climax server: it is an unauthenticated
//! GET of a public URL, sends no identifiers beyond what any HTTP request
//! carries, and is capped by a TTL so a restart cannot turn into a heartbeat.
//! Stash - same licence, same subject matter, same distribution - does the
//! equivalent check unconditionally at every startup.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::settings::{self, ReleaseInfo, UpdateCheckCache};

/// The PUBLIC release repository. Deliberately not the private dev repo: this
/// is the address a released build ships pointing at, and it is the same one
/// `APP_DOWNLOAD_URL` in the frontend already uses.
const RELEASES_API: &str =
    "https://api.github.com/repos/pineapplestorm/climax/releases/latest";

/// Where the Download button sends people.
pub const RELEASES_PAGE: &str = "https://github.com/pineapplestorm/climax/releases";

/// How stale a cached answer may be before a boot check refreshes it.
const CHECK_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// Matches the Stash GraphQL client's budget. A slow answer here is never worth
/// making anything wait - the caller is always a background task or a button.
const TIMEOUT_SECS: u64 = 6;

/// Refresh the cache if it is missing or older than the TTL. This is the boot
/// path: it is deliberately quiet, because nobody asked for it. Any failure is
/// recorded in the cache for the Settings page to show and logged; it is never
/// surfaced as an error to the caller.
pub async fn check_if_stale(pool: &SqlitePool) {
    let cached = settings::get_update_check(pool).await.unwrap_or_default();
    let fresh = cached
        .checked_at
        .is_some_and(|at| now_ms().saturating_sub(at) < CHECK_TTL_MS);
    if fresh {
        return;
    }
    if let Err(e) = check_now(pool).await {
        // check_now already records a designed sentence in the cache; this is
        // only for the log, and at debug because a boot with no network is an
        // ordinary thing rather than a fault.
        tracing::debug!("update check failed: {:#}", e);
    }
}

/// Fetch, cache and return the newest release. Ignores the TTL - the only
/// callers are the boot path (which has already decided) and the user pressing
/// a button, and a button that might quietly do nothing is a bad button.
///
/// Returns the cache it wrote, so a failed check still yields something to
/// render: the previous result plus the new error.
pub async fn check_now(pool: &SqlitePool) -> Result<UpdateCheckCache> {
    let previous = settings::get_update_check(pool).await.unwrap_or_default();

    let outcome = fetch_latest().await;
    let cache = match outcome {
        Ok(latest) => UpdateCheckCache {
            checked_at: Some(now_ms()),
            latest,
            last_error: None,
        },
        Err(e) => {
            tracing::warn!("update check failed: {:#}", e);
            UpdateCheckCache {
                checked_at: Some(now_ms()),
                // Keep whatever we last knew. A dropped connection is no reason
                // to forget that 0.2.0 exists.
                latest: previous.latest,
                last_error: Some(
                    "Couldn't reach GitHub to check for updates. Try again in a moment."
                        .to_string(),
                ),
            }
        }
    };

    settings::set_update_check(pool, &cache).await?;
    Ok(cache)
}

/// `Ok(None)` means the request succeeded and there is simply no release yet.
///
/// GitHub answers 404 both for "this repository has published no release" and
/// for "no such repository", and the two are indistinguishable from the
/// response. Treating that as "nothing published yet" rather than an error is
/// the honest reading for a project whose public repo appears at the same moment
/// as its first release - before then, every check would otherwise show a
/// failure the user can do nothing about.
async fn fetch_latest() -> Result<Option<ReleaseInfo>> {
    // GitHub REJECTS requests with no User-Agent (403), and reqwest sends none
    // unless told to. Verified against the live API. Do not remove this.
    let client = reqwest::Client::builder()
        .user_agent(concat!("climax/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()?;

    let response = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("GitHub answered {}", response.status());
    }

    let body: serde_json::Value = response.json().await?;
    release_from_json(&body).map(Some)
}

/// Split out from the request so the field mapping can be tested against a real
/// payload - the alternative is finding out it was wrong at the first release.
fn release_from_json(body: &serde_json::Value) -> Result<ReleaseInfo> {
    // Prefer the tag: `name` is free-form and a release can be published with an
    // empty one, whereas the tag is what the release is actually keyed on.
    let tag = body
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if tag.is_empty() {
        anyhow::bail!("GitHub returned a release with no tag");
    }

    let published_at = body
        .get("published_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis());

    let url = body
        .get("html_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(RELEASES_PAGE)
        .to_string();

    Ok(ReleaseInfo {
        version: tag.trim_start_matches('v').to_string(),
        published_at,
        url,
    })
}

/// Is `latest` newer than `current`? Plain numeric comparison of the leading
/// dot-separated numbers, with a prerelease suffix ranking BELOW the release it
/// is a prerelease of ("0.2.0-rc1" is older than "0.2.0").
///
/// Deliberately not Stash's approach. Stash compares git commit hashes and pays
/// for it with a second, paginated API call just to resolve a tag to a SHA - it
/// needs that because it ships continuous develop builds whose version strings
/// aren't ordered. Climax ships tagged releases, so the tags order themselves.
///
/// Unparseable input answers `false`: if we cannot tell, we do not nag.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (l_nums, l_pre) = split_version(latest);
    let (c_nums, c_pre) = split_version(current);
    if l_nums.is_empty() || c_nums.is_empty() {
        return false;
    }

    let width = l_nums.len().max(c_nums.len());
    for i in 0..width {
        // A missing component is 0, so "0.2" and "0.2.0" compare equal.
        let l = l_nums.get(i).copied().unwrap_or(0);
        let c = c_nums.get(i).copied().unwrap_or(0);
        if l != c {
            return l > c;
        }
    }

    // Same numbers: a release beats a prerelease of itself, nothing else moves.
    c_pre && !l_pre
}

/// Splits "v1.2.3-rc1" into ([1, 2, 3], true). Anything after the first
/// non-numeric component is treated as prerelease noise and dropped.
fn split_version(raw: &str) -> (Vec<u64>, bool) {
    let trimmed = raw.trim().trim_start_matches('v');
    let core = trimmed
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let nums: Vec<u64> = core
        .split('.')
        .map_while(|part| part.parse::<u64>().ok())
        .collect();
    (nums, trimmed.len() != core.len())
}

#[cfg(test)]
mod tests {
    use super::{is_newer, release_from_json};

    /// The fields, names and shapes of a real `/releases/latest` response,
    /// trimmed to what we read. Captured from the live API so a rename on
    /// GitHub's side or a typo on ours fails here rather than silently
    /// reporting "no releases" forever.
    const REAL_PAYLOAD: &str = r#"{
        "tag_name": "v0.31.1",
        "name": "v0.31.1",
        "published_at": "2026-04-13T01:48:00Z",
        "html_url": "https://github.com/stashapp/stash/releases/tag/v0.31.1",
        "prerelease": false,
        "draft": false,
        "body": "release notes here"
    }"#;

    #[test]
    fn parses_a_real_release_payload() {
        let body: serde_json::Value = serde_json::from_str(REAL_PAYLOAD).unwrap();
        let release = release_from_json(&body).unwrap();
        // The "v" is stripped so it compares directly against a crate version.
        assert_eq!(release.version, "0.31.1");
        assert_eq!(
            release.url,
            "https://github.com/stashapp/stash/releases/tag/v0.31.1"
        );
        assert_eq!(release.published_at, Some(1_776_044_880_000));
    }

    #[test]
    fn tolerates_a_missing_date_and_url() {
        let body: serde_json::Value = serde_json::from_str(r#"{"tag_name": "0.2.0"}"#).unwrap();
        let release = release_from_json(&body).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.published_at, None);
        // Falls back to the releases page rather than producing a dead link.
        assert_eq!(release.url, super::RELEASES_PAGE);
    }

    #[test]
    fn rejects_a_release_with_no_tag() {
        let body: serde_json::Value = serde_json::from_str(r#"{"name": "untagged"}"#).unwrap();
        assert!(release_from_json(&body).is_err());
    }

    #[test]
    fn plain_ordering() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn double_digit_components_compare_numerically_not_lexically() {
        // The case a string compare gets wrong.
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
    }

    #[test]
    fn v_prefix_and_missing_components() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2"));
    }

    #[test]
    fn prereleases_rank_below_their_release() {
        assert!(is_newer("0.2.0", "0.2.0-rc1"));
        assert!(!is_newer("0.2.0-rc1", "0.2.0"));
        assert!(is_newer("0.2.0-rc1", "0.1.0"));
    }

    #[test]
    fn unparseable_never_nags() {
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("0.2.0", ""));
    }
}
