-- Performer gender from Stash (GenderEnum string: FEMALE / MALE /
-- TRANSGENDER_FEMALE / TRANSGENDER_MALE / INTERSEX / NON_BINARY).
-- NULL until (re-)enriched — enrich_entities treats a performer row with
-- NULL gender as stale so existing rows backfill automatically.
ALTER TABLE entity_meta ADD COLUMN gender TEXT;
