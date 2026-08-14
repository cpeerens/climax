// Shared fit-ladder helpers for the browse grid tiles (Scene / Performer /
// Studio / Tag). `runFitLadder` is the shared MACHINERY; each tile still owns
// its own rungs (what to shorten, in what order) and its own measurement, since
// those genuinely differ per tile. See each tile's header comment for its order.

import { tick } from "svelte";

/** One tile's ladder. Everything here is supplied by the tile; this module only
 *  drives the try-full / measure / degrade cycle. */
export type FitLadder = {
  /** Put every label back to its longest form. Runs first, every pass. */
  reset: () => void;
  /** Re-measure: true while the row STILL doesn't fit. A tile whose row has a
   *  shrinkable child must also check that child (it absorbs overflow, so the
   *  row itself never reports any - see StudioTile / SceneTile). */
  squeezed: () => boolean;
  /** The rungs, cheapest loss first. Each shortens exactly one thing. */
  steps: Array<() => void>;
  /** Optional check once the row has settled (the tiles' title check). Runs
   *  even when the row fit immediately, but never on a superseded pass. */
  after?: () => void;
  /** False once a newer pass has started; this one must not write conclusions. */
  current: () => boolean;
};

/** Try the full formats, then degrade only as far as the real rendered widths
 *  require. Callers run this from a ResizeObserver callback (post-layout,
 *  pre-paint); tick() flushes are microtasks, so the cycle never paints an
 *  intermediate frame. */
export async function runFitLadder(l: FitLadder): Promise<void> {
  l.reset();
  await tick();
  if (!l.current()) return;
  for (const step of l.steps) {
    // Stop at the first rung that fits - later rungs cost more information.
    if (!l.squeezed()) break;
    step();
    await tick();
    if (!l.current()) return;
  }
  l.after?.();
}

/** Watch-time label with the tiles' format rules: "-" under a minute (matches
 *  formatWatchShort), "45m" under an hour, "5h 6m" under 100h (or "5h" when
 *  the fit ladder needs the space - floored, never rounded up), and at 100h+
 *  ALWAYS hours-only ("347h") - so the longest possible string is "99h 59m". */
export function watchLabel(ms: number | null, hoursOnly: boolean): string {
  if (!ms || ms < 60_000) return "-";
  const totalMin = Math.floor(ms / 60_000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  if (h >= 100) return `${h}h`;
  if (h === 0) return `${m}m`;
  if (m === 0 || hoursOnly) return `${h}h`;
  return `${h}h ${m}m`;
}

/** A 3+ word name loses its SECOND word (any second word, not just entity
 *  middle names); shorter names come back unchanged. */
export function withoutSecondWord(name: string): string {
  const words = name.trim().split(/\s+/);
  if (words.length < 3) return name;
  return [words[0], ...words.slice(2)].join(" ");
}
