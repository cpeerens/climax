<!--
  Column pattern chart — "Busiest days" (by day of week) and "Peak hours" (by
  hour of day). Presentational: the parent owns the metric toggle (rendered via
  the `controls` snippet) and passes the matching data / subtitle / colour.
  Column colour follows --pc-color (the metric's chart colour); the peak column
  renders solid. Values sit on top of each bar; hover gives the full phrase.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  // `tip` overrides the label in the hover tooltip only (the axis keeps `label`).
  // Peak hours uses it to show the full "4pm" on hover while the axis stays bare.
  type Datum = { label: string; value: number; tip?: string };
  type Props = {
    title: string;
    subtitle?: string;
    /** Period label (e.g. "Last 90 days") appended after the subtitle, in the
     *  same mono style as the area chart's "· <period>" total line. */
    period?: string;
    data: Datum[];
    /** Hover-tooltip formatter for the value (full phrase). */
    formatVal?: (v: number) => string;
    /** Compact formatter for the value shown on top of each bar. */
    formatShort?: (v: number) => string;
    /** Show the value on top of each bar (default true). */
    showValues?: boolean;
    /** Render the x-axis label only every Nth column (default 1 = all). */
    labelStride?: number;
    /** Metric chart colour for the columns (defaults to coral). */
    color?: string;
    /** Header-right controls (metric toggle + Default pill). */
    controls?: Snippet;
  };
  let {
    title,
    subtitle,
    period,
    data,
    formatVal,
    formatShort,
    showValues = true,
    labelStride = 1,
    color,
    controls,
  }: Props = $props();

  const max = $derived(Math.max(1, ...data.map((d) => d.value)));
  const maxIdx = $derived(data.reduce((mi, d, i, arr) => (d.value > arr[mi].value ? i : mi), 0));
</script>

<section class="cx-card cx-pattern" style={color ? `--pc-color: ${color}` : undefined}>
  <header class="cx-pattern-head">
    <div class="cx-pattern-titles">
      <h3>{title}</h3>
      {#if subtitle || period}
        <span class="cx-pattern-sub">
          {subtitle}{#if period}{subtitle ? " " : ""}<span class="cx-pattern-period">· {period}</span>{/if}
        </span>
      {/if}
    </div>
    {#if controls}{@render controls()}{/if}
  </header>
  <div class="cx-pattern-bars">
    {#each data as d, i (d.label + i)}
      <div class="cx-pcol" title={`${d.tip ?? d.label}: ${formatVal ? formatVal(d.value) : d.value}`}>
        <span class="cx-pcol-val" class:hidden={!showValues || d.value === 0}>
          {formatShort ? formatShort(d.value) : d.value}
        </span>
        <div class="cx-pcol-track">
          <div
            class="cx-pcol-bar"
            class:peak={i === maxIdx && d.value > 0}
            style={`height: ${Math.max(3, (d.value / max) * 100)}%`}
          ></div>
        </div>
        <span class="cx-pcol-label" class:hidden={i % labelStride !== 0}>{d.label}</span>
      </div>
    {/each}
  </div>
</section>
