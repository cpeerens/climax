// Filter store — single source of truth for the dashboard's filter bar.
//
// Design notes:
// - A class with $state / $derived runes is the cleanest "global reactive
//   singleton" pattern in Svelte 5 (vs. raw module-level $state, which
//   has weird re-export semantics).
// - The date preset key is the canonical form; start/end day strings are
//   derived from it. Custom range stores explicit start/end and flips the
//   preset to "custom".
// - Granularity (day vs month) for the bar chart is derived from the
//   range duration. Threshold is 60 days — anything shorter renders
//   per-day, anything longer per-month.

import { api, type NamedEntity, type SceneEntity } from "$lib/api";

export type DatePreset =
  | "today"
  | "yesterday"
  | "this_week"
  | "last_week"
  | "this_month"
  | "last_month"
  | "last_7_days"
  | "last_30_days"
  | "last_90_days"
  | "last_12_months"
  | "this_year"
  | "all_time"
  | "custom";

export type Granularity = "day" | "week" | "month" | "year";

export const PRESET_LABELS: Record<DatePreset, string> = {
  today: "Today",
  yesterday: "Yesterday",
  this_week: "This week",
  last_week: "Last week",
  this_month: "This month",
  last_month: "Last month",
  last_7_days: "Last 7 days",
  last_30_days: "Last 30 days",
  last_90_days: "Last 90 days",
  last_12_months: "Last 12 months",
  this_year: "This year",
  all_time: "All time",
  custom: "Custom range",
};

// Ordered list of presets for menu rendering. "custom" is rendered as a
// separate row at the bottom.
export const PRESET_ORDER: DatePreset[] = [
  "today",
  "yesterday",
  "this_week",
  "last_week",
  "this_month",
  "last_month",
  "last_7_days",
  "last_30_days",
  "last_90_days",
  "last_12_months",
  "this_year",
  "all_time",
];

// ---------- Date math (local time, Mon-Sun week) ----------

function pad(n: number): string {
  return n.toString().padStart(2, "0");
}

/** Format a Date as YYYY-MM-DD using LOCAL components. */
export function fmtDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Days since Monday (0 if Monday, 6 if Sunday). */
function dayFromMonday(d: Date): number {
  return (d.getDay() + 6) % 7;
}

function startOfDay(d: Date): Date {
  const r = new Date(d);
  r.setHours(0, 0, 0, 0);
  return r;
}

function startOfWeek(d: Date): Date {
  const r = startOfDay(d);
  r.setDate(r.getDate() - dayFromMonday(r));
  return r;
}

function endOfWeek(d: Date): Date {
  const r = startOfWeek(d);
  r.setDate(r.getDate() + 6);
  return r;
}

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function endOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth() + 1, 0);
}

function shiftDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}

function shiftMonths(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth() + n, 1);
}

/** Compute YYYY-MM-DD start/end for a preset.
 *
 *  `firstSessionDay` is consulted only by the "all_time" preset so it can
 *  clip its start to actual data instead of padding empty leading bars.
 *  Pass null when not known yet — falls back to start-of-current-month. */
