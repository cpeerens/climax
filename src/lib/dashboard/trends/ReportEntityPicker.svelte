<!--
  Report builder's entity selector — appears when Group-by is an entity
  dimension. A chip-style button opening a searchable popover: "All" (the
  default, today's ranked-breakdown behaviour) or one specific entity (drills
  the report into that entity's time series). Options are the RANGE-ACTIVE
  entities only (they come from the period's breakdown rows), so you can't pick
  something with no activity in the selected window.
-->
<script lang="ts">
  import { tick } from "svelte";
  import Icon from "$lib/Icon.svelte";

  type Option = { id: string; name: string };
  type Props = {
    /** Dimension noun for labels/placeholder ("Tag", "Performer", ...). */
    label: string;
    options: Option[];
    /** Selected entity id, or "all". */
    value: string;
    onChange: (id: string) => void;
  };
  let { label, options, value, onChange }: Props = $props();

  let open = $state(false);
  let query = $state("");
  let inputEl = $state<HTMLInputElement | undefined>();

  const currentName = $derived(
    value === "all" ? "All" : (options.find((o) => o.id === value)?.name ?? "All"),
  );
  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return options;
    return options.filter((o) => o.name.toLowerCase().includes(q));
  });

  function toggleOpen(e: MouseEvent) {
    e.stopPropagation();
    open = !open;
    if (open) {
      query = "";
      tick().then(() => inputEl?.focus());
    }
  }
  function pick(id: string) {
    onChange(id);
    open = false;
  }

  // Close on outside click / Escape (same pattern as the date popovers).
  $effect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      // composedPath(), not target.closest() - see FilterBar: a node removed by
      // its own click handler is detached by the time this runs, so closest()
      // returns null and an inside click reads as outside.
      const inside = e.composedPath().some(
        (n) => n instanceof Element && (n.classList.contains("cx-rpick")),
      );
      if (inside) return;
      open = false;
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") open = false;
    }
    document.addEventListener("click", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="cx-rpick">
  <button class="cx-rpick-btn" class:specific={value !== "all"} onclick={toggleOpen} aria-expanded={open}>
    <span class="cx-rpick-name">{currentName}</span>
    <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
  </button>
  {#if open}
    <div class="cx-rpick-pop">
      <div class="cx-rpick-search">
        <Icon name="search" size={13} color="var(--fg-subtle)" />
        <input
          bind:this={inputEl}
          bind:value={query}
          placeholder={`Search ${label.toLowerCase()}s...`}
          spellcheck="false"
        />
      </div>
      <div class="cx-rpick-list">
        <button class="cx-rpick-opt" class:selected={value === "all"} onclick={() => pick("all")}>
          <span>All</span>
          {#if value === "all"}<Icon name="check" size={12} />{/if}
        </button>
        {#each filtered as o (o.id)}
          <button class="cx-rpick-opt" class:selected={value === o.id} onclick={() => pick(o.id)} title={o.name}>
            <span>{o.name}</span>
            {#if value === o.id}<Icon name="check" size={12} />{/if}
          </button>
        {/each}
        {#if filtered.length === 0}
          <div class="cx-rpick-none">no matches in this period</div>
        {/if}
      </div>
    </div>
  {/if}
</div>
