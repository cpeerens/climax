-- Loosen the CHECK constraint on session_pauses.reason to also allow 'video'
-- (auto-pause when Stash video pauses/idles). SQLite doesn't support ALTER on
-- constraints, so we rebuild the table.

PRAGMA foreign_keys = OFF;

CREATE TABLE session_pauses_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    paused_at INTEGER NOT NULL,
    resumed_at INTEGER,
    reason TEXT NOT NULL CHECK (reason IN ('manual','idle','sleep','crash','video'))
);

INSERT INTO session_pauses_new (id, session_id, paused_at, resumed_at, reason)
SELECT id, session_id, paused_at, resumed_at, reason FROM session_pauses;

DROP TABLE session_pauses;
ALTER TABLE session_pauses_new RENAME TO session_pauses;

CREATE INDEX idx_pauses_session ON session_pauses(session_id);

PRAGMA foreign_keys = ON;
