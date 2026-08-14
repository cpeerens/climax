-- Distinguish where a cumshot was logged from.
--
-- 'climax' - user clicked + in the Climax UI (this app is the source of truth)
-- 'stash'  - bridge plugin intercepted Stash's own O button (Stash is the source)
-- 'manual' - reserved for future non-Stash content / retroactive logging
--
-- We default existing rows to 'climax' because they pre-date the bridge intercept
-- distinction; in practice the test DB was already wiped earlier so this default
-- mostly applies to fresh installs.

ALTER TABLE o_events ADD COLUMN origin TEXT NOT NULL DEFAULT 'climax';
