-- Stash play-history mirror (Phase 7, component 2 prerequisite).
--
-- Stores Stash's per-play timestamps (`play_history`) as the play analog of how
-- `o_history` imports into `o_events`. Plays live in their OWN table because
-- `scene_plays` requires a non-NULL session_id (it's Climax's session-bound
-- watch tracking); these are session-less Stash-history imports with no owning
-- session. Session reconstruction (component 2) clusters these timestamps
-- (UNION'd with session-less o_events) to infer past sessions.
--
-- UNIQUE(content_item_id, played_at) makes the mirror merge idempotent: a
-- re-sync inserts nothing, and a timestamp gone from Stash is pruned. Stash
-- floors play_history to whole seconds, so the timestamps are stable across
-- syncs (a sub-second collision on the same scene collapses to one row, an
-- acceptable lossy edge).
CREATE TABLE play_imports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_item_id INTEGER NOT NULL REFERENCES content_items(id) ON DELETE CASCADE,
    played_at INTEGER NOT NULL,   -- unix epoch ms (UTC), from Stash play_history
    created_at INTEGER NOT NULL,
    UNIQUE (content_item_id, played_at)
);
CREATE INDEX idx_play_imports_played ON play_imports(played_at);
CREATE INDEX idx_play_imports_content ON play_imports(content_item_id);
