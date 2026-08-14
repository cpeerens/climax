-- Sessions get an explicit "assigned_day" so cross-midnight sessions stay
-- attached to the day the USER considers them part of.
--
-- Stored as a YYYY-MM-DD string in the user's local time. The application
-- computes it at session start (defaulting to today's local date) and may
-- override it via the wrap-up modal when the session spans midnight.
--
-- Backfills existing rows by computing the local date of their started_at.
-- SQLite's strftime with the 'localtime' modifier handles the timezone shift.

ALTER TABLE sessions ADD COLUMN assigned_day TEXT;

UPDATE sessions
SET assigned_day = strftime('%Y-%m-%d', started_at / 1000, 'unixepoch', 'localtime')
WHERE assigned_day IS NULL;

CREATE INDEX idx_sessions_assigned_day ON sessions(assigned_day);
