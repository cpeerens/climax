-- Scenes that Stash no longer has: deleted, or (far more commonly) MERGED into
-- another scene, which deletes the source and keeps no forwarding record.
--
-- Before this, `fetch_scene` could not tell "gone" from "a scene with no
-- activity", so every metadata sync wrote zeros over the stored snapshot and the
-- row sat in the browse grids with a broken thumbnail, no plays and no last-
-- watched date. The row cannot simply be deleted: sessions reference it, and
-- removing it would blank that scene out of the session's own history.
--
-- Set when a sync gets a definitive "no such scene" from Stash, cleared if it
-- ever comes back. Rows carrying it are hidden from the browse grids but still
-- render inside the sessions that contain them.
ALTER TABLE content_items ADD COLUMN stash_missing_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_content_items_stash_missing
  ON content_items(stash_missing_at);
