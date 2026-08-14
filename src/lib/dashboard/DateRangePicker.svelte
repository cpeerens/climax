<!--
  Date range picker — pops below the Date chip in the FilterBar.

  Vertical list of presets, plus a Custom row that expands into two
  date inputs. Click outside / Escape closes (handled by FilterBar
  parent; this component just exposes onClose for that to call).
-->
<script lang="ts">
  import {
    FilterStore,
    PRESET_LABELS,
    PRESET_ORDER,
    type DatePreset,
  } from "$lib/filter-store.svelte";
  import Icon from "$lib/Icon.svelte";
  import DateTimePicker from "$lib/DateTimePicker.svelte";
  import { dayKey } from "$lib/api";

  // The custom-range form works in "YYYY-MM-DD" strings (what the store wants);
  // the picker works in epoch ms, so convert at the boundary (local midnight).
  const dayStrToMs = (s: string): number => new Date(`${s}T00:00:00`).getTime();

  type Props = {
    /** The filter store this picker drives. Each dashboard section passes its
     *  own instance, so the date window AND the "Default" choice are
     *  per-section. */
    store: FilterStore;
    onClose: () => void;
  };
  let { store, onClose }: Props = $props();

  // Custom range form state. Pre-populated ONCE from the store (these are
  // editable form fields, not a live mirror), so the initial-value capture is
  // intentional — hence the svelte-ignore on each seed.
  function todayStr(): string {
    const d = new Date();
    const pad = (n: number) => n.toString().padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  }
  // svelte-ignore state_referenced_locally
  let showCustom = $state(store.preset === "custom");
  // svelte-ignore state_referenced_locally
  let customStart = $state(store.customStart ?? store.startDay ?? todayStr());
  // svelte-ignore state_referenced_locally
  let customEnd = $state(store.customEnd ?? store.endDay ?? todayStr());

  function pick(p: DatePreset) {
    store.setPreset(p);
    onClose();
  }

  /** Set a preset as this section's default (the red "Default" badge). Does NOT
   *  change the current selection or close the menu — just remembers it. */
  function setDefault(p: DatePreset, e: MouseEvent) {
    e.stopPropagation();
    store.setUserDefault(p);
  }

  function applyCustom() {
    if (!customStart || !customEnd) return;
    if (customStart > customEnd) {
      // Swap rather than error — common slip.
      const tmp = customStart;
      customStart = customEnd;
      customEnd = tmp;
    }
    store.setCustomRange(customStart, customEnd);
    onClose();
  }
</script>

<div class="picker" role="menu" aria-label="Date range">
  {#each PRESET_ORDER as p (p)}
    <div class="preset-row" class:active={store.preset === p}>
      <button class="preset-pick" onclick={() => pick(p)}>{PRESET_LABELS[p]}</button>
      {#if store.userDefaultPreset === p}
        <span class="def-badge is-default" title="This range is the saved default for this view.">Default</span>
      {:else}
        <button
          class="def-badge"
          onclick={(e) => setDefault(p, e)}
          title="Save as the default range for this view."
        >Default</button>
      {/if}
    </div>
  {/each}

  <div class="divider"></div>

  <button
    class="preset custom-toggle"
    class:active={store.preset === "custom"}
    onclick={() => (showCustom = !showCustom)}
    aria-expanded={showCustom}
  >
    <span>{PRESET_LABELS.custom}</span>
    <Icon name={showCustom ? "chevron-down" : "chevron-right"} size={11} color="var(--fg-muted)" />
  </button>

  {#if showCustom}
    <div class="custom-fields">
      <div class="field">
        <span class="label">From</span>
        <DateTimePicker mode="date" value={dayStrToMs(customStart)} onChange={(ms) => (customStart = dayKey(new Date(ms)))} />
      </div>
      <div class="field">
        <span class="label">To</span>
        <DateTimePicker mode="date" value={dayStrToMs(customEnd)} onChange={(ms) => (customEnd = dayKey(new Date(ms)))} />
      </div>
      <button class="apply" onclick={applyCustom}>Apply</button>
    </div>
  {/if}
</div>

<style>
  .picker {
    width: 230px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong, var(--border));
    box-shadow: var(--shadow-md);
    border-radius: var(--radius-md, 8px);
    padding: 4px;
    display: flex;
    flex-direction: column;
    animation: pop-in 120ms var(--ease-out, ease-out);
  }
  @keyframes pop-in {
    from { transform: translateY(-4px); opacity: 0; }
    to   { transform: none; opacity: 1; }
  }

  .preset {
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    padding: 7px 10px;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    transition: background var(--dur-2, 120ms), color var(--dur-2, 120ms);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .preset:hover { background: var(--bg-card-hover); }
  .preset.active {
    color: var(--accent);
    background: var(--accent-soft, var(--bg-card-hover));
    font-weight: 600;
  }

  /* ---------- Preset row (selectable label + Default badge) ---------- */
  .preset-row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--radius-sm, 6px);
    padding-right: 6px;
    transition: background var(--dur-2, 120ms);
  }
  .preset-row:hover { background: var(--bg-card-hover); }
  .preset-row.active { background: var(--accent-soft, var(--bg-card-hover)); }
  .preset-row.active .preset-pick { color: var(--accent); font-weight: 600; }

  .preset-pick {
    flex: 1;
    min-width: 0;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    padding: 7px 4px 7px 10px;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
  }

  /* The "Default" affordance on the right of each row. */
  .def-badge {
    flex-shrink: 0;
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 8px;
    border-radius: var(--radius-pill, 999px);
    white-space: nowrap;
    line-height: 1.4;
  }
  /* The current default — a solid coral pill, always visible. */
  .def-badge.is-default {
    background: var(--accent);
    color: var(--accent-fg, #fff);
    border: 1px solid var(--accent);
  }
  /* Settable on the other rows — a quiet ghost that appears on row hover and
     turns coral on its own hover, so any preset can be made the default. */
  button.def-badge {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--fg-subtle);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-2, 120ms), background var(--dur-2, 120ms),
                color var(--dur-2, 120ms), border-color var(--dur-2, 120ms);
  }
  .preset-row:hover button.def-badge { opacity: 1; }
  button.def-badge:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft, transparent);
  }

  .divider {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }

  .custom-fields {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 8px 8px;
  }
  .custom-fields .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .custom-fields .label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    font-weight: 600;
  }
  .apply {
    margin-top: 2px;
    background: var(--accent);
    color: var(--accent-fg, #fff);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm, 6px);
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    padding: 6px 10px;
    cursor: pointer;
    transition: background var(--dur-2, 120ms), border-color var(--dur-2, 120ms);
  }
  .apply:hover {
    background: var(--accent-hover, var(--accent));
    border-color: var(--accent-hover, var(--accent));
  }
</style>
