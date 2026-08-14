<!--
  Web-client token gate (Phase 3). Mounted by the dashboard route ONLY in a
  plain browser. Dormant until any data command gets a 401 from the server
  (transport fires onUnauthorized) - then it overlays a minimal token prompt.
  Submitting stores the token (localStorage) and reloads so every view re-reads
  with it attached. A wrong token just lands back here with a nudge.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getAuthToken, setAuthToken, onUnauthorized } from "$lib/transport";

  let needed = $state(false);
  // If a stored token existed and we STILL got a 401, it's wrong - say so.
  let hadToken = $state(false);
  let value = $state("");

  onMount(() => {
    onUnauthorized(() => {
      hadToken = getAuthToken() !== null;
      needed = true;
    });
  });
  onDestroy(() => onUnauthorized(null));

  function connect() {
    const t = value.trim();
    if (!t) return;
    setAuthToken(t);
    location.reload();
  }
</script>

{#if needed}
  <div class="overlay" role="dialog" aria-modal="true">
    <div class="card">
      <img class="mark" src="/climax-icon.png" alt="Climax" width="28" height="28" />
      <h2>This server requires a token</h2>
      <p class="help">
        {#if hadToken}
          That token didn't work. Check it with whoever runs this Climax server
          and try again.
        {:else}
          Enter the token set on this Climax server (its CLIMAX_TOKEN) to
          connect.
        {/if}
      </p>
      <form onsubmit={(e) => { e.preventDefault(); connect(); }}>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="token"
          type="password"
          placeholder="Token"
          bind:value={value}
          autofocus
          autocomplete="off"
        />
        <button class="go" type="submit" disabled={!value.trim()}>Connect</button>
      </form>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--ink-950, #0a0b0f);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }
  .card {
    width: min(380px, 92vw);
    background: var(--bg-card, #15171e);
    border: 1px solid var(--border, #252934);
    border-radius: var(--radius-xl, 14px);
    padding: 28px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mark {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    display: block;
  }
  h2 {
    margin: 6px 0 0;
    font-size: 17px;
    font-weight: 600;
    color: var(--fg-strong, #f7f8fa);
    letter-spacing: -0.01em;
  }
  .help {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--fg-muted, #8a909e);
  }
  form {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .token {
    flex: 1;
    min-width: 0;
    padding: 9px 12px;
    background: var(--bg-input, #101218);
    border: 1px solid var(--border, #252934);
    border-radius: var(--radius-sm, 6px);
    color: var(--fg, #eceef3);
    font-size: 13px;
    font-family: var(--font-mono, monospace);
  }
  .token:focus {
    outline: none;
    border-color: var(--accent, #ef6b7a);
    box-shadow: 0 0 0 3px var(--coral-a-24, rgba(239, 107, 122, 0.24));
  }
  .go {
    padding: 9px 18px;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid var(--accent, #ef6b7a);
    background: var(--accent, #ef6b7a);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
  }
  .go:hover:not(:disabled) {
    background: var(--accent-hover, #f58895);
    border-color: var(--accent-hover, #f58895);
  }
  .go:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
