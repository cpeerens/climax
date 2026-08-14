<!--
  Reusable typeahead popover for the scene / performer / studio / tag
  chips on the FilterBar. Talks to the filter store directly to add /
  remove selections. Multi-select; staying open on click is intentional
  so the user can select several without re-opening.

  Behaviour:
   - Scene: debounced backend search via catalogSearchScenes (the catalog
     can have thousands of scenes — never fetch all at once).
   - Performer / Studio / Tag: fetch the FULL deduped list once on mount,
     filter client-side. These catalogs are small (hundreds at most).
   - Click an item → toggles in store; checkmark appears.
   - Close handled by parent FilterBar (click-outside / Escape).
-->
<script lang="ts">
  import { filterStore } from "$lib/filter-store.svelte";
  import { api, type NamedEntity, type SceneEntity } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  type Kind = "scene" | "performer" | "studio" | "tag";

  type Props = {
    kind: Kind;
    onClose: () => void;
  };
  let { kind, onClose }: Props = $props();

  let query = $state("");
  let loading = $state(false);
  let listLoaded = $state(false);
  let fullList = $state<NamedEntity[]>([]);
  let sceneResults = $state<SceneEntity[]>([]);
  let inputEl: HTMLInputElement | null = $state(null);

  async function loadFullList() {
    if (kind === "scene") return;
    loading = true;
    try {
      if (kind === "performer") fullList = await api.catalogListPerformers();
      else if (kind === "studio") fullList = await api.catalogListStudios();
      else if (kind === "tag") fullList = await api.catalogListTags();
    } catch (e) {
      console.error(`catalog ${kind} list failed`, e);
    } finally {
      loading = false;
      listLoaded = true;
    }
  }

  let sceneSearchTimer: ReturnType<typeof setTimeout> | null = null;
  async function runSceneSearch(q: string) {
    if (sceneSearchTimer) clearTimeout(sceneSearchTimer);
    if (!q.trim()) {
      sceneResults = [];
      return;
    }
    sceneSearchTimer = setTimeout(async () => {
      loading = true;
      try {
        sceneResults = await api.catalogSearchScenes(q, 30);
      } catch (e) {
        console.error("scene search failed", e);
      } finally {
        loading = false;
      }
    }, 180);
  }

  // Initial load (small catalogs) or focus the input (scenes).
  $effect(() => {
    if (kind === "scene") return;
    if (!listLoaded) loadFullList();
  });
  $effect(() => {
    if (inputEl) inputEl.focus();
  });
  $effect(() => {
    if (kind === "scene") runSceneSearch(query);
  });

  const filteredEntities = $derived.by<NamedEntity[]>(() => {
    if (kind === "scene") return [];
    const q = query.trim().toLowerCase();
    if (!q) return fullList.slice(0, 80);
    return fullList.filter((e) => e.name.toLowerCase().includes(q)).slice(0, 80);
  });

  function isSelected(id: string | number): boolean {
    switch (kind) {
      case "scene": return filterStore.scenes.some((s) => s.id === id);
      case "performer": return filterStore.performers.some((p) => p.id === id);
      case "studio": return filterStore.studios.some((s) => s.id === id);
      case "tag": return filterStore.tags.some((t) => t.id === id);
    }
  }

  function toggle(item: NamedEntity | SceneEntity) {
    switch (kind) {
      case "scene": filterStore.toggleScene(item as SceneEntity); break;
      case "performer": filterStore.togglePerformer(item as NamedEntity); break;
      case "studio": filterStore.toggleStudio(item as NamedEntity); break;
      case "tag": filterStore.toggleTag(item as NamedEntity); break;
    }
  }

  // Currently-selected entities — rendered as an always-visible "Selected" strip
  // so reopening the picker shows what you've chosen (not an empty search), and
  // each can be removed with a click. Held in full by the store (names + thumbs).
  type SelItem = { id: string | number; label: string; thumbnail_url?: string | null };
  const selectedItems = $derived.by<SelItem[]>(() => {
    switch (kind) {
      case "scene":
        return filterStore.scenes.map((s) => ({
          id: s.id,
          label: s.title ?? `Scene ${s.external_id ?? s.id}`,
          thumbnail_url: s.thumbnail_url,
        }));
      case "performer": return filterStore.performers.map((p) => ({ id: p.id, label: p.name }));
      case "studio": return filterStore.studios.map((s) => ({ id: s.id, label: s.name }));
      case "tag": return filterStore.tags.map((t) => ({ id: t.id, label: t.name }));
    }
  });
  function removeSelected(id: string | number) {
    switch (kind) {
      case "scene": filterStore.removeScene(id as number); break;
      case "performer": filterStore.removePerformer(id as string); break;
      case "studio": filterStore.removeStudio(id as string); break;
      case "tag": filterStore.removeTag(id as string); break;
    }
  }
  // Search / browse results, MINUS already-selected (those live in the strip).
  const sceneResultsToAdd = $derived(sceneResults.filter((s) => !isSelected(s.id)));
  const entitiesToAdd = $derived(filteredEntities.filter((e) => !isSelected(e.id)));

  const placeholder = $derived.by(() => {
    switch (kind) {
      case "scene": return "Search scenes...";
      case "performer": return "Search performers...";
      case "studio": return "Search studios...";
      case "tag": return "Search tags...";
    }
  });
</script>