export function resolvePreset(
  preset: DatePreset,
  customStart: string | null,
  customEnd: string | null,
  firstSessionDay: string | null = null,
): { start: string; end: string } {
  const today = startOfDay(new Date());
  switch (preset) {
    case "today":
      return { start: fmtDay(today), end: fmtDay(today) };
    case "yesterday": {
      const y = shiftDays(today, -1);
      return { start: fmtDay(y), end: fmtDay(y) };
    }
    case "this_week":
      return { start: fmtDay(startOfWeek(today)), end: fmtDay(endOfWeek(today)) };
    case "last_week": {
      const lw = shiftDays(today, -7);
      return { start: fmtDay(startOfWeek(lw)), end: fmtDay(endOfWeek(lw)) };
    }
    case "this_month":
      return { start: fmtDay(startOfMonth(today)), end: fmtDay(endOfMonth(today)) };
    case "last_month": {
      const lm = shiftMonths(today, -1);
      return { start: fmtDay(startOfMonth(lm)), end: fmtDay(endOfMonth(lm)) };
    }
    case "last_7_days":
      return { start: fmtDay(shiftDays(today, -6)), end: fmtDay(today) };
    case "last_30_days":
      return { start: fmtDay(shiftDays(today, -29)), end: fmtDay(today) };
    case "last_90_days":
      return { start: fmtDay(shiftDays(today, -89)), end: fmtDay(today) };
    case "last_12_months": {
      // Rolling 12 months: first day of the month 11 months back → today.
      // Gives 12 monthly buckets at month-granularity in the chart.
      const start = new Date(today.getFullYear(), today.getMonth() - 11, 1);
      return { start: fmtDay(start), end: fmtDay(today) };
    }
    case "this_year":
      return { start: `${today.getFullYear()}-01-01`, end: `${today.getFullYear()}-12-31` };
    case "all_time": {
      // Dynamic: start at the earliest data point. Padding empty bars
      // back to a hardcoded 2000-01-01 is noisy and useless. If there's
      // no data yet (fresh install), fall back to the start of the
      // current month — gives a small useful window of (empty) days.
      const start = firstSessionDay ?? fmtDay(startOfMonth(today));
      return { start, end: fmtDay(today) };
    }
    case "custom":
      return {
        start: customStart ?? fmtDay(today),
        end: customEnd ?? fmtDay(today),
      };
  }
}

