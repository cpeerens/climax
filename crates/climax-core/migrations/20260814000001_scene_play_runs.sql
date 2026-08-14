-- Watch RUNS: the stretches of playback that actually happened inside one
-- scene_play, rather than the single total it keeps.
--
-- scene_plays records how LONG a scene was watched (seconds_tracked) but never
-- WHEN. The session spine therefore had to draw its coral band as one unbroken
-- block anchored at first_seen_at, which is a fiction the moment you pause,
-- rewind, or switch away and come back: the band runs out early and everything
-- positioned by real clock time (cumshot drops, the hour marks) drifts outside
-- it. Measured across a real library, 41 of 48 cumshots fell past the end of
-- their own scene's band, by a median of 4 minutes.
--
-- Each row here is one stretch of playback, written from the same heartbeat
-- that credits seconds_tracked, so the two cannot disagree. Runs also make a
-- rewatch visible: leave a scene and come back and you get a second run, which
-- the spine draws as a second card.
--
-- Rows recorded BEFORE this migration have no runs and never will. Each
-- heartbeat overwrote the last, so the individual timings were never stored -
-- they are gone, not merely absent. Readers MUST fall back to the aggregate
-- (first_seen_at .. last_advance_at) for those, and always will need to:
-- reconstructed and absorbed plays are built from Stash history, which carries
-- no run detail either.

CREATE TABLE scene_play_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- CASCADE matters: deleting a session (or removing one scene from it)
    -- must take that row's evidence with it. Foreign keys are enabled on
    -- every connection (db.rs), so this fires.
    play_id INTEGER NOT NULL REFERENCES scene_plays(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL,
    seconds_tracked INTEGER NOT NULL DEFAULT 0,
    -- 1 when credited by the Stash play_duration poll rather than the bridge.
    -- Coarser by nature: the poll only knows a scene advanced some time in the
    -- last interval, not exactly when.
    off_bridge INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_scene_play_runs_play ON scene_play_runs(play_id, started_at);
