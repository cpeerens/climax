<!--
  Climax-themed date + time picker. Replaces the un-themeable native
  <input type="datetime-local">. A trigger field opens a popover with a month
  calendar + a 12-hour time control (HH : MM + AM/PM), all in the app's dark /
  coral / mono language.

  Works in epoch-ms; calls onChange(ms) on each edit, clamped to [min, max] when
  provided (days fully outside the range are disabled; the time is clamped on the
  boundary day). mode="date" hides the time row.
-->
<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import { formatDateOnly, formatTimeOnly } from "$lib/api";

  type Props = {
    value: number; // epoch ms
    min?: number; // epoch ms, inclusive
    max?: number; // epoch ms, inclusive
    mode?: "datetime" | "date";
    disabled?: boolean;
    onChange: (ms: number) => void;
  };
  let { value, min, max, mode = "datetime", disabled = false, onChange }: Props = $props();

  let open = $state(false);
  let wrapEl = $state<HTMLElement | undefined>(undefined);
  let triggerEl = $state<HTMLElement | undefined>(undefined);
  let popEl = $state<HTMLElement | undefined>(undefined);
  // The popover is position:FIXED (viewport-anchored) so it can't be clipped by an
  // ancestor's overflow/scroll - it's used inside scroll containers + modals (the
  // "while you were away" / Untracked rows, SessionDetail). top/left are computed
  // from the trigger's rect, with flip-up when there's no room below.
  let popTop = $state(0);
  let popLeft = $state(0);

  // Calendar view month (local nav state; reset to the value's month on open).
  let viewYear = $state(2026);
  let viewMonth = $state(0); // 0-11
  // Zoom level. Days is the default and the only level the picker used to have,
  // which meant reaching a date a year back was a dozen clicks on the arrow.
  // Click the header label to zoom out, pick to zoom back in.
  let view = $state<"days" | "months" | "years">("days");
  /** Years are shown a dozen at a time, aligned so a block never straddles oddly. */
  const YEAR_BLOCK = 12;
  const yearBlockStart = $derived(Math.floor(viewYear / YEAR_BLOCK) * YEAR_BLOCK);

  function toggle() {
    if (disabled) return;
    if (!open) {
      const d = new Date(value);
      viewYear = d.getFullYear();
      viewMonth = d.getMonth();
      view = "days";
      // Rough initial placement (below the trigger) so the popover doesn't flash at
      // 0,0; the positioning effect refines it (incl. flip-up) after it measures.
      const t = triggerEl?.getBoundingClientRect();
      if (t) { popTop = t.bottom + 8; popLeft = t.left; }
    }
    open = !open;
  }

  // Anchor the fixed popover to the trigger, flipping up when it would overflow the
  // viewport bottom, and clamping into the viewport.
  function reposition() {
    const t = triggerEl?.getBoundingClientRect();
    if (!t || !popEl) return;
    const gap = 8;
    const margin = 8;
    const ph = popEl.offsetHeight;
    const pw = popEl.offsetWidth;
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    let top = t.bottom + gap;
    if (top + ph > vh - margin && t.top - gap - ph >= margin) top = t.top - gap - ph;
    top = Math.max(margin, Math.min(top, vh - ph - margin));
    const left = Math.max(margin, Math.min(t.left, vw - pw - margin));
    popTop = top;
    popLeft = left;
  }

  const display = $derived(
    mode === "date"
      ? `${formatDateOnly(value)}, ${new Date(value).getFullYear()}`
      : `${formatDateOnly(value)} · ${formatTimeOnly(value)}`,
  );

  // Selected parts (from the live value).
  const sel = $derived.by(() => {
    const d = new Date(value);
    return { y: d.getFullYear(), m: d.getMonth(), day: d.getDate(), h: d.getHours(), min: d.getMinutes() };
  });
  const hour12 = $derived(sel.h % 12 === 0 ? 12 : sel.h % 12);
  const isPM = $derived(sel.h >= 12);

  const WEEKDAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
  const MONTHS = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
  ];

  type Cell = { day: number; ms: number; disabled: boolean } | null;
  const cells = $derived.by<Cell[]>(() => {
    const first = new Date(viewYear, viewMonth, 1);
    const startCol = (first.getDay() + 6) % 7; // 0 = Monday
    const daysInMonth = new Date(viewYear, viewMonth + 1, 0).getDate();
    const out: Cell[] = [];
    for (let i = 0; i < startCol; i++) out.push(null);
    for (let day = 1; day <= daysInMonth; day++) {
      // Picking a day keeps the current time-of-day.
      const ms = new Date(viewYear, viewMonth, day, sel.h, sel.min, 0, 0).getTime();
      const dayStart = new Date(viewYear, viewMonth, day, 0, 0, 0, 0).getTime();
      const dayEnd = new Date(viewYear, viewMonth, day, 23, 59, 59, 999).getTime();
      const off = (min != null && dayEnd < min) || (max != null && dayStart > max);
      out.push({ day, ms, disabled: off });
    }
    return out;
  });

  const today = (() => {
    const n = new Date();
    return { y: n.getFullYear(), m: n.getMonth(), day: n.getDate() };
  })();

  function clamp(ms: number): number {
    if (min != null && ms < min) return min;
    if (max != null && ms > max) return max;
    return ms;
  }
  function commit(ms: number) {
    onChange(clamp(ms));
  }

  function prevMonth() {
    if (viewMonth === 0) { viewMonth = 11; viewYear -= 1; } else viewMonth -= 1;
  }
  function nextMonth() {
    if (viewMonth === 11) { viewMonth = 0; viewYear += 1; } else viewMonth += 1;
  }

  // A whole month/year is only unreachable when it lies ENTIRELY outside the
  // bounds - a partly-in-range month must stay pickable so you can get to its
  // valid days.
  function monthOff(y: number, m: number): boolean {
    const start = new Date(y, m, 1, 0, 0, 0, 0).getTime();
    const end = new Date(y, m + 1, 0, 23, 59, 59, 999).getTime();
    return (min != null && end < min) || (max != null && start > max);
  }
  function yearOff(y: number): boolean {
    const start = new Date(y, 0, 1, 0, 0, 0, 0).getTime();
    const end = new Date(y, 11, 31, 23, 59, 59, 999).getTime();
    return (min != null && end < min) || (max != null && start > max);
  }

  /** Header arrows step by whatever the current zoom level is. */
  function stepBack() {
    if (view === "days") prevMonth();
    else if (view === "months") viewYear -= 1;
    else viewYear -= YEAR_BLOCK;
  }
  function stepForward() {
    if (view === "days") nextMonth();
    else if (view === "months") viewYear += 1;
    else viewYear += YEAR_BLOCK;
  }
  /** Header label zooms OUT one level; years is the top. */
  function zoomOut() {
    if (view === "days") view = "months";
    else if (view === "months") view = "years";
  }
  function pickMonth(m: number) {
    if (monthOff(viewYear, m)) return;
    viewMonth = m;
    view = "days";
  }
  function pickYear(y: number) {
    if (yearOff(y)) return;
    viewYear = y;
    view = "months";
  }
  function pickDay(c: Cell) {
    if (!c || c.disabled) return;
    commit(c.ms);
  }

  // ----- time edits (operate on the live value, keep the date) -----
  function withTime(h24: number, m: number): number {
    const d = new Date(value);
    d.setHours(h24, m, 0, 0);
    return d.getTime();
  }
  function setHour12(h: number) {
    const c = Math.min(12, Math.max(1, h));
    const h24 = (c % 12) + (isPM ? 12 : 0);
    commit(withTime(h24, sel.min));
  }
  function setMinute(m: number) {
    commit(withTime(sel.h, ((m % 60) + 60) % 60));
  }
  function stepHour(delta: number) {
    const d = new Date(value);
    d.setHours(sel.h + delta, sel.min, 0, 0);
    commit(d.getTime());
  }
  function stepMinute(delta: number) {
    const d = new Date(value);
    d.setMinutes(sel.min + delta, 0, 0);
    commit(d.getTime());
  }
  function setAmPm(pm: boolean) {
    if (pm === isPM) return;
    commit(withTime(pm ? sel.h + 12 : sel.h - 12, sel.min));
  }

  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (wrapEl && !wrapEl.contains(e.target as Node)) open = false;
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") open = false; };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  });

  // Keep the fixed popover glued to the trigger while open (the modal/section can
  // scroll under it; capture-phase catches inner scrollers). Re-runs when popEl
  // mounts, so the first measure happens right after open.
  $effect(() => {
    if (!open || !popEl) return;
    reposition();
    const on = () => reposition();
    window.addEventListener("scroll", on, true);
    window.addEventListener("resize", on);
    return () => {
      window.removeEventListener("scroll", on, true);
      window.removeEventListener("resize", on);
    };
  });
