-- Mark scene_plays credited via the OFF-BRIDGE poll path (remote.rs / "Off-bridge
-- tracking") so the wrap-up modal can tag them. Default 0 = tracked through the
-- browser bridge (or seeded by a backdated / reconstructed session). The poll
-- INSERT sets it to 1; any bridge heartbeat for that scene clears it back to 0
-- (the bridge is the authoritative, precise source, so if it ever saw the scene
-- it's not "off-bridge"). Existing rows default 0 - the tag only matters for
-- the live session being wrapped up, whose rows are written with the new logic.
ALTER TABLE scene_plays ADD COLUMN off_bridge INTEGER NOT NULL DEFAULT 0;
