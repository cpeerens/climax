// Transport-agnostic command channel - Phase 1/2 of the client-server pivot.
//
// Climax is evolving from a fused desktop app into a server with thin clients
// (see CLAUDE.md "Direction: Climax is becoming a SERVER"). To get there without
// breaking anything, every backend command must be reachable over TWO transports:
//
//   - **Tauri IPC** (`invoke`) when running inside the desktop shell, and
//   - **HTTP `POST /rpc`** when running in a plain browser (the future web
//     access point served by the Climax server).
//
// `api.ts` imports `call` aliased as `invoke`, so all existing command wrappers
// become transport-agnostic with NO call-site changes.
//
// Phase 2 (app-as-client): the DESKTOP shell can now ALSO talk to an external
// server over HTTP - "client mode". When a server is configured (see
// `setClientServer`), the desktop app routes its DATA commands to that server's
// `/rpc` while its ~20 NATIVE commands (window / tray / dialog / autostart /
// restart - the AppHandle ones, NOT in `/rpc`) stay on local Tauri IPC, since
// they only make sense on the desktop client itself. Client mode is OPT-IN and
// defaults OFF, so with no server configured the desktop app behaves exactly as
// before (every command over IPC to its in-process backend, byte-for-byte
// identical). A plain browser always uses HTTP (same-origin by default).
//
// CORS note: the desktop client does NOT `fetch` the server from the webview -
// it can't, cross-origin (the server sends no CORS headers; the browser web
// client is served SAME-ORIGIN instead). Instead client-mode data commands are
// forwarded through the `rpc_http` Tauri command, which POSTs to the server's
// `/rpc` from Rust (reqwest), free of the browser's CORS policy.

import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";

// Re-export Tauri's own `isTauri` (it checks `globalThis.isTauri`, the global
// Tauri injects) so our transport detection is always identical to what the
// Tauri runtime itself uses - no hand-rolled global-name guessing that could
// drift and wrongly route the desktop app down the HTTP path.
export { isTauri };

// Desktop-only NATIVE commands: they take an `AppHandle` (window / tray / dialog
// / autostart / app-restart) and only make sense on the desktop client, so they
// are deliberately ABSENT from the server's `/rpc` surface. Even in client mode
// these ALWAYS run over local Tauri IPC against the desktop shell. This set must
// stay in sync with the AppHandle commands registered in src-tauri/src/lib.rs's
// `invoke_handler` that are NOT mirrored in climax_core::rpc::rpc_dispatch
// (compute the diff if commands change: invoke_handler list minus the /rpc arms).
//
// TWO INTENTIONAL EXCEPTIONS (Phase 2c): `idle_pending_get` and `quit_pending_get`
// DO have /rpc arms, but they read THIS process's pending_idle / pending_quit
// slots - which the desktop client's OWN native loops populate (presence.rs sets
// pending_idle; the tray Quit handler sets pending_quit). In client mode the
// server runs neither loop, so its slots are always null; routing these to the
// server would silently lose the catch-up read. So they're pinned native despite
// being on /rpc. Do NOT remove them in an allowlist re-sync. (`capture_pending_get`
// is the inverse and correctly stays NON-native: the SERVER runs capture, so the
// client must read the server's slot.)
const NATIVE_COMMANDS = new Set<string>([
  // window show/hide
  "open_dashboard",
  "open_tracker",
  // passive-capture toast (shows + positions the tracker window)
  "capture_accept",
  "capture_snooze",
  // idle-return prompt (clears slot + toggles alwaysOnTop on the tracker window)
  "idle_keep",
  "idle_discard_continue",
  "idle_discard_end",
  // idle catch-up read: returns THIS process's pending_idle slot, which the
  // client's own presence loop populates (in client mode the server runs no
  // presence loop, so its slot is always null) - must stay on local IPC.
  "idle_pending_get",
  // native save dialog
  "trends_export_csv",
  // quit guard (app.exit / window ops)
  "confirm_quit",
  "cancel_quit",
  // quit catch-up read: returns THIS process's pending_quit slot, which the
  // client's own tray Quit handler sets (the server has no quit guard) - local IPC.
  "quit_pending_get",
  // crash-recovery prompt (window ops + finish)
  "crash_keep",
  "crash_discard_continue",
  "crash_discard_end",
  // global hotkey register/unregister (needs AppHandle)
  "hotkey_enabled_set",
  // open-at-login (autostart plugin, AppHandle)
  "autostart_get",
  "autostart_set",
  // backup / restore / reset (native folder/file dialog + app.restart)
  "backup_now",
  "backup_restore_pick",
  "backup_restore_apply",
  "backup_reset",
  // the client-mode HTTP proxy itself (forwards data commands to the server from
  // Rust) - always local IPC, never routed to HTTP (it IS the HTTP transport).
  "rpc_http",
  // connection probe: GETs {base}/health from Rust (CORS-free) for the
  // connection indicator + the Settings test button. Local IPC by nature.
  "server_ping",
  // client-mode boot config (the Rust-readable record of which server, if any) -
  // a LOCAL file on this machine, always IPC, never proxied to a server.
  "client_config_get",
  "client_config_set",
  // this app's OWN listener (the port the bridge connects to, and whether the
  // bind succeeded). Routed to a server it would describe the SERVER's port, so
  // it has to stay on local IPC even in client mode.
  "server_port_get",
  "server_port_set",
  // this app's OWN version. Routed to a server it would answer with the
  // SERVER's version, and in client mode those are separately released - the
  // desktop app has to know what it is running, not what it is talking to.
  "app_version_get",
  // app relaunch (Apply when toggling client mode - the Rust boot must re-run).
  "restart_app",
]);