</script>

<div class="dtp" bind:this={wrapEl}>
  <button type="button" class="dtp-trigger" class:open {disabled} onclick={toggle} bind:this={triggerEl}>
    <Icon name="calendar" size={13} />
    <span class="dtp-value">{display}</span>
  </button>

  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dtp-pop" bind:this={popEl} style="top: {popTop}px; left: {popLeft}px;" onmousedown={(e) => e.stopPropagation()}>
      <div class="dtp-cal-head">
        <button type="button" class="dtp-nav" onclick={stepBack}
          aria-label={view === "days" ? "Previous month" : view === "months" ? "Previous year" : "Previous years"}>
          <Icon name="chevron-left" size={16} />
        </button>
        {#if view === "years"}
          <span class="dtp-month">{yearBlockStart} - {yearBlockStart + YEAR_BLOCK - 1}</span>
        {:else}
          <button type="button" class="dtp-month dtp-zoom" onclick={zoomOut}
            title={view === "days" ? "Pick a month" : "Pick a year"}>
            {view === "days" ? `${MONTHS[viewMonth]} ${viewYear}` : viewYear}
          </button>
        {/if}
        <button type="button" class="dtp-nav" onclick={stepForward}
          aria-label={view === "days" ? "Next month" : view === "months" ? "Next year" : "Next years"}>
          <Icon name="chevron-right" size={16} />
        </button>
      </div>

      {#if view === "days"}
        <div class="dtp-grid dtp-weekdays">
          {#each WEEKDAYS as w}<span class="dtp-wd">{w}</span>{/each}
        </div>
        <div class="dtp-grid">
          {#each cells as c}
            {#if c == null}
              <span class="dtp-cell empty"></span>
            {:else}
              <button
                type="button"
                class="dtp-cell"
                class:selected={c.day === sel.day && viewMonth === sel.m && viewYear === sel.y}
                class:today={c.day === today.day && viewMonth === today.m && viewYear === today.y}
                disabled={c.disabled}
                onclick={() => pickDay(c)}
              >{c.day}</button>
            {/if}
          {/each}
        </div>
      {:else if view === "months"}
        <div class="dtp-zgrid">
          {#each MONTHS as name, m (name)}
            <button
              type="button"
              class="dtp-cell dtp-zcell"
              class:selected={m === sel.m && viewYear === sel.y}
              class:today={m === today.m && viewYear === today.y}
              disabled={monthOff(viewYear, m)}
              onclick={() => pickMonth(m)}
            >{name.slice(0, 3)}</button>
          {/each}
        </div>
      {:else}
        <div class="dtp-zgrid">
          {#each Array.from({ length: YEAR_BLOCK }, (_, i) => yearBlockStart + i) as y (y)}
            <button
              type="button"
              class="dtp-cell dtp-zcell"
              class:selected={y === sel.y}
              class:today={y === today.y}
              disabled={yearOff(y)}
              onclick={() => pickYear(y)}
            >{y}</button>
          {/each}
        </div>
      {/if}

      {#if mode === "datetime"}
        <div class="dtp-time">
          <div class="dtp-spin">
            <button type="button" class="dtp-step" onclick={() => stepHour(1)} aria-label="Hour up"><Icon name="chevron-up" size={13} /></button>
            <input
              class="dtp-num"
              type="text"
              inputmode="numeric"
              value={String(hour12)}
              onchange={(e) => { const n = parseInt(e.currentTarget.value, 10); if (!Number.isNaN(n)) setHour12(n); }}
            />
            <button type="button" class="dtp-step" onclick={() => stepHour(-1)} aria-label="Hour down"><Icon name="chevron-down" size={13} /></button>
          </div>
          <span class="dtp-colon">:</span>
          <div class="dtp-spin">
            <button type="button" class="dtp-step" onclick={() => stepMinute(1)} aria-label="Minute up"><Icon name="chevron-up" size={13} /></button>
            <input
              class="dtp-num"
              type="text"
              inputmode="numeric"
              value={String(sel.min).padStart(2, "0")}
              onchange={(e) => { const n = parseInt(e.currentTarget.value, 10); if (!Number.isNaN(n)) setMinute(n); }}
            />
            <button type="button" class="dtp-step" onclick={() => stepMinute(-1)} aria-label="Minute down"><Icon name="chevron-down" size={13} /></button>
          </div>
          <div class="dtp-ampm">
            <button type="button" class:on={!isPM} onclick={() => setAmPm(false)}>AM</button>
            <button type="button" class:on={isPM} onclick={() => setAmPm(true)}>PM</button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .dtp { position: relative; display: inline-flex; }

  .dtp-trigger {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 7px 11px;
    background: var(--bg-input, #101218);
    border: 1px solid var(--border, #252934);
    border-radius: var(--radius-sm, 6px);
    color: var(--fg, #eceef3);
    font-family: var(--font-mono, monospace);
    font-size: 13px;
    cursor: pointer;
    transition: border-color 120ms, background 120ms;
  }
  .dtp-trigger :global(svg) { color: var(--fg-muted, #8a909e); flex-shrink: 0; }
  .dtp-trigger:hover:not(:disabled) { border-color: var(--border-strong, #353a47); }
  .dtp-trigger.open { border-color: var(--accent, #ef6b7a); }
  .dtp-trigger:disabled { opacity: 0.5; cursor: default; }
  .dtp-value { white-space: nowrap; }

  .dtp-pop {
    position: fixed;
    z-index: 1000;
    width: max-content;
    padding: 12px;
    background: var(--bg-card, #15171e);
    border: 1px solid var(--border-strong, #353a47);
    border-radius: 12px;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.55);
    animation: dtp-pop 140ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  @keyframes dtp-pop {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: none; }
  }

  .dtp-cal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .dtp-month {
    font-size: 12px;
    font-weight: 600;
    color: var(--fg-strong, #f7f8fa);
    letter-spacing: 0.01em;
  }
  /* The header doubles as the zoom-out control, so it has to read as pressable
     without turning into a button-looking chip. */
  .dtp-zoom {
    background: transparent;
    border: none;
    padding: 3px 8px;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
    transition: background var(--dur-2, 120ms);
  }
  .dtp-zoom:hover { background: var(--bg-card-hover); }

  /* Fixed to the day grid's width (7 x 30px + 6 x 2px gap = 222px) so the
     popover does not resize as you zoom between levels. Four rows of 44px
     matches six rows of 30px closely enough that nothing jumps either. */
  .dtp-zgrid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 2px;
    width: 222px;
  }
  .dtp-zcell { height: 44px; }

  .dtp-nav {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid var(--border, #252934);
    border-radius: 7px;
    color: var(--fg-muted, #8a909e);
    cursor: pointer;
    transition: background 120ms, border-color 120ms, color 120ms;
  }
  .dtp-nav:hover { background: var(--bg-card-hover, #1b1e26); color: var(--fg, #eceef3); border-color: var(--border-strong, #353a47); }

  .dtp-grid {
    display: grid;
    grid-template-columns: repeat(7, 30px);
    gap: 2px;
  }
  .dtp-weekdays { margin-bottom: 2px; }
  .dtp-wd {
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-subtle, #5c6273);
  }
  .dtp-cell {
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: 7px;
    color: var(--fg, #eceef3);
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .dtp-cell.empty { cursor: default; }
  .dtp-cell:not(.empty):not(:disabled):hover { background: var(--bg-card-hover, #1b1e26); }
  .dtp-cell.today:not(.selected) {
    box-shadow: inset 0 0 0 1px var(--border-strong, #353a47);
    color: var(--fg-strong, #f7f8fa);
  }
  .dtp-cell.selected {
    background: var(--accent, #ef6b7a);
    color: #fff;
    font-weight: 600;
  }
  .dtp-cell:disabled { color: var(--ink-600, #353a47); cursor: default; }

  .dtp-time {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border-subtle, #1c1f26);
  }
  .dtp-spin {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .dtp-step {
    width: 30px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: 5px;
    color: var(--fg-muted, #8a909e);
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .dtp-step:hover { background: var(--bg-card-hover, #1b1e26); color: var(--accent, #ef6b7a); }
  .dtp-num {
    width: 38px;
    height: 30px;
    text-align: center;
    background: var(--bg-input, #101218);
    border: 1px solid var(--border, #252934);
    border-radius: 7px;
    color: var(--fg-strong, #f7f8fa);
    font-family: var(--font-mono, monospace);
    font-size: 15px;
    font-variant-numeric: tabular-nums;
    transition: border-color 120ms;
  }
  .dtp-num:focus { outline: none; border-color: var(--accent, #ef6b7a); }
  .dtp-colon {
    font-family: var(--font-mono, monospace);
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-muted, #8a909e);
    padding-bottom: 2px;
  }
  .dtp-ampm {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-left: 4px;
  }
  .dtp-ampm button {
    padding: 3px 8px;
    background: transparent;
    border: 1px solid var(--border, #252934);
    border-radius: 6px;
    color: var(--fg-muted, #8a909e);
    font-family: inherit;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    cursor: pointer;
    transition: background 120ms, border-color 120ms, color 120ms;
  }
  .dtp-ampm button:hover:not(.on) { border-color: var(--border-strong, #353a47); color: var(--fg, #eceef3); }
  .dtp-ampm button.on {
    background: var(--accent, #ef6b7a);
    border-color: var(--accent, #ef6b7a);
    color: #fff;
  }
</style>
