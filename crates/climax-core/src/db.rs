// SQLite setup + connection pool.
//
// Database lives at $APPDATA/Roaming/com.climax.app/climax.sqlite on Windows
// (resolved via Tauri's app_data_dir). On first run we create the file and run all
// migrations under src-tauri/migrations.

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, TimeZone};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub async fn init_pool(db_dir: PathBuf, db_name: &str) -> Result<SqlitePool> {
    std::fs::create_dir_all(&db_dir)
        .with_context(|| format!("create db dir {}", db_dir.display()))?;
    let db_path = db_dir.join(db_name);

    let url = format!("sqlite://{}", db_path.display());
    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .context("connect sqlite pool")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("run migrations")?;

    tracing::info!("database ready at {}", db_path.display());
    Ok(pool)
}

/// Absolute path of the staged-restore file for a given DB filename, inside the
/// DB directory (e.g. "climax.sqlite" -> "climax.sqlite.restore-pending"). The
/// name is derived from the active profile's DB filename so a restore staged in
/// one profile is only ever applied to that same profile's DB.
pub fn restore_pending_path(db_dir: &Path, db_name: &str) -> PathBuf {
    db_dir.join(format!("{db_name}.restore-pending"))
}

/// If a restore was staged before this launch, swap it into place BEFORE the
/// pool opens. Must run with NO open connection, which is why it's called at boot
/// before `init_pool`. Idempotent: a no-op when nothing is staged. Returns
/// whether a restore was applied.
///
/// Safety ordering: the staged file is RE-VALIDATED first (a power-loss/force-kill
/// mid-staging could have left it truncated), then swapped in via rename — which
/// replaces an existing file atomically on the same volume (incl. Windows). The
/// live DB is therefore never deleted before its replacement is in place, so a
/// failed swap can't leave us DB-less. Only after the swap are the stale
/// `-wal`/`-shm` sidecars cleared (else SQLite could replay an old WAL over the
/// restored DB and corrupt it). An unusable staged file is discarded and the
/// current DB kept untouched.
pub async fn apply_pending_restore(db_dir: &Path, db_name: &str) -> Result<bool> {
    let pending = restore_pending_path(db_dir, db_name);
    if !pending.exists() {
        return Ok(false);
    }
    let main = db_dir.join(db_name);
    let wal = db_dir.join(format!("{db_name}-wal"));
    let shm = db_dir.join(format!("{db_name}-shm"));

    // Re-validate BEFORE touching the live DB — a truncated/unusable staged file
    // must not destroy the current database.
    if let Err(e) = validate_db_file(&pending.to_string_lossy()).await {
        tracing::error!("staged restore is unusable, keeping current DB: {:#}", e);
        let _ = std::fs::remove_file(&pending);
        return Ok(false);
    }

    // Clear the OLD main's stale WAL/SHM BEFORE the swap. They belong to the file
    // we're discarding, so removing them first is safe; and because the staged
    // file is still pending until the rename below completes, a crash in this
    // window simply retries the whole swap next boot. This ordering means SQLite
    // can never replay a stale WAL over the freshly restored DB.
    for f in [&wal, &shm] {
        if f.exists() {
            std::fs::remove_file(f)
                .with_context(|| format!("remove {} during restore", f.display()))?;
        }
    }
    // Swap into place: rename replaces an existing target atomically on the same
    // volume (incl. Windows); copy is the cross-volume fallback (won't happen -
    // same dir). The live DB is only ever REPLACED, never deleted-then-recreated,
    // so a failed swap leaves the staged file pending for an automatic retry.
    if std::fs::rename(&pending, &main).is_err() {
        std::fs::copy(&pending, &main)
            .with_context(|| format!("copy staged restore into {}", main.display()))?;
        let _ = std::fs::remove_file(&pending);
    }
    tracing::info!("restored database from staged backup");
    Ok(true)
}

/// Absolute path of the staged full-reset marker for a given DB filename (e.g.
/// "climax.sqlite" -> "climax.sqlite.wipe-pending"). Profile-stemmed like the
/// restore marker, so a reset staged in one profile only ever wipes that
/// profile's DB.
pub fn wipe_pending_path(db_dir: &Path, db_name: &str) -> PathBuf {
    db_dir.join(format!("{db_name}.wipe-pending"))
}