/** Base URL of the Climax server for the BROWSER HTTP transport. Empty string =
 *  same origin (the web client is served BY the server, so `/rpc` is relative).
 *  Overridable for a dev / separately-hosted browser client. */
let serverBase = "";

/** Point the browser HTTP transport at a specific server origin. Trailing
 *  slashes are trimmed. */
export function setServerBase(base: string): void {
  serverBase = base.replace(/\/+$/, "");
}

/** The browser HTTP transport's server origin ("" = same origin). For the few
 *  non-/rpc HTTP calls (e.g. the web backup download) so they honour the same
 *  origin override the data commands do. */
export function getServerBase(): string {
  return serverBase;
}

// ---- Web-client auth token (Phase 3) ----
// A Climax server exposed beyond localhost requires a shared token on its data
// endpoints. The BROWSER web client stores it here (localStorage) and sends it
// as the X-Climax-Token header on every /rpc call; on a 401 the registered
// handler fires so the UI can prompt (WebTokenGate). The DESKTOP app never uses
// this - its client-mode token rides through the Rust rpc_http proxy instead.

const TOKEN_KEY = "climax_token";
let authToken: string | null = null;
try {
  if (typeof localStorage !== "undefined") {
    const t = localStorage.getItem(TOKEN_KEY);
    if (t && t.trim()) authToken = t.trim();
  }
} catch {
  // localStorage unavailable - in-memory only.
}

/** The stored web-client token, if any. */
export function getAuthToken(): string | null {
  return authToken;
}

/** Store the web-client token (null clears it). Persisted in localStorage. */
export function setAuthToken(token: string | null): void {
  authToken = token && token.trim() ? token.trim() : null;
  try {
    if (authToken) localStorage.setItem(TOKEN_KEY, authToken);
    else localStorage.removeItem(TOKEN_KEY);
  } catch {
    // localStorage unavailable - in-memory only.
  }
}

let unauthorizedHandler: (() => void) | null = null;

/** Register a callback fired whenever a data command gets a 401 (the server
 *  requires a token we don't have / have wrong). One handler; last write wins. */
export function onUnauthorized(handler: (() => void) | null): void {
  unauthorizedHandler = handler;
}

