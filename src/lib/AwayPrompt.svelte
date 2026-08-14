<!--
  "While you were away" launch prompt (Phase 7 component 2, door b). Shown once
  when the dashboard opens if Stash logged activity since your last session that
  Climax didn't track. It's a thin modal shell - the header + the "since <date>"
  caption - around UntrackedSessions in `awayMode`, which does the scan + the
  Add / Skip / Add all / Not now triage. Deeper editing (per-scene, datetimes,
  the gap knob, rejected history) lives in Settings -> Untracked sessions.
-->
<script lang="ts">
  import UntrackedSessions from "$lib/UntrackedSessions.svelte";
  import { formatDateOnly } from "$lib/api";

  let { sinceMs, onClose }: { sinceMs: number | null; onClose: () => void } = $props();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
<div
  class="overlay"
  role="dialog"
  aria-modal="true"
  tabindex="-1"
  onkeydown={(e) => { if (e.key === "Escape") onClose(); }}
  onclick={onClose}
>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal" role="document" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>While you were away</h2>
      {#if sinceMs}
        <p class="since">Since your last session, {formatDateOnly(sinceMs)}</p>
      {/if}
    </header>
    <UntrackedSessions awayMode embedded {onClose} />
  </div>
</div>

<style>
.overlay {
  position: fixed; inset: 0;
  background: rgba(7, 8, 11, 0.72);
  display: flex; align-items: center; justify-content: center;
  z-index: 1200;
  animation: fade-in 140ms cubic-bezier(0.16, 1, 0.3, 1);
}
@keyframes fade-in { from { opacity: 0 } to { opacity: 1 } }

.modal {
  background: var(--bg-card, #15171E);
  border: 1px solid var(--border, #252934);
  border-radius: var(--radius-xl, 14px);
  width: min(900px, 94vw);
  max-height: 86vh;
  display: flex; flex-direction: column;
  box-shadow: var(--shadow-lg, 0 12px 32px rgba(0,0,0,0.45));
  overflow: hidden;
  animation: pop-in 220ms cubic-bezier(0.16, 1, 0.3, 1);
}
@keyframes pop-in {
  from { transform: translateY(8px) scale(0.98); opacity: 0 }
  to   { transform: none; opacity: 1 }
}

header {
  padding: 18px 28px 16px;
  border-bottom: 1px solid var(--border-subtle, #1C1F26);
}
header h2 {
  margin: 0; font-family: var(--font-display, system-ui);
  font-size: 22px; font-weight: 600; letter-spacing: -0.01em;
  color: var(--fg-strong, #F7F8FA);
}
header .since {
  margin: 4px 0 0;
  font-size: 12px; color: var(--fg-muted, #8A909E);
  font-family: var(--font-mono, monospace);
}
</style>
