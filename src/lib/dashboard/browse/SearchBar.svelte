<!--
  Browse search bar — magnifier + input + clear + live result count.
  Shared by all four browse pages. `value` is bindable; `count` is the number
  of results after the current search/filter.
-->
<script lang="ts">
  import Icon from "$lib/Icon.svelte";

  type Props = {
    value: string;
    placeholder?: string;
    count: number;
  };
  let { value = $bindable(""), placeholder = "Search...", count }: Props = $props();
</script>

<div class="search">
  <Icon name="search" size={14} color="var(--fg-subtle)" />
  <input class="search-input" bind:value {placeholder} />
  {#if value}
    <button class="search-clear" onclick={() => (value = "")} aria-label="Clear search">×</button>
  {/if}
  <span class="search-count">{count} result{count === 1 ? "" : "s"}</span>
</div>

<style>
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 12px;
    transition: border-color var(--dur-2), box-shadow var(--dur-2);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--coral-a-24);
  }
  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--fg);
    font-family: inherit;
    font-size: 13px;
    min-width: 0;
  }
  .search-input::placeholder {
    color: var(--fg-subtle);
  }
  .search-clear {
    background: transparent;
    border: none;
    color: var(--fg-muted);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
    font-family: inherit;
  }
  .search-clear:hover {
    color: var(--fg);
  }
  .search-count {
    font-size: 11px;
    color: var(--fg-subtle);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    padding-left: 8px;
    border-left: 1px solid var(--border);
    margin-left: 4px;
  }
</style>
