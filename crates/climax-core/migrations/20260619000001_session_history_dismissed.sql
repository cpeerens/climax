-- Phase 7: per-session "Stash logged activity here you didn't track" review.
-- Folds Stash play_history / o_history that falls inside an EXISTING session's
-- window INTO that session (review-first: the user Adds or Skips each item).
--
-- When the user SKIPS a surfaced item it's recorded here so the panel stops
-- offering it. Adding instead resolves naturally - a counted scene_play /
-- adopted o_event no longer qualifies as pending, so no entry is needed.
--   kind='play' -> ref_id = content_items.id (a scene the session didn't track)
--   kind='o'    -> ref_id = o_events.id (a session-less Stash cumshot in window)
CREATE TABLE session_history_dismissed (
    session_id INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('play','o')),
    ref_id INTEGER NOT NULL,
    dismissed_at INTEGER NOT NULL,
    PRIMARY KEY (session_id, kind, ref_id)
);