/** Human-friendly label for a range. "Mon 20 May – Sun 26 May" etc. */
export function rangeFriendly(start: string, end: string): string {
  const s = parseDay(start);
  const e = parseDay(end);
  if (!s || !e) return `${start} → ${end}`;
  const sameDay = start === end;
  const sameMonth = s.getMonth() === e.getMonth() && s.getFullYear() === e.getFullYear();
  const sameYear = s.getFullYear() === e.getFullYear();
  const fmt = (d: Date, opts: Intl.DateTimeFormatOptions) => d.toLocaleDateString([], opts);
  if (sameDay) {
    return fmt(s, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
  }
  if (sameMonth) {
    return `${fmt(s, { weekday: "short", day: "numeric" })} - ${fmt(e, { weekday: "short", day: "numeric", month: "short", year: "numeric" })}`;
  }
  if (sameYear) {
    return `${fmt(s, { weekday: "short", day: "numeric", month: "short" })} - ${fmt(e, { weekday: "short", day: "numeric", month: "short", year: "numeric" })}`;
  }
  return `${fmt(s, { day: "numeric", month: "short", year: "numeric" })} - ${fmt(e, { day: "numeric", month: "short", year: "numeric" })}`;
}

function parseDay(s: string): Date | null {
  const m = s.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return null;
  return new Date(parseInt(m[1], 10), parseInt(m[2], 10) - 1, parseInt(m[3], 10));
}

/** Day count between two YYYY-MM-DD strings, inclusive. */
export function dayCount(start: string, end: string): number {
  const s = parseDay(start);
  const e = parseDay(end);
  if (!s || !e) return 0;
  return Math.round((e.getTime() - s.getTime()) / 86400000) + 1;
}

// ---------- Store class ----------

/** Fallback default until the user's stored preference loads. Falling back
 *  to "last_12_months" matches the pre-Phase-3 behaviour. The actual default
 *  is configurable in Settings and resolves via `loadDefaults()`. */
const DEFAULT_PRESET: DatePreset = "last_12_months";

/** Granularity thresholds for the bar chart. Boundaries chosen so the
 *  chart's x-axis never exceeds ~31 bars, which is the most that fits
 *  comfortably without label overlap:
 *    ≤ 31 days    → daily bars   (max 31 bars; e.g. "This month")
 *    ≤ 217 days   → weekly bars  (max 31 bars = 31 weeks)
 *    ≤ 1095 days  → monthly bars (max ~36 = 3 years)
 *    otherwise    → yearly bars  (one per calendar year)
 *  Tuned so "Last 12 months" / "This year" → month, "All time" → year. */
const DAY_MAX = 31;
const WEEK_MAX = 217;       // 31 weeks
const MONTH_MAX = 1095;     // ~3 years

/** Pick the bar-chart granularity for a [start, end] window using the same
 *  thresholds the store applies to its own range. Exported so the Phase 5
 *  detail-panel chart can map its period toggle to a granularity without
 *  duplicating the boundaries. */
export function granularityFor(start: string, end: string): Granularity {
  const days = dayCount(start, end);
  if (days <= DAY_MAX) return "day";
  if (days <= WEEK_MAX) return "week";
  if (days <= MONTH_MAX) return "month";
  return "year";
}

export class FilterStore {
  /** Persistence scope for this store's default-preset (e.g. "overview",
   *  "sessions"). Each scope remembers its default independently in the
   *  backend so dashboard sections don't share a default. */
  private scope: string;

  constructor(scope = "overview") {
    this.scope = scope;
  }

  preset = $state<DatePreset>(DEFAULT_PRESET);
  customStart = $state<string | null>(null);
  customEnd = $state<string | null>(null);

  scenes = $state<SceneEntity[]>([]);
  performers = $state<NamedEntity[]>([]);
  studios = $state<NamedEntity[]>([]);
  tags = $state<NamedEntity[]>([]);

  /** True once loadDefaults() has resolved — the saved preset + first-session
   *  day are applied. Data effects gate on this so the FIRST fetch never runs
   *  against the fallback window: that briefly rendered wrong-window data
   *  (e.g. 12 months of rows under a "Last 7 days" default with zero
   *  sessions in range). */
  ready = $state(false);
  /** Cached load promise — all callers share one fetch (a done-flag would let
   *  a second caller proceed before the first's fetch resolved; see the
   *  chart-prefs race for the same lesson). */
  private loadPromise: Promise<void> | null = null;
  /** The user's saved default preset. Updated when loaded / changed; the
   *  isDefault check compares against THIS, not the hardcoded fallback,
   *  so the Reset button does what the user expects. */
  userDefaultPreset = $state<DatePreset>(DEFAULT_PRESET);
  /** Earliest assigned_day across the catalog. Used by "all_time" to clip
   *  to actual data. Null until loadDefaults() resolves. */
  firstSessionDay = $state<string | null>(null);

  range = $derived.by(() =>
    resolvePreset(this.preset, this.customStart, this.customEnd, this.firstSessionDay),
  );

  get startDay() { return this.range.start; }
  get endDay() { return this.range.end; }

  granularity = $derived.by<Granularity>(() => {
    const days = dayCount(this.range.start, this.range.end);
    if (days <= DAY_MAX) return "day";
    if (days <= WEEK_MAX) return "week";
    if (days <= MONTH_MAX) return "month";
    return "year";
  });

  rangeLabel = $derived.by(() => rangeFriendly(this.range.start, this.range.end));

  presetLabel = $derived.by(() => PRESET_LABELS[this.preset]);

  isDefault = $derived.by(() =>
    this.preset === this.userDefaultPreset &&
    this.scenes.length === 0 &&
    this.performers.length === 0 &&
    this.studios.length === 0 &&
    this.tags.length === 0,
  );

  setPreset(p: DatePreset) {
    this.preset = p;
    if (p !== "custom") {
      this.customStart = null;
      this.customEnd = null;
    }
  }

  /** Read the user's saved default preset from the backend and apply it as
   *  the current preset (on first call only). Idempotent — concurrent and
   *  later callers share the same promise. Components that use the store
   *  should `await filterStore.loadDefaults()` in onMount (or gate their data
   *  effects on `store.ready`) so fetches run against the user's preferred
   *  window rather than the fallback. */
  loadDefaults(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoadDefaults();
    return this.loadPromise;
  }

  private async doLoadDefaults(): Promise<void> {
    // Default preset (user pref) for this store's scope.
    try {
      const raw = await api.defaultDatePresetGet(this.scope);
      const validKeys = Object.keys(PRESET_LABELS) as DatePreset[];
      if (validKeys.includes(raw as DatePreset)) {
        const preset = raw as DatePreset;
        this.userDefaultPreset = preset;
        // Only OVERRIDE the current preset if the user hasn't already
        // poked the filter bar before this resolves. They typically
        // won't, but defend against the race anyway.
        if (this.preset === DEFAULT_PRESET) this.preset = preset;
      }
    } catch (e) {
      console.warn("filter-store: load default preset failed", e);
    }
    // First-session-day (dynamic "All time" start).
    try {
      this.firstSessionDay = await api.dashboardFirstSessionDay();
    } catch (e) {
      console.warn("filter-store: load first session day failed", e);
    }
    this.ready = true;
  }

  /** Persist a new default preset choice. Called from Settings → Default
   *  date range. Also updates the in-memory `userDefaultPreset` so
   *  isDefault / Reset behave coherently. */
  async setUserDefault(p: DatePreset): Promise<void> {
    try {
      await api.defaultDatePresetSet(this.scope, p);
      this.userDefaultPreset = p;
    } catch (e) {
      console.warn("filter-store: save default preset failed", e);
      throw e;
    }
  }

  setCustomRange(start: string, end: string) {
    this.preset = "custom";
    this.customStart = start;
    this.customEnd = end;
  }

  toggleScene(s: SceneEntity) {
    const idx = this.scenes.findIndex((x) => x.id === s.id);
    if (idx >= 0) this.scenes = this.scenes.filter((_, i) => i !== idx);
    else this.scenes = [...this.scenes, s];
  }
  removeScene(id: number) {
    this.scenes = this.scenes.filter((s) => s.id !== id);
  }

  togglePerformer(p: NamedEntity) {
    const idx = this.performers.findIndex((x) => x.id === p.id);
    if (idx >= 0) this.performers = this.performers.filter((_, i) => i !== idx);
    else this.performers = [...this.performers, p];
  }
  removePerformer(id: string) {
    this.performers = this.performers.filter((p) => p.id !== id);
  }

  toggleStudio(s: NamedEntity) {
    const idx = this.studios.findIndex((x) => x.id === s.id);
    if (idx >= 0) this.studios = this.studios.filter((_, i) => i !== idx);
    else this.studios = [...this.studios, s];
  }
  removeStudio(id: string) {
    this.studios = this.studios.filter((s) => s.id !== id);
  }

  toggleTag(t: NamedEntity) {
    const idx = this.tags.findIndex((x) => x.id === t.id);
    if (idx >= 0) this.tags = this.tags.filter((_, i) => i !== idx);
    else this.tags = [...this.tags, t];
  }
  removeTag(id: string) {
    this.tags = this.tags.filter((t) => t.id !== id);
  }

  /** Backend payload fragments. */
  get sceneIds(): number[] { return this.scenes.map((s) => s.id); }
  get performerIds(): string[] { return this.performers.map((p) => p.id); }
  get studioIds(): string[] { return this.studios.map((s) => s.id); }
  get tagIds(): string[] { return this.tags.map((t) => t.id); }

  reset() {
    this.preset = this.userDefaultPreset;
    this.customStart = null;
    this.customEnd = null;
    this.scenes = [];
    this.performers = [];
    this.studios = [];
    this.tags = [];
  }
}

/** Overview's filter store — date window + entity facets. Keeps the original
 *  "default_date_preset" backend key. */
export const filterStore = new FilterStore("overview");

/** Sessions' INDEPENDENT date filter store. Separate current window + separate
 *  persisted default from Overview's, so each section's date dropdown is its
 *  own. (Sessions only uses the date facet; the entity arrays go unused.) */
export const sessionsFilter = new FilterStore("sessions");

/** Trends' INDEPENDENT date filter store (Phase 6). Drives the whole Trends
 *  page's period via the reused date dropdown; its own persisted default. Like
 *  Sessions, only the date facet is used — the entity arrays go unused (the
 *  leaderboards ARE the entity breakdown). */
export const trendsFilter = new FilterStore("trends");

/** Per-entity-kind detail-view date filters. Each entity drill-down chart (one
 *  scene / performer / studio / tag) shares its KIND's window + persisted default
 *  (`default_date_preset_detail_<kind>`), so setting e.g. "Last 30 days" + a
 *  Default on one performer applies to every performer detail. Only the date
 *  facet is used. */
export const detailSceneFilter = new FilterStore("detail_scene");
export const detailPerformerFilter = new FilterStore("detail_performer");
export const detailStudioFilter = new FilterStore("detail_studio");
export const detailTagFilter = new FilterStore("detail_tag");
