<!--
  Passive-capture toast (Phase 7 component 3). Rendered inside the tracker
  window (which the backend shows + floats bottom-right) when the user is
  watching a scene with no session running. Climax-themed; Track starts a
  backdated session, Snooze dismisses for the configured grace.

  It lives in the tracker rather than its own window for the same reason the
  idle prompt does: a dedicated Tauri 2 window's IPC didn't initialise reliably.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import type { CapturePayload } from "$lib/api";

  let {
    payload,
    busy = false,
    onTrack,
    onSnooze,
  }: {
    payload: CapturePayload;
    busy?: boolean;
    onTrack: () => void;
    onSnooze: () => void;
  } = $props();

  const title = $derived(
    (payload.title && payload.title.trim()) || `Scene ${payload.scene_id}`,
  );
  // Live "watching for ~Xm" — the watch began at watch_started_at. The toast
  // persists across payload changes (prop update, not remount), so a one-shot
  // Date.now() would freeze; tick it.
  let nowMs = $state(Date.now());
  let ticker: ReturnType<typeof setInterval> | null = null;
  onMount(() => {
    ticker = setInterval(() => { nowMs = Date.now(); }, 10_000);
  });
  onDestroy(() => {
    if (ticker) clearInterval(ticker);
  });
  const watchedMin = $derived(
    Math.max(1, Math.round((nowMs - payload.watch_started_at) / 60000)),
  );
</script>

<div class="cap">
  <div class="cap-head">
    {#if payload.thumbnail_url}
      <img class="cap-thumb" src={payload.thumbnail_url} alt="" />
    {:else}
      <div class="cap-thumb cap-thumb-fallback">
        <Icon name="climax-glyph" size={20} color="var(--accent)" />
      </div>
    {/if}
    <div class="cap-text">
      <div class="cap-eyebrow">Not tracking</div>
      <div class="cap-title" title={title}>{title}</div>
      <div class="cap-sub">Watching {watchedMin} minute{watchedMin === 1 ? "" : "s"} with no session</div>
    </div>
  </div>
  <div class="cap-actions">
    <button class="cap-snooze" onclick={onSnooze} disabled={busy}>Snooze</button>
    <button class="cap-track" onclick={onTrack} disabled={busy}>
      {busy ? "Starting..." : "Track this"}
    </button>
  </div>
</div>

<style>
  .cap {
    display: flex;
    flex-direction: column;
    gap: 13px;
    padding: 2px 2px 4px;
  }
  .cap-head {
    display: flex;
    gap: 11px;
    align-items: center;
    min-width: 0;
  }
  .cap-thumb {
    width: 76px;
    aspect-ratio: 16 / 9;
    height: auto;
    flex-shrink: 0;
    border-radius: 7px;
    border: 1px solid var(--border);
    object-fit: cover;
    background: linear-gradient(135deg, #1d1218 0%, #14101a 100%);
  }
  .cap-thumb-fallback {
    display: grid;
    place-items: center;
  }
  .cap-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cap-eyebrow {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.16em;
    font-weight: 600;
    color: var(--accent);
  }
  .cap-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cap-sub {
    font-size: 11px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .cap-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .cap-actions button {
    padding: 7px 16px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 600;
    border: 1px solid var(--border);
    cursor: pointer;
    font-family: inherit;
    transition: background 120ms, border-color 120ms, transform 60ms;
  }
  .cap-actions button:active:not(:disabled) {
    transform: scale(0.98);
  }
  .cap-actions button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .cap-snooze {
    background: transparent;
    color: var(--fg-muted);
  }
  .cap-snooze:hover:not(:disabled) {
    background: var(--bg-card-hover);
    border-color: var(--border-strong);
    color: var(--fg);
  }
  .cap-track {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .cap-track:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
</style>
