<!--
  Generic dropdown for a filter chip. A transparent full-screen scrim closes it
  on outside click; the menu itself is positioned by the anchoring .chip-wrap in
  BrowseFilterBar (position: relative there).
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    open: boolean;
    onClose: () => void;
    children: Snippet;
  };
  let { open, onClose, children }: Props = $props();
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" onmousedown={onClose}></div>
  <div class="menu">{@render children()}</div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: transparent;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 200px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    box-shadow: var(--shadow-md);
    padding: 4px;
    z-index: 100;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
</style>