<div class="picker" role="dialog" aria-label="{kind} picker">
  <div class="search">
    <input
      bind:this={inputEl}
      type="search"
      bind:value={query}
      placeholder={placeholder}
      autocomplete="off"
      spellcheck="false"
    />
  </div>

  <div class="results">
    {#if selectedItems.length > 0}
      <div class="sel-head">Selected ({selectedItems.length}) - click to remove</div>
      {#each selectedItems as it (it.id)}
        <button class="result selected" onclick={() => removeSelected(it.id)} title="Remove from filter">
          {#if kind === "scene"}
            {#if it.thumbnail_url}
              <img class="thumb" src={it.thumbnail_url} alt="" loading="lazy" />
            {:else}
              <div class="thumb placeholder"><Icon name="clapperboard" size={12} color="var(--fg-muted)" /></div>
            {/if}
          {/if}
          <span class="result-label">{it.label}</span>
          <span class="check">✓</span>
        </button>
      {/each}
      <div class="sel-divider"></div>
    {/if}

    {#if kind === "scene"}
      {#if !query.trim()}
        <div class="empty">{selectedItems.length > 0 ? "Type to search for more scenes" : "Type to search scenes"}</div>
      {:else if loading}
        <div class="empty">Searching...</div>
      {:else if sceneResultsToAdd.length === 0}
        <div class="empty">No matching scenes</div>
      {:else}
        {#each sceneResultsToAdd as s (s.id)}
          <button class="result scene-result" onclick={() => toggle(s)}>
            {#if s.thumbnail_url}
              <img class="thumb" src={s.thumbnail_url} alt="" loading="lazy" />
            {:else}
              <div class="thumb placeholder">
                <Icon name="clapperboard" size={12} color="var(--fg-muted)" />
              </div>
            {/if}
            <span class="result-label">{s.title ?? `Scene ${s.external_id ?? s.id}`}</span>
          </button>
        {/each}
      {/if}
    {:else}
      {#if loading && !listLoaded}
        <div class="empty">Loading...</div>
      {:else if entitiesToAdd.length === 0}
        <div class="empty">{listLoaded ? (query ? "No matches" : (selectedItems.length > 0 ? `No more ${kind}s to add` : `No ${kind}s in the catalogue yet`)) : "Loading..."}</div>
      {:else}
        {#each entitiesToAdd as e (e.id)}
          <button class="result" onclick={() => toggle(e)}>
            <span class="result-label">{e.name}</span>
          </button>
        {/each}
      {/if}
    {/if}
  </div>

  {#if (kind === "performer" && filterStore.performers.length > 0) ||
       (kind === "studio" && filterStore.studios.length > 0) ||
       (kind === "tag" && filterStore.tags.length > 0) ||
       (kind === "scene" && filterStore.scenes.length > 0)}
    <div class="footer">
      <button
        class="clear-all"
        onclick={() => {
          if (kind === "scene") filterStore.scenes = [];
          else if (kind === "performer") filterStore.performers = [];
          else if (kind === "studio") filterStore.studios = [];
          else if (kind === "tag") filterStore.tags = [];
        }}
      >Clear selected</button>
      <button class="done" onclick={onClose}>Done</button>
    </div>
  {/if}
</div>

<style>
  .picker {
    width: 280px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong, var(--border));
    box-shadow: var(--shadow-md);
    border-radius: var(--radius-md, 8px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    max-height: 420px;
    animation: pop-in 120ms var(--ease-out, ease-out);
  }
  @keyframes pop-in {
    from { transform: translateY(-4px); opacity: 0; }
    to   { transform: none; opacity: 1; }
  }

  .search {
    padding: 8px;
    border-bottom: 1px solid var(--border);
  }
  .search input {
    width: 100%;
    background: var(--bg-input, var(--bg));
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: var(--radius-sm, 6px);
    padding: 6px 9px;
    font-family: inherit;
    font-size: 12px;
  }
  .search input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .results {
    flex: 1;
    overflow-y: auto;
    padding: 4px;
    min-height: 80px;
  }
  .empty {
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
    padding: 20px 12px;
  }
  .sel-head {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-muted);
    font-weight: 600;
    padding: 4px 10px 3px;
  }
  .sel-divider { height: 1px; background: var(--border); margin: 4px 6px; }

  .result {
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    padding: 6px 10px;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 8px;
    transition: background var(--dur-2, 120ms), color var(--dur-2, 120ms);
  }
  .result:hover { background: var(--bg-card-hover); }
  .result.selected {
    background: var(--accent-soft, var(--bg-card-hover));
    color: var(--accent);
  }
  .result-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .check { color: var(--accent); font-weight: 700; font-size: 13px; }

  .result.scene-result {
    align-items: center;
    gap: 8px;
  }
  .thumb {
    width: 40px;
    height: 22px;
    object-fit: cover;
    border-radius: 3px;
    flex-shrink: 0;
    background: var(--bg-card-hover);
  }
  .thumb.placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }

  .footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    gap: 6px;
  }
  .clear-all, .done {
    background: transparent;
    border: 1px solid transparent;
    color: var(--text-muted);
    font-family: inherit;
    font-size: 11px;
    padding: 4px 9px;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    transition: color var(--dur-2, 120ms), background var(--dur-2, 120ms);
  }
  .clear-all:hover { color: var(--danger, var(--accent)); }
  .done {
    color: var(--accent);
    font-weight: 600;
    border-color: var(--border);
  }
  .done:hover { background: var(--accent-soft); }
</style>
