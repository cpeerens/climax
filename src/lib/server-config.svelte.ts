// Client-local server config + connection state for app-as-client mode (Phase 2).
//
// The desktop app can talk to an external Climax server over HTTP instead of its
// in-process backend ("client mode"). WHICH server is a CLIENT-LOCAL choice - it
// must be known BEFORE we can reach any server, so it canNOT live in the server's
// DB. It's persisted in localStorage under the same key transport.ts bootstraps
// from ("climax_server_url"); absent / empty = the in-process default (off).
// "climax_server_token" is the matching shared auth token (Phase 3), if the
// server requires one.
//
// This store is the single place the UI reads/writes that choice + the live
// connection state. transport.ts reads the same keys once at module load (so the
// transport is already targeted before this store initialises); `set()` keeps
// the two in sync by calling `setClientServer` / `setClientToken`.

import { api } from "$lib/api";
import { setClientServer, setClientToken, isTauri } from "$lib/transport";

const KEY = "climax_server_url";
const TOKEN_KEY = "climax_server_token";

function readLs(key: string): string | null {
  try {
    const v = typeof localStorage !== "undefined" ? localStorage.getItem(key) : null;
    return v && v.trim() ? v.trim() : null;
  } catch {
    return null;
  }
}

export type ConnState = "local" | "connecting" | "connected" | "unreachable";

let url = $state<string | null>(readLs(KEY)?.replace(/\/+$/, "") ?? null);
let token = $state<string | null>(readLs(TOKEN_KEY));
let state = $state<ConnState>(url ? "connecting" : "local");

export const serverConfig = {
  /** Configured server origin, or null = the in-process backend (default). */
  get url(): string | null {
    return url;
  },
  /** The server's shared auth token, or null = none configured. */
  get token(): string | null {
    return token;
  },
  /** Whether client mode is on (an external server is configured). */
  get enabled(): boolean {
    return url !== null;
  },
  /** Live connection state for the indicator: "local" = using the in-process
   *  backend; otherwise connecting / connected / unreachable. */
  get state(): ConnState {
    return state;
  },
  /** Switch backend: pass a server origin to enable client mode, or null to
   *  revert to the in-process backend. `nextToken` is the server's shared token
   *  (Phase 3), if it requires one. Persists the choice + retargets the
   *  transport. The CALLER should reload the window afterwards so every view
   *  re-fetches from the new backend (data cached from the old one is stale). */
  set(next: string | null, nextToken: string | null = null): void {
    const norm = next && next.trim() ? next.trim().replace(/\/+$/, "") : null;
    // A token only means something alongside a server.
    const tok = norm && nextToken && nextToken.trim() ? nextToken.trim() : null;
    url = norm;
    token = tok;
    state = norm ? "connecting" : "local";
    try {
      if (norm) localStorage.setItem(KEY, norm);
      else localStorage.removeItem(KEY);
      if (tok) localStorage.setItem(TOKEN_KEY, tok);
      else localStorage.removeItem(TOKEN_KEY);
    } catch {
      // localStorage unavailable - in-memory only for this session.
    }
    setClientServer(norm);
    setClientToken(tok);
    // Mirror the choice into the Rust-readable boot config so the shell's boot
    // path agrees with the frontend transport. Read at the NEXT launch to decide
    // whether to spawn the in-process backend (Phase 2c). Fire-and-forget: a webview
    // reload keeps the Rust process alive, so the write lands; the in-process
    // default (Tauri absent / no command) just ignores the rejection.
    void api.clientConfigSet(norm, tok).catch(() => {});
  },
  /** Probe the configured server and update `state`. No-op (stays "local") when
   *  on the in-process default. Used by the connection indicator's poll loop. */
  async refresh(): Promise<void> {
    if (url === null) {
      state = "local";
      return;
    }
    if (!isTauri()) {
      // A plain browser client is served BY the server it talks to, so it's
      // reachable by definition (and server_ping is a desktop-only command).
      state = "connected";
      return;
    }
    try {
      const ok = await api.serverPing(url);
      state = ok ? "connected" : "unreachable";
    } catch {
      state = "unreachable";
    }
  },
};
