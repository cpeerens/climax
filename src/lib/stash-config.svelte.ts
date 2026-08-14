// The configured Stash base URL (e.g. http://localhost:9999), loaded once and
// shared. Used to build "Open in Stash" links for scene cards as
// `{base}/scenes/{external_id}` — derived live rather than stored per-scene,
// so it's always correct even if the user later changes their Stash URL, and
// it works for every Stash scene without needing a stored `url` field.
//
// Loads eagerly on first import; consumers just read `stashConfig.baseUrl`
// reactively (it's null until the async load resolves, which renders as "no
// link yet" — fine, the link appears once loaded).

import { api } from "$lib/api";

let baseUrl = $state<string | null>(null);
let loading = false;

async function load(): Promise<void> {
  if (loading) return;
  loading = true;
  try {
    const conn = await api.stashConnectionGet();
    const trimmed = (conn.url ?? "").trim().replace(/\/+$/, "");
    baseUrl = trimmed.length > 0 ? trimmed : null;
  } catch (e) {
    console.error("stash-config load failed", e);
    loading = false; // allow a later retry via reload()
  }
}

// Kick the load off as soon as this module is first imported.
load();

export const stashConfig = {
  get baseUrl(): string | null {
    return baseUrl;
  },
  /** Re-fetch (e.g. after the user changes the Stash URL in Settings). */
  reload(): Promise<void> {
    loading = false;
    return load();
  },
};
