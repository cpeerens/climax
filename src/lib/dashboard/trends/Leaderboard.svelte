<!--
  Top performers / studios / tags — horizontal bar list (range-scoped). Rows are
  clickable: they cross-navigate to the entity's browse detail (Back returns to
  Trends). Bar colour follows the shared ranking metric (cumshots → coral,
  watch time → white, sessions → bone). When items carry a `delta`, a sports-
  table movement badge renders next to the rank (▲N up / ▼N down / – no change /
  "new" = unranked last period).
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  // `rank` is competition-style and supplied by the parent, so entities tied on
  // the metric share it (two tied at the top are both #1, the next is #3). It is
  // deliberately NOT the row's index, which would break every tie arbitrarily.
  type Item = { id: string; name: string; value: number; rank: number; delta?: number | "new" };
  type Props = {
    title: string;
    items: Item[];
    metric: "cumshots" | "watch_time" | "sessions";
    formatVal: (v: number) => string;
    onSelect: (id: string) => void;
    /** Optional controls on the right of the title (e.g. the performers'
     *  female/male toggle). */
    headRight?: Snippet;
  };
  let { title, items, metric, formatVal, onSelect, headRight }: Props = $props();

  const max = $derived(Math.max(1, ...items.map((i) => i.value)));
  const hasDeltas = $derived(items.some((i) => i.delta !== undefined));

  function deltaTitle(d: number | "new"): string {
    if (d === "new") return "Not in the rankings in the previous period.";
    if (d === 0) return "Same rank as the previous period.";
    const places = Math.abs(d);
    return `Moved ${d > 0 ? "up" : "down"} ${places} place${places === 1 ? "" : "s"} since the previous period.`;
  }
</script>

<section class="cx-card cx-leaderboard">
  <header class="cx-lb-head">
    <h3>{title}</h3>
    {#if headRight}{@render headRight()}{/if}
  </header>
  {#if items.length === 0}
    <div class="cx-lb-empty">nothing logged in this range</div>
  {:else}
    <div class="cx-lb-list">
      {#each items as it (it.id)}
        <button class="cx-lb-row" class:with-move={hasDeltas} onclick={() => onSelect(it.id)} title={it.name}>
          <span class="cx-lb-rank" class:is-top={it.rank === 1}>{it.rank}</span>
          {#if hasDeltas}
            {#if it.delta === "new"}
              <span class="cx-lb-move fresh" title={deltaTitle("new")}>new</span>
            {:else if typeof it.delta === "number" && it.delta > 0}
              <span class="cx-lb-move up" title={deltaTitle(it.delta)}>▲{it.delta}</span>
            {:else if typeof it.delta === "number" && it.delta < 0}
              <span class="cx-lb-move down" title={deltaTitle(it.delta)}>▼{Math.abs(it.delta)}</span>
            {:else}
              <span class="cx-lb-move" title={deltaTitle(0)}>-</span>
            {/if}
          {/if}
          <span class="cx-lb-name">{it.name}</span>
          <div class="cx-lb-track">
            <div class="cx-lb-bar metric-{metric}" style={`width: ${(it.value / max) * 100}%`}></div>
          </div>
          <span class="cx-lb-value">{formatVal(it.value)}</span>
        </button>
      {/each}
    </div>
  {/if}
</section>
