-- Exact watch-time rollback: record how much of Stash's play_duration a session
-- is actually responsible for, instead of guessing from Climax's own sampling.
--
-- THE BUG THIS FIXES. Stash owns `play_duration` - its player accumulates it
-- continuously, and Climax only ever READS it. Deleting a session therefore had
-- to guess the session's share, and it guessed with Climax's own
-- SUM(seconds_tracked), which comes from 5-second bridge heartbeat samples. The
-- two never agree: a scene watched for 277s in Stash was tracked as 269s here,
-- so the rollback subtracted 269 and left 8 seconds stranded on a scene that
-- should have been clean. The error compounds with each separate leg of
-- watching (every sampling gap adds more), so a fragmented session leaves more
-- behind than a single sitting.
--
-- THE FIX. Snapshot Stash's play_duration for every scene when a session starts,
-- snapshot it again when the session stops, and store the real per-scene
-- difference. Two bulk Stash calls per session (the same lean id+play_duration
-- query the off-bridge poller already uses), nothing in the heartbeat path.
--
-- Both columns are NULLABLE ON PURPOSE. Null means "we don't have a trustworthy
-- number" - Stash unreachable at start or stop, a session that predates this
-- migration, or a reconstructed session whose seconds were derived from Stash's
-- figures in the first place. Rollback falls back to the old
-- SUM(seconds_tracked) behaviour in exactly those cases, so this can only ever
-- improve accuracy, never break a delete.

-- Stash's play_duration for every already-watched scene at the moment this
-- session started, as a JSON object: {"<stash scene id>": <seconds>, ...}.
-- Scenes absent from it were unwatched (baseline 0). Written best-effort just
-- after start(); stays NULL if Stash couldn't be reached.
ALTER TABLE sessions ADD COLUMN stash_pd_baseline TEXT;

-- The session's TRUE share of this scene's Stash watch time, in seconds:
-- (play_duration at stop) - (play_duration at start), floored at 0. Written at
-- stop(); NULL until then, and for any scene we couldn't attribute.
ALTER TABLE scene_plays ADD COLUMN stash_watched_secs INTEGER;
