<!--
  FilterBar — sticky filter strip on the Overview. Sits between the
  top hero row (immutable lifestyle metrics) and the "this range"
  hero row, so the layout itself communicates what's affected by
  filters.

  Reads / writes the global filter store. Each chip opens its own
  popover (DateRangePicker for the date chip; EntityPicker for the
  four entity chips). Below the chip row, a small meta line shows
  the matching session count + a human range summary + Reset.
-->
<script lang="ts">
  import { filterStore } from "$lib/filter-store.svelte";
  import DateRangePicker from "$lib/dashboard/DateRangePicker.svelte";
  import EntityPicker from "$lib/dashboard/EntityPicker.svelte";
  import { api } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  type ChipKey = "date" | "scene" | "performer" | "studio" | "tag";

  let openChip = $state<ChipKey | null>(null);
  let matchCount = $state<number | null>(null);
  let countInFlight = $state(false);

  /** Whenever any filter facet changes, re-fetch the matching session
   *  count for the meta line. Tiny query; happens often but cheap. */
  $effect(() => {
    const start = filterStore.startDay;
    const end = filterStore.endDay;
    const scenes = filterStore.sceneIds;
    const performers = filterStore.performerIds;
    const studios = filterStore.studioIds;
    const tags = filterStore.tagIds;
    refreshCount({ startDay: start, endDay: end, sceneContentItemIds: scenes, performerIds: performers, studioIds: studios, tagIds: tags });
  });

  async function refreshCount(args: {
    startDay: string;
    endDay: string;
    sceneContentItemIds: number[];
    performerIds: string[];
    studioIds: string[];
    tagIds: string[];
  }) {
    countInFlight = true;
    try {
      matchCount = await api.dashboardFilteredSessionCount(args);
    } catch (e) {
      console.error("filtered session count failed", e);
    } finally {
      countInFlight = false;
    }
  }

  /** Document-level click / Escape handling for the open popover. */
  $effect(() => {
    if (openChip === null) return;
    function onDocClick(e: MouseEvent) {
      // composedPath() is captured when the event is DISPATCHED, so it still
      // holds the ancestors even if the clicked node was removed by its own
      // handler. e.target.closest() cannot: a detached node has no ancestors,
      // returns null, and an INSIDE click then reads as an outside one and
      // closes the popover. Bit the date picker's month/year zoom, where
      // picking a month destroys the button you just clicked.
      const inside = e.composedPath().some(
        (n) => n instanceof Element && (n.classList.contains("filter-popover") || n.classList.contains("chip-wrap")),
      );
      if (inside) return;
      openChip = null;
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") openChip = null;
    }
    document.addEventListener("click", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  });

  function toggleChip(k: ChipKey, e: MouseEvent) {
    e.stopPropagation();
    openChip = openChip === k ? null : k;
  }

  /** Chip label: the noun when empty, the single name when one is selected, else
   *  "first +N" so you can see WHAT you've filtered to (full list in the title). */
  function entityChipLabel(noun: string, names: string[]): string {
    if (names.length === 0) return noun;
    if (names.length === 1) return names[0];
    return `${names[0]} +${names.length - 1}`;
  }
  const sceneNames = $derived(filterStore.scenes.map((s) => s.title ?? `Scene ${s.external_id ?? s.id}`));
  const performerNames = $derived(filterStore.performers.map((p) => p.name));
  const studioNames = $derived(filterStore.studios.map((s) => s.name));
  const tagNames = $derived(filterStore.tags.map((t) => t.name));
</script>

<div class="filter-bar">
  <div class="chips-row">
    <div class="chip-wrap">
      <button
        class="chip date"
        class:active={openChip === "date"}
        onclick={(e) => toggleChip("date", e)}
        aria-expanded={openChip === "date"}
      >
        <Icon name="calendar" size={13} />
        <span>{filterStore.presetLabel}</span>
        <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
      </button>
      {#if openChip === "date"}
        <div class="filter-popover">
          <DateRangePicker store={filterStore} onClose={() => (openChip = null)} />
        </div>
      {/if}
    </div>

    <div class="chip-wrap">
      <button
        class="chip"
        class:has-selections={filterStore.scenes.length > 0}
        class:active={openChip === "scene"}
        onclick={(e) => toggleChip("scene", e)}
        aria-expanded={openChip === "scene"}
        title={sceneNames.join(", ")}
      >
        <Icon name="clapperboard" size={13} />
        <span class="chip-label">{entityChipLabel("Scene", sceneNames)}</span>
        <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
      </button>
      {#if openChip === "scene"}
        <div class="filter-popover">
          <EntityPicker kind="scene" onClose={() => (openChip = null)} />
        </div>
      {/if}
    </div>

    <div class="chip-wrap">
      <button
        class="chip"
        class:has-selections={filterStore.performers.length > 0}
        class:active={openChip === "performer"}
        onclick={(e) => toggleChip("performer", e)}
        aria-expanded={openChip === "performer"}
        title={performerNames.join(", ")}
      >
        <Icon name="user" size={13} />
        <span class="chip-label">{entityChipLabel("Performer", performerNames)}</span>
        <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
      </button>
      {#if openChip === "performer"}
        <div class="filter-popover">
          <EntityPicker kind="performer" onClose={() => (openChip = null)} />
        </div>
      {/if}
    </div>

    <div class="chip-wrap">
      <button
        class="chip"
        class:has-selections={filterStore.studios.length > 0}
        class:active={openChip === "studio"}
        onclick={(e) => toggleChip("studio", e)}
        aria-expanded={openChip === "studio"}
        title={studioNames.join(", ")}
      >
        <Icon name="video" size={13} />
        <span class="chip-label">{entityChipLabel("Studio", studioNames)}</span>
        <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
      </button>
      {#if openChip === "studio"}
        <div class="filter-popover">
          <EntityPicker kind="studio" onClose={() => (openChip = null)} />
        </div>
      {/if}
    </div>

    <div class="chip-wrap">
      <button
        class="chip"
        class:has-selections={filterStore.tags.length > 0}
        class:active={openChip === "tag"}
        onclick={(e) => toggleChip("tag", e)}
        aria-expanded={openChip === "tag"}
        title={tagNames.join(", ")}
      >
        <Icon name="tag" size={13} />
        <span class="chip-label">{entityChipLabel("Tag", tagNames)}</span>
        <Icon name="chevron-down" size={11} color="var(--fg-subtle)" />
      </button>
      {#if openChip === "tag"}
        <div class="filter-popover">
          <EntityPicker kind="tag" onClose={() => (openChip = null)} />
        </div>
      {/if}
    </div>
  </div>

  <div class="meta-row">
    <span class="count">
      {#if countInFlight && matchCount === null}
        ...
      {:else}
        {matchCount ?? 0} session{matchCount === 1 ? " matches" : "s match"}
      {/if}
    </span>
    <span class="sep">·</span>
    <span class="range">{filterStore.rangeLabel}</span>
    {#if !filterStore.isDefault}
      <span class="sep">·</span>
      <button class="reset" onclick={() => filterStore.reset()}>Reset</button>
    {/if}
  </div>
</div>

<style>
  .filter-bar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 0 8px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg);
    position: sticky;
    top: 0;
    z-index: 8;
  }
  /* The scroll container (.content) has an 18px top padding, so the bar pins a
     little BELOW the true top, leaving a strip above it where the content below
     peeks through as you scroll past. Extend the bar's opaque background up to
     cover that strip. It rides with the pinned bar and is clipped by .content's
     overflow, so it never bleeds above the scroll top - and it adds no layout. */
  .filter-bar::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 100%;
    /* MUST equal .content's top padding. Too small and content peeks above the
       pinned bar; too large and it paints over whatever sits above the bar when
       NOT pinned, which is how it was clipping the hero stats card. Clipped at
       the scroll top, so it never bleeds outside. */
    height: 12px;
    background: var(--bg);
  }

  .chips-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .chip-wrap {
    position: relative;
  }

  .chip {
    background: var(--bg-card);
    border: 1px solid var(--border);
    color: var(--fg-muted);
    border-radius: 7px;
    padding: 5px 11px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--dur-2), border-color var(--dur-2), color var(--dur-2);
  }
  .chip:hover:not(:disabled) {
    background: var(--bg-card-hover);
    border-color: var(--border-strong);
    color: var(--fg);
  }
  .chip.date {
    color: var(--fg);
    background: var(--bg-card-hover);
  }
  .chip.has-selections {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-soft, var(--bg-card-hover));
  }
  .chip.active {
    border-color: var(--accent);
    color: var(--fg);
  }
  /* Entity-chip label can hold a name now — cap width + ellipsis so the chip
     stays scannable (full list is in the button's title). */
  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 160px;
  }

  .filter-popover {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 50;
  }

  .meta-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .meta-row .count { font-weight: 600; color: var(--fg); }
  .meta-row .sep { color: var(--border-strong, var(--border)); }
  .meta-row .reset {
    background: transparent;
    border: 1px solid transparent;
    color: var(--text-muted);
    font-family: inherit;
    font-size: 11px;
    padding: 1px 7px;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    transition: color var(--dur-2, 120ms), border-color var(--dur-2, 120ms);
  }
  .meta-row .reset:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
</style>
