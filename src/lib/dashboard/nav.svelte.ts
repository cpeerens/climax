// Dashboard navigation store — active section + a back-stack so cross-navigation
// returns you to where you came from.
//
// Flow: a pill / related-row calls `nav.goto(kind, id, origin)`. `origin` is a
// snapshot of the CURRENT view (where the click happened). goto pushes it on the
// back-stack and points the destination section at the entity to open. When the
// user hits Back, `nav.back()` pops the origin and re-points the section there;
// each section reads its `target` snapshot on activation and restores that view
// (which entity detail / session / day-drill was open).
//
// `target` carries BOTH a forward nav (open entity X) and a back-restore (re-
// establish snapshot Y) — a section claims whichever is addressed to it.

export type Section =
  | "overview"
  | "trends"
  | "sessions"
  | "scenes"
  | "performers"
  | "studios"
  | "tags";

/** The four browse entity kinds. Note singular (kind) vs plural (section). */
export type BrowseKind = "scene" | "performer" | "studio" | "tag";

export const KIND_SECTION: Record<BrowseKind, Section> = {
  scene: "scenes",
  performer: "performers",
  studio: "studios",
  tag: "tags",
};

/** A restorable view. The fields used depend on `section`:
 *  - browse sections: entityKind + entityId (which detail was open; null = grid)
 *  - sessions: sessionId + page (which row was expanded)
 *  - overview: the drill state (level + selected year/month/week/day/session) */
export type ViewSnapshot = {
  section: Section;
  entityKind?: BrowseKind;
  entityId?: string | number | null;
  sessionId?: number | null;
  page?: number;
  /** Forward-nav flag: open this sessionId fresh in the Sessions section
   *  (land on its day so the row is in range), vs a back-restore which keeps
   *  the filter as-left and carries the page it was on. */
  openSession?: boolean;
  overview?: {
    level: "range" | "months" | "days" | "sessions";
    selectedYear: string | null;
    selectedMonth: string | null;
    selectedWeek: string | null;
    selectedDay: string | null;
    selectedSessionId: number | null;
  };
  /** The dashboard .content scrollTop at the moment of navigating away —
   *  re-established on Back. Captured centrally by goto(). */
  scrollTop?: number;
  /** The open SessionDetail's INNER (spine) scrollTop at the moment of
   *  navigating away — restored by the detail via its initialScroll prop.
   *  Captured centrally by goto() when a detail scroller is registered. */
  detailScroll?: number;
};

class NavStore {
  activeSection = $state<Section>("overview");
  /** What the active section should establish — a forward nav target (open an
   *  entity) OR a back-restore snapshot. Claimed by the addressed section. */
  target = $state<ViewSnapshot | null>(null);
  /** Back-stack of origin views. $state so `canGoBack` is reactive. */
  stack = $state<ViewSnapshot[]>([]);
  /** Scroll position the dashboard .content should re-establish after a
   *  navigation: the origin's position after Back, 0 after a sidebar switch.
   *  Consumed by the dashboard shell, which owns the scroll container and
   *  retries until the destination's content is tall enough to honour it. */
  pendingScroll = $state<number | null>(null);
  /** The dashboard's scrolling element, registered by the shell on mount —
   *  lets goto() capture the departure scroll position centrally. */
  private scroller: HTMLElement | null = null;
  /** The currently-open SessionDetail's inner scroller (null when no detail is
   *  open) — lets goto() also capture the within-session position. */
  private detailScroller: HTMLElement | null = null;

  registerScroller(el: HTMLElement | null) {
    this.scroller = el;
  }

  registerDetailScroller(el: HTMLElement | null) {
    this.detailScroller = el;
  }

  get canGoBack(): boolean {
    return this.stack.length > 0;
  }

  /** Plain section switch (sidebar). A fresh navigation — clears the back-stack
   *  and starts the new section at the top (no scroll bleed between sections). */
  setSection(s: Section) {
    this.activeSection = s;
    this.target = null;
    this.stack = [];
    this.pendingScroll = 0;
  }

  /** Push `origin` onto the back-stack, capturing the current scroll
   *  positions (page + open session detail) centrally. */
  private pushOrigin(origin?: ViewSnapshot) {
    if (!origin) return;
    this.stack = [
      ...this.stack,
      {
        ...origin,
        scrollTop: origin.scrollTop ?? this.scroller?.scrollTop ?? 0,
        detailScroll: origin.detailScroll ?? this.detailScroller?.scrollTop,
      },
    ];
  }

  /** Cross-navigate to an entity's detail, remembering `origin` (the view the
   *  click came from) so Back returns there — including its scroll position. */
  goto(kind: BrowseKind, id: string | number, origin?: ViewSnapshot) {
    this.pushOrigin(origin);
    // Forward nav: cancel any in-flight restore (the destination manages its
    // own initial scroll).
    this.pendingScroll = null;
    this.activeSection = KIND_SECTION[kind];
    this.target = { section: KIND_SECTION[kind], entityKind: kind, entityId: id };
  }

  /** Forward-navigate to Overview's day-drill for `day` (YYYY-MM-DD) — used by
   *  the Trends heatmap ("click the cell, see that day's sessions"). Reuses
   *  the same drill-state snapshot Overview's Back-restore already handles. */
  gotoOverviewDay(day: string, origin?: ViewSnapshot) {
    this.pushOrigin(origin);
    this.pendingScroll = 0;
    this.activeSection = "overview";
    this.target = {
      section: "overview",
      overview: {
        level: "sessions",
        selectedYear: null,
        selectedMonth: null,
        selectedWeek: null,
        selectedDay: day,
        selectedSessionId: null,
      },
    };
  }

  /** Forward-navigate to the Sessions section and open a specific session by id
   *  (Overview's SessionDetail "open in Sessions" link). The Sessions section
   *  lands on that session's day so the row is in range, then expands it; Back
   *  returns to the origin (e.g. Overview's day-drill). */
  gotoSession(sessionId: number, origin?: ViewSnapshot) {
    this.pushOrigin(origin);
    this.pendingScroll = 0;
    this.activeSection = "sessions";
    this.target = { section: "sessions", sessionId, openSession: true };
  }

  /** Pop to the previous origin view. Returns true if it navigated back. */
  back(): boolean {
    if (this.stack.length === 0) return false;
    const snap = this.stack[this.stack.length - 1];
    this.stack = this.stack.slice(0, -1);
    this.activeSection = snap.section;
    this.target = snap;
    this.pendingScroll = snap.scrollTop ?? 0;
    return true;
  }

  /** A section claims the target addressed to it (forward drill or back-restore),
   *  clearing it so it fires once. Returns null if the target isn't for it. */
  claimTarget(section: Section): ViewSnapshot | null {
    if (this.target && this.target.section === section) {
      const t = this.target;
      this.target = null;
      return t;
    }
    return null;
  }
}

export const nav = new NavStore();
