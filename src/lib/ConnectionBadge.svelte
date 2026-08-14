<!--
  Connection indicator for app-as-client mode (Phase 2). Shows ONLY when client
  mode is on - i.e. the app talks to an external Climax server rather than its
  in-process backend - so a normal single-PC user never sees it.

  Rendered INLINE in each window's brand row (tracker header, dashboard sidebar)
  rather than as a floating fixed pill. It used to sit fixed in the bottom-left,
  which read as a debug overlay: the only other floating badge is SCRATCH, and
  that one is deliberately a dev warning.

  The label stays "Server" in every state so the width never jumps; the DOT
  carries the state, the same way the session dot beside it does. Unreachable
  also colours the text, so a failure isn't signalled by a 7px dot alone.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { serverConfig } from "$lib/server-config.svelte";

  let timer: ReturnType<typeof setInterval> | undefined;

  onMount(() => {
    // Only poll when client mode is on (config is set before mount; switching it
    // reloads the window, so a mount-time check is enough).
    if (serverConfig.enabled) {
      serverConfig.refresh();
      timer = setInterval(() => serverConfig.refresh(), 10000);
    }
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const hint = $derived(
    serverConfig.state === "connected"
      ? `Connected to ${serverConfig.url}`
      : serverConfig.state === "connecting"
        ? `Connecting to ${serverConfig.url}`
        : serverConfig.state === "unreachable"
          ? `Can't reach ${serverConfig.url}`
          : serverConfig.url,
  );
</script>

{#if serverConfig.enabled}
  <span
    class="conn"
    class:ok={serverConfig.state === "connected"}
    class:warn={serverConfig.state === "connecting"}
    class:err={serverConfig.state === "unreachable"}
    title={hint}
  >
    <span class="dot"></span>Server
  </span>
{/if}

<style>
  .conn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    padding: 2px 8px 2px 6px;
    border-radius: 999px;
    /* Geist, not mono: mono is reserved for numbers/times across the app, so a
       mono word read as system output. 500/11px matches the app's chip language
       (browse pills, filter chips) rather than the 700/0.08em badge language. */
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    line-height: 1.5;
    color: var(--fg-muted, #8a909e);
    background: var(--bg-card-hover, #1b1e26);
    user-select: none;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--fg-subtle, #5c6273);
    flex: 0 0 auto;
  }
  /* Healthy = quiet. No glow ring here: it competed with the session dot
     sitting right next to this in the same row. */
  .conn.ok .dot { background: var(--live, #4ade80); }
  .conn.warn .dot { background: var(--warn, #fbb454); }
  /* Only a real problem gets loud. */
  .conn.err {
    color: var(--danger, #f26b6b);
    background: color-mix(in srgb, var(--danger, #f26b6b) 14%, transparent);
  }
  .conn.err .dot { background: var(--danger, #f26b6b); }
</style>