/// Stage a full reset for the next launch: drop any pending restore (a reset
/// overrides it) and write the wipe marker. `apply_pending_wipe` then deletes
/// the DB + sidecars on boot before the pool opens, so `init_pool` recreates a
/// pristine, migrated DB, exactly like a fresh install. The caller restarts.
pub fn stage_wipe(db_dir: &Path, db_name: &str) -> Result<()> {
    let _ = std::fs::remove_file(restore_pending_path(db_dir, db_name));
    let marker = wipe_pending_path(db_dir, db_name);
    std::fs::write(&marker, b"wipe")
        .with_context(|| format!("write wipe marker {}", marker.display()))?;
    Ok(())
}

/// If a full reset was staged before this launch, delete the live DB + its
/// `-wal`/`-shm` sidecars BEFORE the pool opens, so `init_pool` recreates an
/// empty, freshly-migrated database (onboarding re-arms, since the
/// `onboarding_completed` flag went with the DB). Returns true if a wipe ran.
/// Mirrors `apply_pending_restore`'s boot-time, pre-pool ordering.
pub async fn apply_pending_wipe(db_dir: &Path, db_name: &str) -> Result<bool> {
    let marker = wipe_pending_path(db_dir, db_name);
    if !marker.exists() {
        return Ok(false);
    }
    let main = db_dir.join(db_name);
    let wal = db_dir.join(format!("{db_name}-wal"));
    let shm = db_dir.join(format!("{db_name}-shm"));
    for f in [&main, &wal, &shm] {
        if f.exists() {
            std::fs::remove_file(f)
                .with_context(|| format!("remove {} during reset", f.display()))?;
        }
    }
    // Clear the marker last, so a crash mid-delete simply retries the wipe next
    // boot rather than leaving a half-deleted DB the pool would choke on.
    let _ = std::fs::remove_file(&marker);
    tracing::warn!("full reset: wiped database, recreating empty");
    Ok(true)
}

/// Best-effort sweep of leftover web-endpoint temp files in the DB dir:
/// `.{stem}-webrestore-*` (staged uploads) and `.{stem}-webbackup-*` (download
/// snapshots). They're normally removed inline, but a client disconnect
/// mid-upload cancels the handler future before its cleanup runs, and a
/// concurrent restore/reset's exit can strand an in-flight temp - so the
/// standalone server sweeps at boot (the desktop never creates these).
pub fn sweep_web_temps(db_dir: &Path, db_name: &str) {
    let stem = db_name.trim_end_matches(".sqlite");
    let restore_prefix = format!(".{stem}-webrestore-");
    let backup_prefix = format!(".{stem}-webbackup-");
    let Ok(entries) = std::fs::read_dir(db_dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&restore_prefix) || name.starts_with(&backup_prefix) {
            match std::fs::remove_file(e.path()) {
                Ok(()) => tracing::info!("swept stale web temp {}", name),
                Err(err) => tracing::warn!("couldn't sweep stale web temp {}: {}", name, err),
            }
        }
    }
}

