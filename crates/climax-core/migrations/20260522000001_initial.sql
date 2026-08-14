-- Climax initial schema.
--
-- Design notes:
-- - Timestamps are unix epoch milliseconds (i64). UTC. Frontend converts to local.
-- - "Content items" are watched things - Stash scenes, manual entries, anything else.
--   Source taxonomy keeps Stash as one of N possible sources.
-- - Sessions own paused_ranges (zero or more), scene_plays (zero or more),
--   o_events (zero or more), and a free-form notes blob + mood tag join.
-- - All deletes cascade except o_events.session_id (set null) because an O event
--   logged inside a session is still meaningful if the session record is purged.

PRAGMA foreign_keys = ON;

CREATE TABLE sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    config_json TEXT,
    created_at INTEGER NOT NULL
);

INSERT INTO sources (key, name, config_json, created_at) VALUES
    ('stash',  'Stash',         '{"url":"http://localhost:9999"}', strftime('%s', 'now') * 1000),
    ('manual', 'Manual entry',  NULL, strftime('%s', 'now') * 1000),
    ('other',  'Other / unknown', NULL, strftime('%s', 'now') * 1000);

CREATE TABLE content_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    external_id TEXT,
    title TEXT,
    url TEXT,
    thumbnail_url TEXT,
    duration_seconds INTEGER,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    metadata_json TEXT
);
CREATE UNIQUE INDEX idx_content_source_external
    ON content_items(source_id, external_id)
    WHERE external_id IS NOT NULL;
CREATE INDEX idx_content_last_seen ON content_items(last_seen_at);

CREATE TABLE sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    last_heartbeat INTEGER,
    status TEXT NOT NULL CHECK (status IN ('active','paused','ended','discarded')),
    notes TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    session_type TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_started ON sessions(started_at);

CREATE TABLE session_pauses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    paused_at INTEGER NOT NULL,
    resumed_at INTEGER,
    reason TEXT NOT NULL CHECK (reason IN ('manual','idle','sleep','crash'))
);
CREATE INDEX idx_pauses_session ON session_pauses(session_id);

CREATE TABLE mood_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    color TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE session_mood_tags (
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    mood_tag_id INTEGER NOT NULL REFERENCES mood_tags(id) ON DELETE CASCADE,
    PRIMARY KEY (session_id, mood_tag_id)
);

CREATE TABLE scene_plays (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    content_item_id INTEGER NOT NULL REFERENCES content_items(id) ON DELETE CASCADE,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    seconds_tracked INTEGER NOT NULL DEFAULT 0,
    UNIQUE (session_id, content_item_id)
);
CREATE INDEX idx_scene_plays_session ON scene_plays(session_id);

CREATE TABLE o_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    content_item_id INTEGER REFERENCES content_items(id) ON DELETE SET NULL,
    occurred_at INTEGER NOT NULL,
    intensity INTEGER,
    notes TEXT,
    stash_synced INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_o_events_session ON o_events(session_id);
CREATE INDEX idx_o_events_occurred ON o_events(occurred_at);

CREATE TABLE o_event_performers (
    o_event_id INTEGER NOT NULL REFERENCES o_events(id) ON DELETE CASCADE,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    external_id TEXT NOT NULL,
    display_name TEXT,
    PRIMARY KEY (o_event_id, source_id, external_id)
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO settings (key, value_json, updated_at) VALUES
    ('idle_threshold_seconds', '120', strftime('%s', 'now') * 1000),
    ('hotkey_toggle',          '"CommandOrControl+Shift+L"', strftime('%s', 'now') * 1000),
    ('http_port',              '9876', strftime('%s', 'now') * 1000);
