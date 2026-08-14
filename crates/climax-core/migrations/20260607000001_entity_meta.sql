-- Per-entity Stash metadata cache for the browse pages (Phase 5 enrichment).
--
-- Performers / studios / tags are referenced inside content_items.metadata_json
-- only as {id, name}. The browse Scenes grid uses real scene screenshots, but
-- performer / studio / tag cards had no images or demographics. This table
-- caches the richer per-entity metadata fetched from Stash GraphQL
-- (findPerformer / findStudio / findTag), keyed by Stash id, refreshed on the
-- same TTL as scene metadata (metadata_refresh_ttl_seconds).
--
-- image_url is stored NULL when Stash has no real image (its URL carries
-- `default=true`), so the UI cleanly falls back to the initials / glyph card.
-- Fields not applicable to a kind are simply left NULL (e.g. tags only have an
-- image; studios add parent_studio; performers add the demographics).

CREATE TABLE entity_meta (
    kind         TEXT NOT NULL CHECK (kind IN ('performer','studio','tag')),
    external_id  TEXT NOT NULL,
    image_url    TEXT,
    favorite     INTEGER NOT NULL DEFAULT 0,
    ethnicity    TEXT,
    country      TEXT,
    height_cm    INTEGER,
    hair_color   TEXT,
    aliases_json TEXT,
    parent_studio TEXT,
    fetched_at   INTEGER,
    PRIMARY KEY (kind, external_id)
);
