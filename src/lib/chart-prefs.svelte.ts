// Shared chart preferences — the user's default metric for dashboard chart
// toggles, PER CHART. Each chart (Overview bar chart, Trends area chart, Trends
// rankings) has its own independent default keyed by a `scope` string. A
// reactive singleton; the "Default" pill on a toggle writes its scope's value,
// and the chart reads its scope to seed the initial selection on mount.

import { api } from "$lib/api";

/** "active" (Active days) is only offered by the Trends area chart's 4-metric
 *  toggle — 3-metric call sites (Overview bar chart, Trends rankings) must
 *  narrow it away when seeding from their scope. */
export type ChartMetric = "cumshots" | "watch_time" | "sessions" | "active";

function isChartMetric(v: string): v is ChartMetric {
  return v === "cumshots" || v === "watch_time" || v === "sessions" || v === "active";
}

class ChartPrefs {
  /** scope → default metric. Reassigned (not mutated) so reads stay reactive. */
  private metrics = $state<Record<string, ChartMetric>>({});
  /** scope → in-flight/settled load. Caching the PROMISE (not a done-flag) is
   *  load-bearing: the pill and its host chart both call load() on mount, and
   *  the second caller must wait for the first's fetch — a boolean flag made it
   *  return early and seed the chart from a not-yet-populated record. */
  private loads = new Map<string, Promise<void>>();

  /** The saved default for a scope (falls back to cumshots). */
  defaultMetric(scope: string): ChartMetric {
    return this.metrics[scope] ?? "cumshots";
  }

  /** Read a scope's saved default. All concurrent/later callers share one
   *  fetch; awaiting this guarantees the record is populated. */
  load(scope: string): Promise<void> {
    let p = this.loads.get(scope);
    if (!p) {
      p = (async () => {
        try {
          const raw = await api.defaultChartMetricGet(scope);
          if (isChartMetric(raw)) this.metrics = { ...this.metrics, [scope]: raw };
        } catch (e) {
          console.warn("chart-prefs: load default metric failed", scope, e);
          // Allow a retry on the next mount instead of caching the failure.
          this.loads.delete(scope);
        }
      })();
      this.loads.set(scope, p);
    }
    return p;
  }

  /** Pin a new default metric for a scope (optimistic + persisted). */
  async setDefault(scope: string, m: ChartMetric): Promise<void> {
    this.metrics = { ...this.metrics, [scope]: m };
    // The in-memory record is now authoritative — a later load() must not
    // clobber it with an older DB read.
    this.loads.set(scope, Promise.resolve());
    try {
      await api.defaultChartMetricSet(scope, m);
    } catch (e) {
      console.warn("chart-prefs: save default metric failed", scope, e);
    }
  }
}

export const chartPrefs = new ChartPrefs();
