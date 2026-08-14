-- Mirror sync clock (Phase 7, component 1): when Climax last reconciled this
-- content item against Stash's authoritative history (o_history / play_count /
-- play_duration / last_played_at). Kept SEPARATE from metadata_fetched_at so the
-- Stash-history mirror cadence (settings.mirror_sync) is independent of the
-- descriptive-metadata refresh TTL. NULL = never mirrored, which the
-- reconciliation pass treats as stale (so it gets picked up on the next sweep).
ALTER TABLE content_items ADD COLUMN mirror_synced_at INTEGER;
