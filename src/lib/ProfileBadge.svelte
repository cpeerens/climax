<!--
  Scratch-profile indicator. Renders a small fixed "SCRATCH" pill ONLY when the
  app launched in the scratch/test profile (Stash off, disposable
  climax-test.sqlite). Invisible in the real profile. pointer-events:none so it
  never blocks the UI. Mounted once per window (tracker + dashboard) so a test
  session is never mistaken for the real library.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "$lib/api";

  let scratch = $state(false);

  onMount(async () => {
    try {
      const p = await api.profileGet();
      scratch = !p.stash_enabled;
    } catch {
      // Real profile, or command unavailable -> no badge.
    }
  });
</script>

{#if scratch}
  <div
    class="scratch-badge"
    title="Scratch profile: disposable test sandbox. Stash is off, so nothing here touches your real library."
  >
    SCRATCH
  </div>
{/if}

<style>
  .scratch-badge {
    position: fixed;
    bottom: 8px;
    right: 10px;
    z-index: 3000;
    pointer-events: none;
    padding: 3px 9px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    font-family: var(--font-mono, monospace);
    color: #1a1205;
    background: #f2b441;
    border: 1px solid #d99a1f;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
    user-select: none;
  }
</style>
