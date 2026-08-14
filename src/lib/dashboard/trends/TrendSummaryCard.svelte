<!--
  Summary stat card — period total, the two per-day rates, a ▲/▼ delta vs the
  previous equal-length window, and a mini trend line. The line uses the SAME
  buckets as the area chart below (bucketDays at the page's granularity), so it's
  a faithful miniature of that metric's line — not an independent re-bucketing.

  The rate line carries BOTH denominators side by side. Per calendar day is the
  one that responds to the date picker; per active day is the one that stays
  legible when the window is much longer than your history (a 12-month window over
  two months of data reads 0.08 per day but 1.6 per active day — same truth, and
  neither alone tells you what you want). The Active days card can't average
  itself, so it shows the ratio between the two denominators instead, which
  doubles as the key for reading the other three.

  The "vs ..." label is contextual to the selected preset (passed in); all-time
  has no prior window so it shows no comparison. The Cumshots card is `primary`
  (coral icon + coral line), matching the brand-metric treatment.
-->
<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { METRICS, sumMetric, bucketDays, areaValue, type MetricId, type DayPoint } from "./trends-data";
  import { type Granularity } from "$lib/filter-store.svelte";

  type Props = {
    metric: MetricId;
    cur: DayPoint[];
    prev: DayPoint[];
    granularity: Granularity;
    /** Contextual comparison label (e.g. "yesterday", "previous 7 days"); null
     *  for all-time (no prior window → no delta shown). */
    vsLabel: string | null;
    primary?: boolean;
    /** True when the trend chart below is plotting this card's metric — the
     *  card border tints in the metric's chart colour. */
    active?: boolean;
    /** Click → plot this metric on the trend chart below (selection only;
     *  defaults stay with the chart's Default pill). */
    onclick?: () => void;
  };
  let { metric, cur, prev, granularity, vsLabel, primary = false, active = false, onclick }: Props = $props();

  const def = $derived(METRICS[metric]);
  const curVal = $derived(sumMetric(cur, metric));

  // Rates under the total. BOTH readings are shown rather than toggled, because
  // they answer different questions: per calendar day moves with the period you
  // picked, per active day describes what a day you actually used Climax looks
  // like. `cur` is the zero-filled contiguous slice, so its length IS the number
  // of days in the range — not "days that happened to have rows".
  const days = $derived(cur.length);
  // Same definition the Active days card counts (enrich() sets active =
  // sessions > 0), so the two cards can never disagree about the denominator.
  const activeDays = $derived(sumMetric(cur, "active"));
  /** Watch time is milliseconds, so it formats through the metric's own fmt;
   *  counts keep 2dp below 1 (0.08 would round away to 0.1) and 1dp above. */
  function rate(total: number, denom: number): string {
    const v = total / denom;
    if (metric === "watch_time") return METRICS.watch_time.fmt(Math.round(v));
    return v < 1 ? v.toFixed(2) : v.toFixed(1);
  }
  const rateTip = $derived(
    metric === "active"
      ? `You had a session on ${activeDays} of the ${days} days in this period.`
      : `Per day averages over all ${days} days in this period. Per active day averages over the ${activeDays} with a session.`,
  );
  const prevVal = $derived(sumMetric(prev, metric));
  const hasPrev = $derived(prev.length > 0 && prevVal > 0);
  const deltaPct = $derived(hasPrev ? Math.round(((curVal - prevVal) / prevVal) * 100) : null);
  const up = $derived(deltaPct != null && deltaPct >= 0);

  // Same buckets as the area chart → the card line matches the chart line.
  const series = $derived(bucketDays(cur, granularity).map((b) => areaValue(b, metric)));
  const sparkMax = $derived(Math.max(1, ...series));
  const sparkPoints = $derived(
    series.length < 2
      ? ""
      : series.map((v, i) => `${(i / (series.length - 1)) * 100},${28 - (v / sparkMax) * 24 - 2}`).join(" "),
  );
</script>

<button
  class="cx-card cx-tsum"
  class:primary
  class:active
  style={`--tsum-color: ${def.color}`}
  {onclick}
  title={`Plot ${def.label.toLowerCase()} on the trend chart`}
>
  <div class="cx-tsum-head">
    <span class="cx-tsum-icon">
      {#if metric === "cumshots"}
        <Icon name="cumshot" size={15} color={primary ? "var(--accent)" : "var(--fg-muted)"} filled />
      {:else if metric === "watch_time"}
        <Icon name="timer" size={15} color="var(--fg-muted)" />
      {:else if metric === "sessions"}
        <Icon name="list" size={15} color="var(--fg-muted)" />
      {:else}
        <Icon name="flame" size={15} color="var(--fg-muted)" />
      {/if}
    </span>
    <span class="cx-tsum-label">{def.label}</span>
  </div>
  <div class="cx-tsum-value">{def.fmt(curVal)}</div>
  {#if days > 0}
    <div class="cx-tsum-rate" title={rateTip}>
      {#if metric === "active"}
        <b>{Math.round((activeDays / days) * 100)}%</b> of days
      {:else}
        <b>{rate(curVal, days)}</b> per day
        {#if activeDays > 0}
          <span class="cx-tile-sep">·</span>
          <b>{rate(curVal, activeDays)}</b> per active day
        {/if}
      {/if}
    </div>
  {/if}
  <div class="cx-tsum-foot">
    {#if vsLabel === null}
      <span class="cx-tsum-vs">all time</span>
    {:else}
      {#if deltaPct != null}
        <span class="cx-delta {up ? 'up' : 'down'}">{up ? "▲" : "▼"} {Math.abs(deltaPct)}%</span>
      {:else}
        <span class="cx-delta flat">- new</span>
      {/if}
      <span class="cx-tsum-vs">vs {vsLabel}</span>
    {/if}
  </div>
  {#if sparkPoints}
    <svg class="cx-spark" viewBox="0 0 100 28" preserveAspectRatio="none">
      <polyline
        points={sparkPoints}
        fill="none"
        stroke={primary ? "var(--accent)" : "var(--border-strong)"}
        stroke-width="1.5"
        vector-effect="non-scaling-stroke"
        stroke-linejoin="round"
        stroke-linecap="round"
      />
    </svg>
  {/if}
</button>
