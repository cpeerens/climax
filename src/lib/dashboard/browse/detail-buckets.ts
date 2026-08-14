// Detail-panel time-series helper: format a RangeBucket into a short x-axis
// label. The detail chart reuses the existing `dashboard_filtered_buckets`
// command + `BarChart`, driven by the same FilterStore / DateRangePicker as the
// Overview chart — so the date window + granularity come from the store; this
// file only labels the resulting bucket keys.

import { type RangeBucket } from "$lib/api";

const WEEKDAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
const MONTHS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/** Short x-axis label for a bucket, keyed off its granularity. `weekday` shows
 *  weekday names at day granularity (for a ~week-wide window); otherwise it shows
 *  day-of-month numbers. */
export function detailLabel(b: RangeBucket, weekday: boolean): string {
  if (b.granularity === "year") return b.key; // "2024"
  if (b.granularity === "month") {
    const m = parseInt(b.key.split("-")[1], 10);
    return MONTHS[(m - 1 + 12) % 12];
  }
  // day (and the unlikely week fallback) — key is YYYY-MM-DD.
  const [y, m, d] = b.key.split("-").map((s) => parseInt(s, 10));
  if (b.granularity === "week") return `${d}/${m}`;
  if (weekday) return WEEKDAYS[new Date(y, m - 1, d).getDay()];
  return String(d);
}
