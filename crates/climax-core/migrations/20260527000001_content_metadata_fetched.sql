-- Track when content_items.metadata_json was last successfully fetched from
-- Stash GraphQL. The refresh-on-stale logic in session.rs::record_heartbeat
-- reads this on every sighting: if NULL or older than the configured TTL
-- (metadata_refresh_ttl_seconds), it kicks off a background re-fetch and
-- updates this timestamp on success.
--
-- Why: the old behaviour was a one-shot enrichment on first insert and never
-- refreshed. Edits made in Stash (a performer removed, studio re-tagged, etc)
-- never propagated back into Climax. Climax now owns the full catalog and
-- refreshes it on its own schedule.
--
-- Backfill: rows that already have metadata_json get set to "now" so they
-- don't all immediately look stale right after the upgrade. Rows with NULL
-- metadata_json keep this NULL too — they'll get fetched on next sighting.

ALTER TABLE content_items ADD COLUMN metadata_fetched_at INTEGER;

UPDATE content_items
SET metadata_fetched_at = strftime('%s', 'now') * 1000
WHERE metadata_json IS NOT NULL
  AND metadata_fetched_at IS NULL;

-- Default refresh TTL: 7 days in seconds. The user can change this from the
-- Settings modal (Stash connection section).
INSERT INTO settings (key, value_json, updated_at)
VALUES (
  'metadata_refresh_ttl_seconds',
  '604800',
  strftime('%s', 'now') * 1000
)
ON CONFLICT(key) DO NOTHING;
