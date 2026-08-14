-- One-time cleanup of "phantom" counted plays.
--
-- The play-threshold check (session.rs::mark_counted_if_threshold) used to count
-- a scene IMMEDIATELY when its duration wasn't known yet ("over-count beats
-- losing a watched scene while metadata loads"). But a brand-new scene's first
-- heartbeat fires that check BEFORE the async Stash metadata (duration) lands,
-- so a brief open -- e.g. 7 seconds of a 30-minute scene -- counted as a play.
-- The code is now fixed to NOT count without a known duration and to re-check
-- the moment the duration lands, so no new phantoms are created.
--
-- This heals the ones already in the DB: un-count any play whose watched
-- seconds are below the user's actual play threshold, now that durations are
-- known. The threshold percent is read live from the settings row (Stash's 8%
-- default if the user has never set it), matching effective_play_threshold_frac.
--
-- Safe by construction: seconds_tracked only ever increases, so a legitimately
-- counted play always has seconds >= threshold and can never match here; only
-- genuine false-positives are cleared. Plays whose duration is still unknown
-- are left alone (unmeasurable). A phantom that also has a cumshot stays visible
-- in the session via the `counted_at IS NOT NULL OR cumshot_count > 0` rule, so
-- its session membership is unaffected -- only the spurious threshold mark goes.
UPDATE scene_plays
SET counted_at = NULL
WHERE counted_at IS NOT NULL
  AND content_item_id IN (SELECT id FROM content_items WHERE duration_seconds > 0)
  AND seconds_tracked < (
        (SELECT duration_seconds FROM content_items WHERE id = scene_plays.content_item_id)
        * COALESCE(
            (SELECT json_extract(value_json, '$.threshold_pct') FROM settings WHERE key = 'play_counting'),
            8.0
          ) / 100.0
      );