/** Desktop CLIENT-MODE server origin. `null` = the in-process default (desktop
 *  routes everything over local IPC, byte-identical to pre-pivot). A non-null
 *  origin (e.g. "http://localhost:9998") makes the desktop app route DATA
 *  commands to that server over HTTP while keeping NATIVE commands on IPC. */
let clientServer: string | null = null;
/** The configured server's shared auth token (Phase 3), if it requires one.
 *  Rides through the rpc_http proxy on every client-mode data command. */
let clientToken: string | null = null;

/** Configure desktop client mode. Pass a server origin to route data commands to
 *  it over HTTP; pass `null` to revert to the in-process IPC default. Trailing
 *  slashes are trimmed. */
export function setClientServer(base: string | null): void {
  clientServer = base === null ? null : base.replace(/\/+$/, "");
}

/** Set the client-mode server token (null = the server needs none). */
export function setClientToken(token: string | null): void {
  clientToken = token && token.trim() ? token.trim() : null;
}

/** The configured client-mode server token, if any. */
export function getClientToken(): string | null {
  return clientToken;
}

/** The configured desktop client-mode server origin, or `null` if in the
 *  in-process default. */
export function getClientServer(): string | null {
  return clientServer;
}

// Opt-in before a Settings UI exists: a desktop user can point the app at a
// server by setting `localStorage.climax_server_url` (e.g. "http://localhost:9998")
// and reloading. Empty / unset = the in-process default. The connection-state UI
// for configuring this lands with the next increment; this keeps it testable now.
// `climax_server_token` is the matching shared token (Phase 3), if the server
// requires one.
try {
  if (typeof localStorage !== "undefined") {
    const saved = localStorage.getItem("climax_server_url");
    if (saved && saved.trim()) clientServer = saved.trim().replace(/\/+$/, "");
    const tok = localStorage.getItem("climax_server_token");
    if (tok && tok.trim()) clientToken = tok.trim();
  }
} catch {
  // localStorage unavailable (non-browser context) - stay on the default.
}

/** HTTP transport: POST {cmd, args} to `{base}/rpc` and unwrap the result.
 *  Mirrors how Tauri's `invoke` resolves (with the value) / rejects (with the
 *  error message). Sends the stored auth token (if any); a 401 fires the
 *  registered unauthorized handler so the web client can prompt for a token. */
async function httpRpc<T>(base: string, cmd: string, args?: Record<string, unknown>): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (authToken) headers["X-Climax-Token"] = authToken;
  const res = await fetch(`${base}/rpc`, {
    method: "POST",
    headers,
    body: JSON.stringify({ cmd, args: args ?? {} }),
  });
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    if (res.status === 401) unauthorizedHandler?.();
    const msg = body && typeof body === "object" && "error" in body ? body.error : `rpc ${cmd} failed (${res.status})`;
    // Reject with the PLAIN string, exactly like Tauri's `invoke` does - wrapping
    // in `new Error` would make String(e) render an "Error: " prefix on this
    // transport that the desktop IPC path doesn't have.
    throw String(msg);
  }
  return body as T;
}

/** Invoke a backend command over whichever transport is active. Drop-in
 *  replacement for Tauri's `invoke<T>(cmd, args)`. */
export function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    // Desktop shell. Native commands ALWAYS run locally over IPC. Data commands
    // go to the configured server in client mode, else to the in-process backend
    // over IPC (the default - byte-identical to pre-pivot behaviour).
    if (clientServer !== null && !NATIVE_COMMANDS.has(cmd)) {
      // Forward through the Rust `rpc_http` proxy (CORS-free), not a webview
      // fetch - see the CORS note at the top of this file.
      return tauriInvoke<T>("rpc_http", {
        base: clientServer,
        cmd,
        args: args ?? {},
        token: clientToken,
      });
    }
    return tauriInvoke<T>(cmd, args);
  }
  // Plain browser: always HTTP (same-origin by default, or a configured base).
  return httpRpc<T>(serverBase, cmd, args);
}
