<!--
  Browse page chassis — shared by Scenes / Performers / Studios / Tags.
  Search + filter bar + a PAGINATED grid (tile via a snippet) with a Sessions-
  style "Rows" selector + Prev/Next pager (top & bottom). "Fit" (the default)
  measures how many tile rows × columns fit the viewport so the page loads
  without a scrollbar; explicit sizes (10/25/50/100) may scroll.

  Clicking a tile DRILLS IN: the grid is replaced by a full-width detail view
  with a "← Back" control that restores the grid at the exact scroll position.
  Search / filter / sort run in-memory over `items`.

  Imports browse.css once (global, cx- prefixed) — the card design system.
-->
<script lang="ts">
  import "./browse.css";
  import Select from "$lib/Select.svelte";
  import { tick, onMount, onDestroy, type Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import SearchBar from "./SearchBar.svelte";
  import BrowseFilterBar, { type ChipDef } from "./BrowseFilterBar.svelte";
  import DetailPanel from "./DetailPanel.svelte";
  import { nav, KIND_SECTION, type BrowseKind } from "$lib/dashboard/nav.svelte";
  import { api } from "$lib/api";

  type Item = any;
  type Option = { value: string; label: string };
  type Props = {
    kind: BrowseKind;
    items: Item[];
    loading: boolean;
    searchPlaceholder: string;
    chipDefs: ChipDef[];
    sortOptions: Option[];
    defaultFilters: Record<string, string>;
    applyFilters: (items: Item[], filters: Record<string, string>, search: string) => Item[];
    idOf: (item: Item) => string | number;
    gridClass: string;
    tile: Snippet<[Item, boolean, () => void]>;
  };
  let {
    kind,
    items,
    loading,
    searchPlaceholder,
    chipDefs,
    sortOptions,
    defaultFilters,
    applyFilters,
    idOf,
    gridClass,
    tile,
  }: Props = $props();

  let search = $state("");
  // defaultFilters is a static per-section config; seeding mutable local filter
  // state from it once is intentional (it never changes after mount).
  // svelte-ignore state_referenced_locally
  let filters = $state<Record<string, string>>({ ...defaultFilters });
  let selected = $state<Item | null>(null);
  let rootEl = $state<HTMLDivElement | undefined>();
  let gridEl = $state<HTMLDivElement | undefined>();

  const filtered = $derived(applyFilters(items, filters, search));
  const backLabel = $derived(
    ({ scene: "Scenes", performer: "Performers", studio: "Studios", tag: "Tags" } as Record<
      BrowseKind,
      string
    >)[kind],
  );

  // ---------- Pagination ----------
  type RowsMode = "fit" | "10" | "25" | "50" | "100";
  const ROWS_OPTIONS: RowsMode[] = ["fit", "10", "25", "50", "100"];
  let rowsMode = $state<RowsMode>("fit");
  let page = $state(0);
  let fitCount = $state(12); // recomputed from the viewport; provisional until measured

  const pageSize = $derived(rowsMode === "fit" ? Math.max(1, fitCount) : parseInt(rowsMode, 10));
  const total = $derived(filtered.length);
  const offset = $derived(page * pageSize);
  const pageItems = $derived(filtered.slice(offset, offset + pageSize));
  const rangeStart = $derived(total === 0 ? 0 : offset + 1);
  const rangeEnd = $derived(Math.min(offset + pageSize, total));
  const canPrev = $derived(page > 0);
  const canNext = $derived(offset + pageSize < total);

  function setFilter(key: string, value: string) {
    filters[key] = value;
  }
  function reset() {
    filters = { ...defaultFilters };
  }

  // ---------- Per-page default view ----------
  // The user can save the current search + filters + sort + rows as this page's
  // default; it's restored on every visit. Independent per kind.
  type DefaultView = { search: string; filters: Record<string, string>; rowsMode: RowsMode };
  let savedDefault = $state<DefaultView | null>(null);

  function currentView(): DefaultView {
    return { search, filters: { ...filters }, rowsMode };
  }
  function viewsEqual(a: DefaultView, b: DefaultView): boolean {
    if (a.search !== b.search || a.rowsMode !== b.rowsMode) return false;
    const keys = new Set([...Object.keys(a.filters), ...Object.keys(b.filters)]);
    for (const k of keys) if ((a.filters[k] ?? "") !== (b.filters[k] ?? "")) return false;
    return true;
  }
  // Baseline when the user hasn't saved one: the static config + empty search + Fit.
  // defaultFilters is a static per-section config — capturing it once is intended.
  // svelte-ignore state_referenced_locally
  const baselineDefault: DefaultView = { search: "", filters: { ...defaultFilters }, rowsMode: "fit" };
  const isDefaultView = $derived(viewsEqual(currentView(), savedDefault ?? baselineDefault));

  async function loadDefault() {
    try {
      const raw = await api.browseDefaultGet(kind);
      if (!raw) return;
      const v = JSON.parse(raw) as Partial<DefaultView>;
      const view: DefaultView = {
        search: typeof v.search === "string" ? v.search : "",
        filters: { ...defaultFilters, ...(v.filters ?? {}) },
        rowsMode: (ROWS_OPTIONS as string[]).includes(v.rowsMode as string) ? (v.rowsMode as RowsMode) : "fit",
      };
      savedDefault = view;
      // Apply it as the current view.
      search = view.search;
      filters = { ...view.filters };
      rowsMode = view.rowsMode;
    } catch (e) {
      console.warn("browse default load failed", e);
    }
  }
  async function setAsDefault() {
    const v = currentView();
    savedDefault = v;
    try {
      await api.browseDefaultSet(kind, JSON.stringify(v));
    } catch (e) {
      console.warn("browse default save failed", e);
    }
  }

  // Reset to page 0 when the result set changes (new search / filter / sort).
  let prevFilterSig = "";
  $effect(() => {
    // NUL separator (spelled out so the source stays pure text / grep-auditable).
    const sig = JSON.stringify(filters) + String.fromCharCode(0) + search;
    if (prevFilterSig !== "" && prevFilterSig !== sig) page = 0;
    prevFilterSig = sig;
  });
  // Reset to page 0 when the page size changes.
  let prevPageSize = -1;
  $effect(() => {
    if (prevPageSize !== -1 && pageSize !== prevPageSize) page = 0;
    prevPageSize = pageSize;
  });
  // Backstop: never leave `page` past the end (e.g. after a poll shrinks total).
  $effect(() => {
    const maxPage = Math.max(0, Math.ceil(total / Math.max(1, pageSize)) - 1);
    if (page > maxPage) page = maxPage;
  });

  // "Fit": measure the real grid (column count × tallest tile) against the
  // scroll container's visible height, so the default page fills the window
  // without a scrollbar. Re-runs on mount, resize, data change, and mode change.
  function computeFit() {
    if (rowsMode !== "fit" || selected || !gridEl) return;
    const sc = getScrollParent(rootEl ?? null);
    const cs = getComputedStyle(gridEl);
    const cols = cs.gridTemplateColumns.split(" ").filter((t) => t && t !== "none").length || 1;
    const gap = parseFloat(cs.rowGap || cs.gap || "14") || 14;
    // DERIVE the tile height; do not measure a rendered tile. Measuring is a
    // feedback loop - fitCount decides what gets rendered, and we were sizing
    // from those same tiles, so one bad reading fed itself: measure short,
    // render more, cram them, measure shorter. A collapsed tile is not zero, so
    // the old `tileH <= 0` guard never caught it. (Seen for real on macOS: a
    // 2px-wide thumbnail produced tileH=44 and fitCount=91 for 34 items.)
    //
    // The thumbnail's height is a known ratio of the COLUMN width, and the body
    // is text that lays out reliably, so the two together give the right answer
    // on the first pass - before images load, and whatever the tiles look like.
    const first = gridEl.children[0] as HTMLElement | undefined;
    if (!first) return;
    const colGap = parseFloat(cs.columnGap || cs.gap || "14") || 14;
    const colW = (gridEl.getBoundingClientRect().width - colGap * (cols - 1)) / cols;
    const thumbEl = first.querySelector(".cx-tile-thumb") as HTMLElement | null;
    // TALLEST body, not the first one. Tiles vary (1- vs 2-line titles), and the
    // original measured the tallest tile for exactly that reason - taking the
    // first would compute a row too many and overflow into a scrollbar. The
    // body is TEXT, so it lays out correctly even when the thumbnail does not,
    // which is what makes it safe to measure when the tile as a whole is not.
    let bodyH = 0;
    for (const el of Array.from(gridEl.children)) {
      const b = (el as HTMLElement).querySelector(".cx-tile-body") as HTMLElement | null;
      if (b) bodyH = Math.max(bodyH, b.getBoundingClientRect().height);
    }
    let tileH = 0;
    const ar = thumbEl ? getComputedStyle(thumbEl).aspectRatio : "auto";
    const m = /^\s*([\d.]+)\s*\/\s*([\d.]+)\s*$/.exec(ar ?? "");
    if (m && colW > 0 && bodyH > 0) {
      const tileGap = parseFloat(getComputedStyle(first).rowGap || "8") || 8;
      tileH = colW * (parseFloat(m[2]) / parseFloat(m[1])) + tileGap + bodyH;
    } else {
      // No aspect-ratio to derive from: fall back to measuring.
      for (const el of Array.from(gridEl.children)) {
        tileH = Math.max(tileH, (el as HTMLElement).getBoundingClientRect().height);
      }
    }
    if (tileH <= 0) return;
    const gridTop = gridEl.getBoundingClientRect().top;
    const visibleBottom = sc
      ? sc.getBoundingClientRect().top + sc.clientHeight
      : window.innerHeight;
    const RESERVE = 64; // bottom pager + gap + content padding + small slack
    const availH = visibleBottom - gridTop - RESERVE;
    const rows = Math.max(1, Math.floor((availH + gap) / (tileH + gap)));
    fitCount = Math.max(cols, cols * rows);
  }

  function onResize() {
    if (rowsMode === "fit") computeFit();
  }
  onMount(() => {
    loadDefault();
    tick().then(computeFit);
    window.addEventListener("resize", onResize);
  });
  onDestroy(() => window.removeEventListener("resize", onResize));

  // Recompute Fit after the grid (re)renders with data or the mode changes.
  $effect(() => {
    items;
    rowsMode;
    filtered.length;
    if (rowsMode === "fit" && !selected) tick().then(computeFit);
  });

  // ---------- Drill in / back ----------
  function getScrollParent(node: HTMLElement | null): HTMLElement | null {
    let el = node?.parentElement ?? null;
    while (el) {
      const oy = getComputedStyle(el).overflowY;
      // First scrollable ancestor — don't require it to currently overflow (a
      // sparse grid may not yet, but .content is always overflow-y:auto).
      if (oy === "auto" || oy === "scroll") return el;
      el = el.parentElement;
    }
    return null;
  }

  let scrollParentEl: HTMLElement | null = null;
  let savedScroll = 0;

  function drillInto(item: Item, keepScroll = false) {
    scrollParentEl = getScrollParent(rootEl ?? null);
    savedScroll = scrollParentEl?.scrollTop ?? 0;
    selected = item;
    // keepScroll: a nav back-restore re-establishes the captured position via
    // the dashboard shell — don't zero it out from here.
    if (!keepScroll) {
      tick().then(() => {
        if (scrollParentEl) scrollParentEl.scrollTop = 0;
      });
    }
  }
  function back() {
    // If we arrived here via cross-navigation, return to the origin view.
    if (nav.back()) return;
    // Otherwise local back: collapse the detail, restore the grid.
    selected = null;
    tick().then(() => {
      if (rowsMode === "fit") computeFit();
      if (scrollParentEl) scrollParentEl.scrollTop = savedScroll;
    });
  }

  // Establish the view the nav store points us at (a forward cross-nav target
  // OR a back-restore): once items are loaded, open the targeted entity's
  // detail (or the grid if none).
  $effect(() => {
    // Track deps explicitly — nav.target would otherwise only be read (via
    // claimTarget) AFTER the early return, and idOf only inside a nested call.
    nav.target;
    idOf;
    if (items.length === 0) return;
    const t = nav.claimTarget(KIND_SECTION[kind]);
    if (!t) return;
    if (t.entityId != null) {
      const match = items.find((it) => String(idOf(it)) === String(t.entityId));
      // Back-restores carry a scrollTop (the shell re-establishes it); forward
      // drills don't, and should open the detail at the top as usual.
      if (match) drillInto(match, t.scrollTop !== undefined);
      else selected = null;
    } else {
      selected = null;
    }
  });

  // Keep the drilled-in selection pointing at the freshest object as the parent
  // re-polls — so enrichment (images/demographics/favorite) that lands while a
  // detail panel is open shows up live instead of only after re-opening it.
  $effect(() => {
    idOf; // explicit dep — otherwise only read inside the nested find callback
    if (!selected) return;
    const fresh = items.find((it) => String(idOf(it)) === String(idOf(selected)));
    if (fresh && fresh !== selected) selected = fresh;
  });
</script>

{#snippet rowsSelect()}
  <div class="cx-rows" title="Items per page">
    <span>Rows</span>
    <Select
      value={rowsMode}
      options={ROWS_OPTIONS.map((o) => ({ value: o, label: o === "fit" ? "Fit" : o }))}
      onChange={(v) => { rowsMode = v as RowsMode; if (rowsMode === "fit") tick().then(computeFit); }}
      ariaLabel="Items per page"
    />
  </div>
{/snippet}

{#snippet pagerBtns()}
  <div class="cx-pager-btns">
    <button class="cx-pgbtn" disabled={!canPrev} onclick={() => (page = page - 1)} aria-label="Previous page">
      <Icon name="chevron-left" size={14} /> Prev
    </button>
    <button class="cx-pgbtn" disabled={!canNext} onclick={() => (page = page + 1)} aria-label="Next page">
      Next <Icon name="chevron-right" size={14} />
    </button>
  </div>
{/snippet}

<!-- Compact controls that ride on the Sort line (top), only when there's data. -->
{#snippet topControls()}
  {#if total > 0}
    {@render rowsSelect()}
    <span class="cx-range">{rangeStart}-{rangeEnd} of {total}</span>
    {@render pagerBtns()}
  {/if}
{/snippet}

<!-- Full-width pager for the bottom of the grid. -->
{#snippet bottomPager()}
  <div class="cx-pager">
    <div class="cx-pager-left">
      {@render rowsSelect()}
      <span class="cx-range">{rangeStart}-{rangeEnd} of {total}</span>
    </div>
    {@render pagerBtns()}
  </div>
{/snippet}

<div class="cx-browse" bind:this={rootEl}>
  {#if selected}
    <div class="cx-drill">
      <button class="cx-back-btn" onclick={back}>
        <Icon name="chevron-left" size={14} /> {nav.canGoBack ? "Back" : `Back to ${backLabel}`}
      </button>
      <DetailPanel {kind} item={selected} peers={items} onClose={back} />
    </div>
  {:else}
    <div class="cx-browse-top">
      <SearchBar bind:value={search} placeholder={searchPlaceholder} count={filtered.length} />
      {#if chipDefs.length > 0 || sortOptions.length > 0}
        <BrowseFilterBar {filters} set={setFilter} {chipDefs} {sortOptions} onReset={reset} isDefault={isDefaultView} onSetDefault={setAsDefault} rightExtra={topControls} />
      {/if}
    </div>

    {#if loading && items.length === 0}
      <div class="loading">loading...</div>
    {:else if filtered.length === 0}
      <div class="cx-empty">
        <Icon name="search" size={28} color="var(--fg-subtle)" />
        <div class="cx-empty-title">No results</div>
        <div class="cx-empty-hint">Try a different search or clear your filters.</div>
      </div>
    {:else}
      <div class={`cx-grid ${gridClass}`} bind:this={gridEl}>
        {#each pageItems as item (idOf(item))}
          {@render tile(item, false, () => drillInto(item))}
        {/each}
      </div>
      {@render bottomPager()}
    {/if}
  {/if}
</div>

<style>
  .loading {
    text-align: center;
    padding: 64px 0;
    color: var(--fg-muted);
    font-size: 13px;
  }
</style>
