<!--
  Primary time-series — an SVG area + line chart with a 4-metric toggle. Distinct
  from the Overview bars (gives Trends its own identity) but skinned to the app:
  coral/white/bone/green per metric (METRICS map), mono uppercase x-axis labels,
  tabular-nums totals. Buckets the daily series by the app's own granularity
  (granularityFor) so day/week/month/year matches the rest of the dashboard.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { bucketDays, areaValue, METRICS, METRIC_ORDER, type MetricId, type DayPoint } from "./trends-data";
  import { type Granularity } from "$lib/filter-store.svelte";
  import MetricDefaultPill from "$lib/dashboard/MetricDefaultPill.svelte";
  import { chartPrefs } from "$lib/chart-prefs.svelte";

  type Props = {
    cur: DayPoint[];
    granularity: Granularity;
    periodLabel: string;
    /** Bindable so the summary cards above can drive the plotted metric —
     *  clicking a card equals clicking its button on this chart's toggle.
     *  Selection only; the saved default stays with the Default pill. */
    metric?: MetricId;
  };
  let { cur, granularity, periodLabel, metric = $bindable<MetricId>("cumshots") }: Props = $props();

  // Seed from this chart's saved default metric (set via the "Default" pill),
  // unless the user already toggled while the load was in flight.
  onMount(() => {
    chartPrefs.load("trends_area").then(() => {
      if (metric === "cumshots") metric = chartPrefs.defaultMetric("trends_area");
    });
  });

  const buckets = $derived(bucketDays(cur, granularity));
  const def = $derived(METRICS[metric]);
  const total = $derived(buckets.reduce((a, b) => a + areaValue(b, metric), 0));
  const max = $derived(Math.max(1, ...buckets.map((b) => areaValue(b, metric))));

  const W = 1000;
  const H = 240;
  const PAD = 6;
  function px(i: number): number {
    return buckets.length <= 1 ? W / 2 : (i / (buckets.length - 1)) * (W - PAD * 2) + PAD;
  }
  function py(v: number): number {
    return H - (v / max) * (H - 20) - 4;
  }
  const linePts = $derived(buckets.map((b, i) => `${px(i)},${py(areaValue(b, metric))}`).join(" "));
  const areaPts = $derived(`${PAD},${H} ${linePts} ${W - PAD},${H}`);
  const labelEvery = $derived(Math.max(1, Math.ceil(buckets.length / 8)));
</script>

<section class="cx-card cx-trend-chart">
  <header class="cx-trend-chart-head">
    <div class="cx-trend-chart-title">
      <h3>{def.label} over time</h3>
      <span class="cx-trend-chart-total">{def.fmt(total)} total · {periodLabel}</span>
    </div>
    <div class="cx-trend-chart-controls">
      <MetricDefaultPill scope="trends_area" current={metric} />
      <div class="cx-toggle">
        {#each METRIC_ORDER as id (id)}
          <button class:active={metric === id} onclick={() => (metric = id)}>{METRICS[id].label}</button>
        {/each}
      </div>
    </div>
  </header>

  {#if buckets.length === 0}
    <div class="cx-area-empty">nothing logged in this range</div>
  {:else}
    <div class="cx-area-wrap">
      <svg class="cx-area" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        <defs>
          <linearGradient id={`cx-aregrad-${metric}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color={def.color} stop-opacity="0.28" />
            <stop offset="100%" stop-color={def.color} stop-opacity="0" />
          </linearGradient>
        </defs>
        {#each [0.25, 0.5, 0.75] as g (g)}
          <line x1="0" y1={H * g} x2={W} y2={H * g} stroke="var(--chart-grid)" stroke-width="1" vector-effect="non-scaling-stroke" stroke-dasharray="3 3" />
        {/each}
        <polygon points={areaPts} fill={`url(#cx-aregrad-${metric})`} />
        <polyline points={linePts} fill="none" stroke={def.color} stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round" stroke-linecap="round" />
      </svg>
      <div class="cx-area-xaxis">
        {#each buckets as b, i (b.key)}
          {#if i % labelEvery === 0 || i === buckets.length - 1}
            <span class="cx-area-xlabel" style={`left: ${buckets.length <= 1 ? 50 : (i / (buckets.length - 1)) * 100}%`}>{b.label}</span>
          {/if}
        {/each}
      </div>
    </div>
  {/if}
</section>
