<!--
  Presentational "Default" pill — the app's standard pin-a-default affordance
  (date dropdown, metric toggles, gender toggle). Solid coral "Default" when the
  current selection IS the saved default; otherwise a ghost "Set default" button
  that pins it. Persistence is the caller's job via `onSet`.
-->
<script lang="ts">
  type Props = {
    isDefault: boolean;
    onSet: () => void;
    /** Tooltip overrides (defaults suit most call sites). */
    isTitle?: string;
    setTitle?: string;
  };
  let {
    isDefault,
    onSet,
    isTitle = "This is your default.",
    setTitle = "Pin this as your default",
  }: Props = $props();
</script>

{#if isDefault}
  <span class="md-pill is-default" title={isTitle}>Default</span>
{:else}
  <button class="md-pill" onclick={onSet} title={setTitle}>Set default</button>
{/if}

<style>
  .md-pill {
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 3px 9px;
    border-radius: var(--radius-pill, 999px);
    white-space: nowrap;
    line-height: 1.4;
    flex-shrink: 0;
  }
  .md-pill.is-default {
    background: var(--accent);
    color: var(--accent-fg, #fff);
    border: 1px solid var(--accent);
  }
  button.md-pill {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--fg-subtle);
    cursor: pointer;
    transition: color var(--dur-2, 120ms), border-color var(--dur-2, 120ms), background var(--dur-2, 120ms);
  }
  button.md-pill:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft, transparent);
  }
</style>
