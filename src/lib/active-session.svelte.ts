// Single source of truth for "is a session running right now?".
//
// Previously the dashboard hit the backend's `session_active` from TWO
// independent timers — the chassis (sidebar status dot) and the Overview
// section (active-session card / live timer) — each on its own interval.
// This store polls `session_active` ONCE and shares the result, so there's
// one backend call and one source of truth.
//
// Usage in a component:
//   import { activeSession } from "$lib/active-session.svelte";
//   let unsub: (() => void) | null = null;
//   onMount(() => { unsub = activeSession.subscribe(); });
//   onDestroy(() => { unsub?.(); });
//   ... then read `activeSession.current` / `activeSession.lastRefreshAt`
//       reactively in markup or $derived ...
//
// `lastRefreshAt` is exposed so a consumer can render a smooth live timer
// (effective_duration_ms + (now − lastRefreshAt)) between polls.

import { api, type Session } from "$lib/api";

let current = $state<Session | null>(null);
let lastRefreshAt = $state(Date.now());

// Ref-counted poll: the interval only runs while at least one component is
// subscribed, and stops when the last one unsubscribes.
let subscribers = 0;
let timer: ReturnType<typeof setInterval> | null = null;
const POLL_MS = 4000;

async function refresh(): Promise<void> {
  try {
    current = await api.sessionActive();
    lastRefreshAt = Date.now();
  } catch (e) {
    console.error("active-session poll failed", e);
  }
}

export const activeSession = {
  get current(): Session | null {
    return current;
  },
  get lastRefreshAt(): number {
    return lastRefreshAt;
  },
  /** Force an immediate refresh — call right after start/stop/pause/log so the
   *  UI reflects the change without waiting for the next poll tick. */
  refresh,
  /** Start the shared poll (ref-counted). Returns an unsubscribe fn to call in
   *  onDestroy. The interval runs only while ≥1 consumer is subscribed. */
  subscribe(): () => void {
    subscribers++;
    if (!timer) {
      refresh();
      timer = setInterval(refresh, POLL_MS);
    }
    return () => {
      subscribers--;
      if (subscribers <= 0 && timer) {
        clearInterval(timer);
        timer = null;
        subscribers = 0;
      }
    };
  },
};
