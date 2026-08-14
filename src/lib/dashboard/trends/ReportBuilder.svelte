<!--
  Build report — pick metric × group-by × range → live horizontal-bar breakdown
  + totals + native CSV export. The Group-by options gate on the selected metric
  (the honesty matrix in trends-data.dimsForMetric): Watch time can't be grouped
  by Time of day. No silent proxies.

  Entity group-bys (scene/performer/studio/tag) grow a third, searchable
  selector: "All" ranks the period-active entities (the classic breakdown);
  picking ONE entity drills the report into that entity's TIME SERIES at the
  app's granularity for the range (via dashboard_filtered_buckets — the same
  chip-aware path as the Overview filters, so numbers match the browse charts).
  Active days never offers entity group-bys (ranking entities by active days is
  meaningless), per the honesty matrix.

  Time groupings (day/week/month/dow) come from the daily series; Time of day
  from the hour histogram; entity groupings from the range-scoped breakdowns.
  Date-bucketed rows render newest-first (screen and CSV share the order).
-->
<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import Select from "$lib/Select.svelte";
  import Icon from "$lib/Icon.svelte";
  import { api, type HourHistogram, type EntityBreakdownRow, type RangeBucket } from "$lib/api";
  import { granularityFor } from "$lib/filter-store.svelte";
  import { isTauri } from "$lib/transport";
  import ReportEntityPicker from "./ReportEntityPicker.svelte";
  import {
    METRICS,
    METRIC_ORDER,
    REPORT_DIMS,
    dimsForMetric,
    isEntityDim,
    reportTimeRows,
    reportHourRows,
    reportEntityRows,
    reportSeriesRows,
    granDimLabel,
    type MetricId,
    type ReportDim,
    type DayPoint,
    type ReportRow,
  } from "./trends-data";

  type Props = {
    cur: DayPoint[];
    hist: HourHistogram | null;
    sceneRows: EntityBreakdownRow[];
    perfRows: EntityBreakdownRow[];
    studioRows: EntityBreakdownRow[];
    tagRows: EntityBreakdownRow[];
    /** The page's date window (YYYY-MM-DD), for the entity-drill series fetch. */
    range: { start: string; end: string };
  };
  let { cur, hist, sceneRows, perfRows, studioRows, tagRows, range }: Props = $props();

  let metric = $state<MetricId>("cumshots");
  let dim = $state<ReportDim>("week");

  const availableDims = $derived(dimsForMetric(metric));

  // Snap an invalid grouping to "week" when the metric change invalidates it.
  $effect(() => {
    if (!availableDims.includes(dim)) dim = "week";
  });

  const def = $derived(METRICS[metric]);
  const dimLabel = $derived(REPORT_DIMS.find((d) => d.id === dim)?.label ?? "");

  // ---- Entity drill (third selector on entity group-bys) ----
  let entity = $state<string>("all");
  // Selection belongs to a dimension — switching Group-by resets it.
  $effect(() => {
    dim;
    entity = "all";
  });

  const entityRowsFor = $derived.by<EntityBreakdownRow[]>(() => {
    switch (dim) {
      case "scene": return sceneRows;
      case "performer": return perfRows;
      case "studio": return studioRows;
      case "tag": return tagRows;
      default: return [];
    }
  });
  // Picker options: only entities ACTIVE in the selected period, alphabetical.
  const entityOptions = $derived(
    [...entityRowsFor].map((r) => ({ id: r.id, name: r.name })).sort((a, b) => a.name.localeCompare(b.name)),
  );
  // Range change can drop the selected entity out of the period → back to All.
  $effect(() => {
    if (entity !== "all" && !entityOptions.some((o) => o.id === entity)) entity = "all";
  });
  const entityActive = $derived(isEntityDim(dim) && entity !== "all");
  const entityName = $derived(entityOptions.find((o) => o.id === entity)?.name ?? "");
  const seriesGran = $derived(granularityFor(range.start, range.end));

  // Specific entity → fetch its filtered time buckets at the range's display
  // granularity. Buckets carry all three metrics, so switching the metric
  // re-derives rows without a refetch. (Active days never reaches here — its
  // entity group-bys aren't offered.)
  let seriesBuckets = $state<RangeBucket[]>([]);
  let seriesLoading = $state(false);
  let seriesToken = 0;
  $effect(() => {
    if (!entityActive) return;
    const d = dim;
    const ent = entity;
    const start = range.start;
    const end = range.end;
    const token = ++seriesToken;
    (async () => {
      seriesLoading = true;
      try {
        const result = await api.dashboardFilteredBuckets({
          startDay: start,
          endDay: end,
          granularity: granularityFor(start, end),
          ...(d === "scene" ? { sceneContentItemIds: [Number(ent)] }
            : d === "performer" ? { performerIds: [ent] }
            : d === "studio" ? { studioIds: [ent] }
            : { tagIds: [ent] }),
        });
        if (token !== seriesToken) return;
        seriesBuckets = result;
      } catch (e) {
        if (token === seriesToken) console.error("report entity series failed", e);
      } finally {
        if (token === seriesToken) seriesLoading = false;
      }
    })();
  });

  const rows = $derived.by<ReportRow[]>(() => {
    if (isEntityDim(dim)) {
      if (entity !== "all") return reportSeriesRows(seriesBuckets, metric);
      return reportEntityRows(entityRowsFor, metric);
    }
    if (dim === "hour") return hist ? reportHourRows(hist, metric) : [];
    return reportTimeRows(cur, dim, metric);
  });

  const max = $derived(Math.max(1, ...rows.map((r) => r.value)));
  const total = $derived(rows.reduce((a, r) => a + r.value, 0));
  /** With a specific entity, the first CSV column is the time bucket. */
  const csvDimLabel = $derived(entityActive ? granDimLabel(seriesGran) : dimLabel);

  // ---- Breakdown height: grow with content, cap at the viewport ----
  // The list grows naturally until it would push past the visible page; only
  // then does ITS scrollbar appear (the outer page never scrolls in report
  // mode). Cap = space from the list's top to the scroll container's visible
  // bottom, minus the card/content bottom paddings — measured, so it's "the
  // maximum comfortable size" at any window shape, 16:9 included.
  let chartEl = $state<HTMLDivElement | undefined>();
  let chartMaxH = $state(460); // provisional until measured

  function scrollParent(node: HTMLElement | null): HTMLElement | null {
    let el = node?.parentElement ?? null;
    while (el) {
      const oy = getComputedStyle(el).overflowY;
      if (oy === "auto" || oy === "scroll") return el;
      el = el.parentElement;
    }
    return null;
  }

  // Reserve below the list: result-card bottom padding (16) + card border +
  // .content bottom padding (24) + small slack.
  const RESERVE = 44;
  const MIN_H = 180;

  function computeMaxH() {
    if (!chartEl) return;
    const sc = scrollParent(chartEl);
    const visibleBottom = sc
      ? sc.getBoundingClientRect().top + sc.clientHeight
      : window.innerHeight;
    chartMaxH = Math.max(MIN_H, Math.floor(visibleBottom - chartEl.getBoundingClientRect().top - RESERVE));
  }

  onMount(() => {
    tick().then(computeMaxH);
    window.addEventListener("resize", computeMaxH);
  });
  onDestroy(() => window.removeEventListener("resize", computeMaxH));

  function slug(s: string): string {
    return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  }

  let exporting = $state(false);
  async function exportCsv() {
    if (rows.length === 0 || exporting) return;
    exporting = true;
    // Watch time exports as raw seconds — say so in the header so the column
    // is self-describing in Excel. Counts (cumshots/sessions/days) are
    // self-evident.
    const header = `${csvDimLabel},${def.label}${metric === "watch_time" ? " (seconds)" : ""}\n`;
    const body = rows
      .map((r) => {
        // Watch time exported in seconds (CSV convention).
        const v = metric === "watch_time" ? Math.round(r.value / 1000) : r.value;
        // Machine label when the row has one (ISO dates); display label else.
        const raw = r.csvLabel ?? r.label;
        const label = /[",\n]/.test(raw) ? `"${raw.replace(/"/g, '""')}"` : raw;
        return `${label},${v}`;
      })
      .join("\n");
    const csv = header + body + "\n";
    const name = entityActive
      ? `climax-${slug(def.label)}-for-${slug(entityName) || dim}.csv`
      : `climax-${slug(def.label)}-by-${dim}.csv`;
    try {
      if (isTauri()) {
        // Desktop: native save dialog via the trends_export_csv command.
        await api.trendsExportCsv(csv, name);
      } else {
        // Browser web client: no native dialog - a plain browser download.
        // In-document click + deferred revoke: revoking synchronously can
        // abort the download in stricter browsers.
        const blob = new Blob([csv], { type: "text/csv" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = name;
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 10_000);
      }
    } catch (e) {
      console.error("csv export failed", e);
    } finally {
      exporting = false;
    }
  }
</script>

<div class="cx-report">
  <section class="cx-card cx-report-controls">
    <div class="cx-report-field">
      <label for="rb-metric">Metric</label>
      <Select
        id="rb-metric"
        value={metric}
        options={METRIC_ORDER.map((id) => ({ value: id, label: METRICS[id].label }))}
        onChange={(v) => (metric = v as MetricId)}
        ariaLabel="Metric"
      />
    </div>
    <span class="cx-report-by">by</span>
    <div class="cx-report-field">
      <label for="rb-dim">Group by</label>
      <Select
        id="rb-dim"
        value={dim}
        options={REPORT_DIMS.filter((d) => availableDims.includes(d.id)).map((d) => ({ value: d.id, label: d.label }))}
        onChange={(v) => (dim = v as ReportDim)}
        ariaLabel="Group by"
      />
    </div>
    {#if isEntityDim(dim)}
      <div class="cx-report-field">
        <label for="rb-entity">Which {dimLabel.toLowerCase()}</label>
        <ReportEntityPicker
          label={dimLabel}
          options={entityOptions}
          value={entity}
          onChange={(id) => (entity = id)}
        />
      </div>
    {/if}
    <button class="cx-btn cx-btn-primary cx-report-export" onclick={exportCsv} disabled={rows.length === 0 || exporting}>
      <Icon name="external-link" size={14} /> Export CSV
    </button>
  </section>

  <section class="cx-card cx-report-result">
    <header class="cx-report-result-head">
      <h3>
        {#if entityActive}
          {def.label} for {entityName} <span class="cx-report-gran">· by {granDimLabel(seriesGran).toLowerCase()}</span>
        {:else}
          {def.label} by {dimLabel.toLowerCase()}
        {/if}
      </h3>
      <span class="cx-report-result-total">{def.fmt(total)} total · {rows.length} row{rows.length === 1 ? "" : "s"}</span>
    </header>
    {#if entityActive && seriesLoading && rows.length === 0}
      <div class="cx-report-empty">loading...</div>
    {:else if rows.length === 0}
      <div class="cx-report-empty">nothing logged for this combination</div>
    {:else}
      <div class="cx-report-chart" bind:this={chartEl} style={`max-height: ${chartMaxH}px`}>
        {#each rows as r, i (r.label + i)}
          <div class="cx-report-bar-row" title={`${r.label}: ${def.fmt(r.value)}`}>
            <span class="cx-report-bar-label">{r.label}</span>
            <div class="cx-report-bar-track">
              <div class="cx-report-bar metric-{metric}" style={`width: ${(r.value / max) * 100}%`}></div>
            </div>
            <span class="cx-report-bar-value">{def.fmt(r.value)}</span>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>
