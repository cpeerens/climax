-- Threshold-based play counting (mirrors Stash's "minimum play percent").
--
-- A scene_play row is created on every sighting of a scene during a session,
-- but it should only "count" once the user has actually watched it for at
-- least a configurable fraction of the scene's duration (default = whatever
-- the user's Stash setting is, fetched on first launch). Until counted, the
-- row exists in the DB but is hidden from the tracker / overview / wrap-up
-- (unless a cumshot has been logged on it, which forces it back into view).
--
-- `counted_at` is the timestamp (ms-since-epoch) of the moment seconds_tracked
-- first crossed the threshold. Sticky once set. NULL means below threshold.
ALTER TABLE scene_plays ADD COLUMN counted_at INTEGER;

-- Backfill: every existing scene_play with watched time should be treated
-- as already counted — this feature is forward-looking only, and we don't
-- want to retroactively un-count user history. Anchor counted_at to the
-- first heartbeat we recorded (best available proxy for "when this scene
-- crossed whatever threshold").
UPDATE scene_plays
SET counted_at = first_seen_at
WHERE seconds_tracked > 0
  AND counted_at IS NULL;
