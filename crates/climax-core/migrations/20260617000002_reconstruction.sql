-- Phase 7 component 2: reconstructed (estimated) sessions + dismissed-candidate
-- memory.
--
-- sessions.estimated: 0 = live-tracked (precise), 1 = reconstructed from Stash
-- history (a best-guess - bounds and per-scene watch-minutes are approximate).
-- The Sessions UI tags estimated sessions, so the honesty lives on the data, not
-- just a one-time setup disclaimer.
ALTER TABLE sessions ADD COLUMN estimated INTEGER NOT NULL DEFAULT 0;

-- Candidates the user dismissed in the "while you were away" prompt, so it
-- doesn't re-offer them. Keyed by the underlying import row: kind='play' ->
-- play_imports.id, kind='o' -> o_events.id. The Settings "import older history"
-- scan IGNORES this table (declined != gone - you can still pull them in later).
-- Accepted candidates need no entry: their new session's time window already
-- excludes those events from future scans.
CREATE TABLE reconstruction_dismissed (
    kind TEXT NOT NULL CHECK (kind IN ('play','o')),
    source_id INTEGER NOT NULL,
    dismissed_at INTEGER NOT NULL,
    PRIMARY KEY (kind, source_id)
);
