<!--
  Browse filter bar — filter chips (each a dropdown) on the left, a "Clear all"
  chip when any chip is non-default, and the Sort dropdown on the right. Chip
  state + sort live in the parent's `filters` object; this component reports
  changes via `set(key, value)`.
-->
<script lang="ts">
  import type { ComponentProps, Snippet } from "svelte";
  import Select from "$lib/Select.svelte";
  import Icon from "$lib/Icon.svelte";
  import FilterMenu from "./FilterMenu.svelte";

  type IconName = ComponentProps<typeof Icon>["name"];
  type Option = { value: string; label: string };
  export type ChipDef = {
    id: string;
    label: string;
    icon?: IconName;
    options: Option[];
  };

  type Props = {
    filters: Record<string, string>;
    set: (key: string, value: string) => void;
    chipDefs: ChipDef[];
    sortOptions: Option[];
    onReset: () => void;
    /** Per-page default-view affordance: `isDefault` reflects whether the current
     *  view matches the saved default; `onSetDefault` pins the current view. */
    isDefault?: boolean;
    onSetDefault?: () => void;
    /** Optional controls rendered on the right, after Sort (e.g. the pager) —
     *  so they share this line instead of taking their own row. */
    rightExtra?: Snippet;
  };
  let { filters, set, chipDefs, sortOptions, onReset, isDefault = false, onSetDefault, rightExtra }: Props = $props();

  let openChip = $state<string | null>(null);

  // A chip is "active" when its value differs from its first option (the
  // default, conventionally "Any").
  function isActive(def: ChipDef): boolean {
    return !!filters[def.id] && filters[def.id] !== def.options[0].value;
  }
  function current(def: ChipDef): Option {
    return def.options.find((o) => o.value === filters[def.id]) ?? def.options[0];
  }
  const anyActive = $derived(chipDefs.some(isActive));
</script>

<div class="bar">
  <div class="chips">
    {#each chipDefs as def (def.id)}
      <div class="chip-wrap">
        <button
          class="chip"
          class:active={isActive(def)}
          onclick={(e) => {
            e.stopPropagation();
            openChip = openChip === def.id ? null : def.id;
          }}
        >
          {#if def.icon}<Icon name={def.icon} size={13} />{/if}
          <span>{def.label}{isActive(def) ? `: ${current(def).label}` : ""}</span>
          <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
        </button>
        <FilterMenu open={openChip === def.id} onClose={() => (openChip = null)}>
          {#each def.options as opt (opt.value)}
            <button
              class="opt"
              class:selected={filters[def.id] === opt.value}
              onclick={() => {
                set(def.id, opt.value);
                openChip = null;
              }}
            >
              <span>{opt.label}</span>
              {#if filters[def.id] === opt.value}<Icon name="check" size={12} />{/if}
            </button>
          {/each}
        </FilterMenu>
      </div>
    {/each}
    {#if anyActive}
      <button class="chip clear" onclick={onReset}>Clear all</button>
    {/if}
  </div>

  <div class="bb-right">
    {#if onSetDefault}
      {#if isDefault}
        <span class="bb-default is-default" title="This is the saved default view for this page.">Default</span>
      {:else}
        <button class="bb-default" onclick={onSetDefault} title="Save the current search, filters, sort and rows as this page's default view.">
          Set as default
        </button>
      {/if}
    {/if}
    <div class="sort">
      <span class="sort-label">Sort</span>
      <Select
        value={filters.sort}
        options={sortOptions}
        onChange={(v) => set("sort", v)}
        ariaLabel="Sort"
      />
    </div>
    {#if rightExtra}{@render rightExtra()}{/if}
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .chip-wrap {
    position: relative;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-card-hover);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 7px;
    padding: 5px 11px;
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--dur-2), border-color var(--dur-2), color var(--dur-2);
  }
  .chip:hover {
    border-color: var(--border-strong);
  }
  .chip.active {
    background: var(--coral-a-16);
    border-color: var(--coral-700);
    color: var(--fg-strong);
  }
  .chip.clear {
    border-style: dashed;
    color: var(--fg-subtle);
  }
  .chip.clear:hover {
    color: var(--fg);
  }

  .opt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 10px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--fg);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background var(--dur-2);
  }
  .opt:hover {
    background: var(--bg-card-hover);
  }
  .opt.selected {
    background: var(--coral-a-08);
    color: var(--fg-strong);
  }
  .opt.selected :global(svg) {
    color: var(--accent);
  }

  .bb-right {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  /* "Set as default" / "Default" pill — same language as the date dropdown. */
  .bb-default {
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 4px 11px;
    border-radius: var(--radius-pill, 999px);
    white-space: nowrap;
    line-height: 1.4;
  }
  .bb-default.is-default {
    background: var(--accent);
    color: var(--accent-fg, #fff);
    border: 1px solid var(--accent);
  }
  button.bb-default {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--fg-subtle);
    cursor: pointer;
    transition: color var(--dur-2), border-color var(--dur-2), background var(--dur-2);
  }
  button.bb-default:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--coral-a-08, transparent);
  }
  .sort {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sort-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.14em;
    color: var(--fg-subtle);
    font-weight: 600;
  }
</style>
