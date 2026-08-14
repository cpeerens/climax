<!--
  In-tracker session-gap recovery prompt. Two variants, same three actions:
  - "idle": AFK during an active session (fired by presence.rs `idle_detected`).
  - "crash": a session left running, found on boot (lib.rs crash detection).
  Shows a live-updating gap duration + Keep / Discard & Continue / Discard & End;
  the backend handles the effect (the variant just picks copy + which commands).

  Why this lives inside the tracker rather than a dedicated Tauri window:
  multi-window capability scope in Tauri 2 is finicky and the dedicated
  window's IPC wouldn't initialise reliably. The tracker is already a
  configured window with full IPC access, so we host the modal here and the
  backend briefly sets alwaysOnTop=true to float it above other apps.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, formatTimeOnly } from "$lib/api";

  type Props = {
    /** Session being recovered. */
    sessionId: number;
    /** Gap-start anchor: idle = user's last input; crash = last heartbeat. Drives
     *  the live duration and is the "Discard & End" cut point. */
    gapStartedAt: number;
    /** "idle" = AFK during an active session; "crash" = a session left running,
     *  found on boot. Same three actions + effects; only the copy differs. */
    variant?: "idle" | "crash";
    /** Called after a button is clicked and the backend command finished. */
    onResolved: () => void;
  };
  let { sessionId, gapStartedAt, variant = "idle", onResolved }: Props = $props();

  let nowMs = $state(Date.now());
  let submitting = $state(false);
  let errorMsg = $state<string | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    timer = setInterval(() => { nowMs = Date.now(); }, 1000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const durationSeconds = $derived(Math.max(0, Math.floor((nowMs - gapStartedAt) / 1000)));

  function durationLabel(totalSeconds: number): string {
    const min = Math.floor(totalSeconds / 60);
    const sec = totalSeconds % 60;
    if (min === 0) return `${sec}s`;
    if (min < 60) return `${min}m ${sec.toString().padStart(2, "0")}s`;
    const h = Math.floor(min / 60);
    const rm = min % 60;
    return `${h}h ${rm}m`;
  }

  const gapLabel = $derived(formatTimeOnly(gapStartedAt));

  async function pick(action: "keep" | "discard_continue" | "discard_end") {
    if (submitting) return;
    submitting = true;
    errorMsg = null;
    try {
      if (action === "keep") {
        await (variant === "crash" ? api.crashKeep() : api.idleKeep());
      } else if (action === "discard_continue") {
        await (variant === "crash"
          ? api.crashDiscardContinue(sessionId, gapStartedAt)
          : api.idleDiscardContinue(sessionId, gapStartedAt));
      } else {
        await (variant === "crash"
          ? api.crashDiscardEnd(sessionId, gapStartedAt)
          : api.idleDiscardEnd(sessionId, gapStartedAt));
      }
      onResolved();
    } catch (e) {
      console.error("recovery resolve failed", e);
      // Designed backend sentences (e.g. client mode's "Couldn't reach the
      // Climax server.") render as-is; raw chains get the generic line.
      const s = String(e).trim();
      errorMsg =
        /^[A-Z]/.test(s) && s.endsWith(".") && !s.includes(": ") && s.length <= 200
          ? s
          : "Couldn't apply that. Try again.";
      submitting = false;
    }
  }
</script>

<div class="overlay">
  <div class="modal">
    <header>
      <div class="eyebrow">{variant === "crash" ? "Session left running for" : "You've been away for"}</div>
      <div class="duration" aria-live="polite">{durationLabel(durationSeconds)}</div>
      <div class="meta">{variant === "crash" ? "Last tracked at" : "Last input at"} <strong>{gapLabel}</strong></div>
    </header>

    <div class="actions">
      <button class="opt" disabled={submitting} onclick={() => pick("keep")}>
        <span class="opt-title">Keep</span>
        <span class="opt-meta">{variant === "crash" ? "Count the gap as session time." : "Count the time away as session time."}</span>
      </button>
      <button class="opt" disabled={submitting} onclick={() => pick("discard_continue")}>
        <span class="opt-title">Discard and continue</span>
        <span class="opt-meta">{variant === "crash" ? "Discard the gap, resume the session." : "Discard the gap, keep tracking."}</span>
      </button>
      <button class="opt danger" disabled={submitting} onclick={() => pick("discard_end")}>
        <span class="opt-title">Discard and end</span>
        <span class="opt-meta">Discard the gap, end at {gapLabel}.</span>
      </button>
    </div>

    {#if errorMsg}
      <div class="error">{errorMsg}</div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(7, 8, 11, 0.72);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1500;
    animation: fade-in 160ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  @keyframes fade-in { from { opacity: 0 } to { opacity: 1 } }

  .modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 14px;
    width: min(360px, 94vw);
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.55);
    overflow: hidden;
    animation: pop-in 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes pop-in {
    from { transform: translateY(8px) scale(0.97); opacity: 0 }
    to   { transform: none;                        opacity: 1 }
  }

  header {
    text-align: center;
    padding: 14px 18px 12px;
    border-bottom: 1px solid var(--border);
  }
  .eyebrow {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.18em;
    color: var(--fg-muted);
    margin-bottom: 2px;
  }
  .duration {
    font-family: var(--font-mono, "Geist Mono", "SF Mono", monospace);
    font-size: 22px;
    font-weight: 600;
    color: var(--fg-strong);
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin: 2px 0 4px;
  }
  .meta {
    font-size: 10px;
    color: var(--fg-muted);
  }
  .meta strong {
    color: var(--fg, #ECEEF3);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .actions {
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .opt {
    width: 100%;
    text-align: left;
    background: var(--bg-card-hover);
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 8px 11px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--fg);
    cursor: pointer;
    transition: background 120ms, border-color 120ms, transform 60ms;
    font-family: inherit;
  }
  .opt:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: #232631;
  }
  .opt:active:not(:disabled) { transform: scale(0.99); }
  .opt:disabled { opacity: 0.55; cursor: default; }
  .opt.danger:hover:not(:disabled) {
    border-color: var(--danger, #f26b6b);
    background: rgba(242, 107, 107, 0.08);
  }
  .opt-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--fg-strong);
    flex-shrink: 0;
  }
  .opt-meta {
    font-size: 10px;
    color: var(--fg-muted);
    text-align: right;
  }

  .error {
    margin: 0 8px 8px;
    padding: 6px 10px;
    border: 1px solid rgba(242, 107, 107, 0.25);
    background: rgba(242, 107, 107, 0.07);
    color: var(--danger, #f26b6b);
    border-radius: 6px;
    font-size: 11px;
  }
</style>