/// Validate a candidate database file: it must open, pass a quick integrity
/// check, carry the expected Climax tables, and NOT be from a NEWER Climax build
/// (whose extra `_sqlx_migrations` rows would fail sqlx's applied-migration
/// validation and panic the app at boot). Uses a short-lived `immutable`
/// connection, so it reads WAL-mode files without creating `-wal`/`-shm` sidecars
/// next to the user's chosen file. Shared by the restore command (pre-stage) and
/// the boot-time swap (pre-replace).
pub async fn validate_db_file(path: &str) -> Result<()> {
    use sqlx::Connection;
    if !Path::new(path).exists() {
        anyhow::bail!("That file no longer exists.");
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .immutable(true)
        .create_if_missing(false);
    // These errors render in the restore flow's toast, so each is a plain
    // sentence in the same voice as the bail! messages below; the sqlx detail
    // goes to the log.
    let mut conn = sqlx::SqliteConnection::connect_with(&opts)
        .await
        .map_err(|e| {
            tracing::warn!("validate_db_file: open {} failed: {:#}", path, e);
            anyhow::anyhow!("That file isn't a valid SQLite database.")
        })?;

    let check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut conn)
        .await
        .map_err(|e| {
            tracing::warn!("validate_db_file: quick_check on {} failed: {:#}", path, e);
            anyhow::anyhow!("That file failed an integrity check.")
        })?;
    if check != "ok" {
        let _ = conn.close().await;
        anyhow::bail!("That file failed an integrity check.");
    }

    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('sessions','_sqlx_migrations')",
    )
    .fetch_one(&mut conn)
    .await
    .map_err(|e| {
        tracing::warn!("validate_db_file: inspect schema of {} failed: {:#}", path, e);
        anyhow::anyhow!("Couldn't read that file.")
    })?;
    if tables < 2 {
        let _ = conn.close().await;
        anyhow::bail!("That file isn't a Climax backup.");
    }

    let applied: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations")
            .fetch_all(&mut conn)
            .await
            .map_err(|e| {
                tracing::warn!("validate_db_file: read schema version of {} failed: {:#}", path, e);
                anyhow::anyhow!("Couldn't read that file.")
            })?;
    let _ = conn.close().await;

    // Compare against the embedded migrator (same path token init_pool uses).
    // Two checks: (a) nothing NEWER than this binary ships; (b) every applied
    // migration must EXIST here with a MATCHING checksum - sqlx re-validates
    // both at pool open, and a file that passes quick_check but fails either
    // would otherwise get swapped in and then crash-loop boot until someone
    // deletes the DB by hand (found by the restore-endpoint security review).
    let migrator = sqlx::migrate!("./migrations");
    let max_known = migrator.iter().map(|m| m.version).max().unwrap_or(i64::MAX);
    for (version, checksum) in &applied {
        if *version > max_known {
            anyhow::bail!(
                "This backup is from a newer version of Climax. Update Climax before restoring it."
            );
        }
        let known = migrator.iter().find(|m| m.version == *version);
        let matches = known.map(|m| m.checksum.as_ref() == checksum.as_slice());
        if matches != Some(true) {
            tracing::warn!(
                "validate_db_file: {} has migration {} this build doesn't know (or a checksum mismatch)",
                path, version
            );
            anyhow::bail!("That backup doesn't match this version of Climax, so it can't be restored safely.");
        }
    }
    Ok(())
}

/// Convenience: current unix epoch milliseconds as i64.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Epoch-ms of local midnight at the start of `date`. DST-aware: picks the
/// earliest valid instant, and on the (extremely rare) midnight DST gap nudges
/// to 01:00 so we never return a nonsense value.
pub fn local_midnight_ms(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).unwrap();
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => dt.timestamp_millis(),
        chrono::LocalResult::Ambiguous(dt, _) => dt.timestamp_millis(),
        chrono::LocalResult::None => Local
            .from_local_datetime(&date.and_hms_opt(1, 0, 0).unwrap())
            .earliest()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0),
    }
}

/// Half-open epoch-ms bounds `[lo, hi)` covering every instant whose LOCAL date
/// falls in `[start_day, end_day]` inclusive (YYYY-MM-DD). `hi` is local
/// midnight of the day AFTER `end_day`.
///
/// Phase 7: session-less Stash-history O imports carry no `assigned_day` (that's
/// a session concept), so day-bucketed aggregations scope them by the local
/// date of `occurred_at` instead — these bounds turn a day range into the
/// matching `occurred_at >= lo AND occurred_at < hi` predicate, on the same
/// `chrono::Local` basis the trends hour histogram already uses.
pub fn local_day_bounds_ms(start_day: &str, end_day: &str) -> Result<(i64, i64)> {
    let start = NaiveDate::parse_from_str(start_day, "%Y-%m-%d")?;
    let end = NaiveDate::parse_from_str(end_day, "%Y-%m-%d")?;
    let end_next = end.succ_opt().unwrap_or(end);
    Ok((local_midnight_ms(start), local_midnight_ms(end_next)))
}

/// Local date (YYYY-MM-DD) of an epoch-ms instant, or None if out of range.
/// Used to attribute a session-less import to a day for Rust-side bucketing.
pub fn local_day_of_ms(ms: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
}
