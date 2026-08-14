-- Seed the idle_detection setting used by presence.rs.
--
-- value_json shape: {"enabled": bool, "threshold_minutes": int}
--
-- The existing idle_threshold_seconds row from the initial migration is no
-- longer used (it predates this design); leaving it in place is harmless.

INSERT INTO settings (key, value_json, updated_at)
VALUES ('idle_detection', '{"enabled":true,"threshold_minutes":10}', strftime('%s', 'now') * 1000)
ON CONFLICT(key) DO NOTHING;
