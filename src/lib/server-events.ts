// Server -> client event channel for app-as-client mode (Phase 2).
//
// When the desktop app is configured to talk to an EXTERNAL Climax server (client
// mode), the server's backend can't reach into this app's Tauri event bus, so it
// PUSHES its UiSignals over a WebSocket (the server's `/events` endpoint - a
// pure server->client push, separate from the bridge's `/ws`). This module
// connects there and translates each signal into the SAME local action the
// in-process `lib.rs` reactor performs, so the existing tracker listeners
// (`capture_prompt`, `tracker_stop_requested`) fire unchanged.
//
// Active ONLY in client mode + only in the Tauri shell. On the in-process default
// the Rust reactor handles UiSignals directly, so this stays dormant. (Pointing
// the app at its OWN in-process server is a transition-time self-test: there the
// reactor AND this channel both fire, a benign duplicate - `open_tracker` is
// idempotent, a second `tracker_stop_requested` no-ops, and the capture toast is
// default-off. The duplication disappears once the in-process backend retires in
// the single-PC cutover.) Reconnects with a fixed backoff while client mode is on.

import { emit } from "@tauri-apps/api/event";
import { call, isTauri } from "$lib/transport";
import { serverConfig } from "$lib/server-config.svelte";

/** http(s)://host:port -> ws(s)://host:port/events. The token (if the server
 *  requires one) rides as ?token= - a browser WebSocket can't set headers. */
function eventsUrl(httpBase: string): string {
  const base = httpBase.replace(/\/+$/, "");
  const path = serverConfig.token
    ? `/events?token=${encodeURIComponent(serverConfig.token)}`
    : "/events";
  if (base.startsWith("https://")) return "wss://" + base.slice("https://".length) + path;
  if (base.startsWith("http://")) return "ws://" + base.slice("http://".length) + path;
  return "ws://" + base + path;
}

let ws: WebSocket | null = null;
let stopped = false;
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

function dispatch(msg: unknown): void {
  const type = (msg as { type?: string })?.type;
  switch (type) {
    case "show_tracker":
      // Bridge pill clicked while idle (a session was started server-side): raise
      // the tracker, mirroring the reactor's show_solo.
      void call("open_tracker").catch(() => {});
      break;
    case "show_dashboard":
      // Bridge pill "Open Climax": raise the dashboard (the same surface the
      // browser path opens), without touching the session.
      void call("open_dashboard").catch(() => {});
      break;
    case "tracker_stop_requested":
      // Bridge pill "request stop": show the tracker + run its stop / wrap-up flow
      // (the tracker route listens for this and runs the same stop() as the button).
      void call("open_tracker").catch(() => {});
      void emit("tracker_stop_requested").catch(() => {});
      break;
    case "capture_prompt": {
      // Passive-capture candidate: the message is the flattened CapturePayload +
      // the "type" wire tag. Strip the tag so the listener gets a bare
      // CapturePayload, then raise the tracker so the toast is visible.
      const payload = { ...(msg as Record<string, unknown>) };
      delete payload.type;
      void emit("capture_prompt", payload).catch(() => {});
      void call("open_tracker").catch(() => {});
      break;
    }
    // "hello" + anything unknown: ignore.
  }
}

function connect(): void {
  if (stopped || !serverConfig.url) return;
  try {
    ws = new WebSocket(eventsUrl(serverConfig.url));
  } catch {
    scheduleReconnect();
    return;
  }
  ws.onmessage = (e) => {
    try {
      dispatch(JSON.parse(e.data as string));
    } catch {
      // ignore malformed frames
    }
  };
  ws.onclose = () => {
    ws = null;
    scheduleReconnect();
  };
  ws.onerror = () => {
    try {
      ws?.close();
    } catch {
      /* noop */
    }
  };
}

function scheduleReconnect(): void {
  if (stopped || reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = undefined;
    connect();
  }, 5000);
}

/** Start the server-event channel if in client mode (Tauri shell only). Returns a
 *  stop fn (for onDestroy). A no-op on the in-process default. */
export function startServerEvents(): () => void {
  if (!isTauri() || !serverConfig.enabled) return () => {};
  stopped = false;
  connect();
  return () => {
    stopped = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = undefined;
    }
    try {
      ws?.close();
    } catch {
      /* noop */
    }
    ws = null;
  };
}
