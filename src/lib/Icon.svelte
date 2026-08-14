<!--
  Climax icon system. Lucide-style line icons + a custom three-droplet
  Cumshot glyph + the brand mark.

  All icons are 24×24 viewBox, 1.75px stroke, no fills (except Cumshot
  which is filled when used as a content marker). They inherit
  `currentColor` so set the colour on the parent or via the `color` prop.

  Usage:
    <Icon name="bar-chart-3" size={16} />
    <Icon name="cumshot" size={11} color="var(--accent)" />
    <Icon name="climax-glyph" size={14} color="var(--accent)" strokeWidth={2.5} />

  Names map 1:1 to the design system mapping table.
-->
<script lang="ts">
  type IconName =
    | "bar-chart-3" | "trending-up" | "list" | "clapperboard" | "user"
    | "video" | "tag" | "calendar" | "settings" | "arrow-left-to-line"
    | "trophy" | "award" | "flame" | "alert-triangle" | "hourglass" | "timer"
    | "external-link" | "plus" | "minus" | "pause" | "play" | "square"
    | "chevron-down" | "chevron-right" | "chevron-up" | "chevron-left" | "x" | "edit"
    | "refresh-cw" | "trash-2" | "search" | "check" | "heart"
    | "inbox" | "brain" | "plug" | "server" | "database-backup" | "download"
    | "climax-glyph" | "cumshot";

  type Props = {
    name: IconName;
    size?: number;
    color?: string;
    strokeWidth?: number;
    /** When true, fills with currentColor instead of stroking. Only meaningful
     *  for icons that have a sensible fill rendering (currently `cumshot`). */
    filled?: boolean;
  };

  let { name, size = 18, color, strokeWidth = 1.75, filled = false }: Props = $props();
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill={filled ? "currentColor" : "none"}
  stroke={filled ? "none" : (color ?? "currentColor")}
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  style="color: {color ?? 'currentColor'}; flex-shrink: 0;"
  aria-hidden="true"
>
  {#if name === "bar-chart-3"}
    <rect x="3" y="12" width="4" height="9" />
    <rect x="10" y="6" width="4" height="15" />
    <rect x="17" y="9" width="4" height="12" />
  {:else if name === "trending-up"}
    <polyline points="22 7 13.5 15.5 8.5 10.5 2 17" />
    <polyline points="16 7 22 7 22 13" />
  {:else if name === "list"}
    <line x1="8" y1="6" x2="21" y2="6" />
    <line x1="8" y1="12" x2="21" y2="12" />
    <line x1="8" y1="18" x2="21" y2="18" />
    <line x1="3" y1="6" x2="3.01" y2="6" />
    <line x1="3" y1="12" x2="3.01" y2="12" />
    <line x1="3" y1="18" x2="3.01" y2="18" />
  {:else if name === "clapperboard"}
    <rect x="2" y="4" width="20" height="16" rx="2" />
    <path d="M2 8h20M6 4l2 4M12 4l2 4M18 4l2 4" />
  {:else if name === "user"}
    <path d="M20 21a8 8 0 1 0-16 0" />
    <circle cx="12" cy="7" r="4" />
  {:else if name === "video"}
    <path d="m16 13 5.2 3a2 2 0 0 0 2.8-1.7V8.7A2 2 0 0 0 21.2 7L16 10" />
    <rect x="2" y="6" width="14" height="12" rx="2" />
  {:else if name === "tag"}
    <!-- Flat horizontal label, no eyelet/string. Centred in the 24x24 viewBox
         so it visually aligns with the other chip icons rather than sitting
         high-left like Lucide's default tag glyph does. -->
    <path d="M5 5 H14 L20 12 L14 19 H5 a2 2 0 0 1 -2 -2 V7 a2 2 0 0 1 2 -2 Z" />
  {:else if name === "calendar"}
    <rect x="3" y="4" width="18" height="18" rx="2" />
    <path d="M16 2v4M8 2v4M3 10h18" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09A1.65 1.65 0 0 0 15 4.6a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9c.34.13.6.39.74.74.14.36.16.75.06 1.12V11a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
  {:else if name === "arrow-left-to-line"}
    <path d="M3 19V5" />
    <path d="m13 6-6 6 6 6" />
    <path d="M7 12h14" />
  {:else if name === "trophy"}
    <path d="M6 9H4.5a2.5 2.5 0 0 1 0-5H6" />
    <path d="M18 9h1.5a2.5 2.5 0 0 0 0-5H18" />
    <path d="M4 22h16M10 14.66V17a1 1 0 0 1-1 1H6m12 4h-2a1 1 0 0 1-1-1v-2.34M18 2H6v7a6 6 0 0 0 12 0V2Z" />
  {:else if name === "award"}
    <!-- Lucide "award": a rosette. Sits beside the trophy as the other
         milestone marker without repeating it. -->
    <circle cx="12" cy="8" r="6" />
    <path d="M15.477 12.89 17 22l-5-3-5 3 1.523-9.11" />
  {:else if name === "flame"}
    <path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z" />
  {:else if name === "alert-triangle"}
    <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
    <line x1="12" y1="9" x2="12" y2="13" />
    <circle cx="12" cy="17" r=".5" />
  {:else if name === "hourglass"}
    <path d="M5 22h14" />
    <path d="M5 2h14" />
    <path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22" />
    <path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2" />
  {:else if name === "timer"}
    <line x1="10" y1="2" x2="14" y2="2" />
    <line x1="12" y1="14" x2="15" y2="11" />
    <circle cx="12" cy="14" r="8" />
  {:else if name === "external-link"}
    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
    <polyline points="15 3 21 3 21 9" />
    <line x1="10" y1="14" x2="21" y2="3" />
  {:else if name === "plus"}
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  {:else if name === "minus"}
    <line x1="5" y1="12" x2="19" y2="12" />
  {:else if name === "pause"}
    <rect x="6" y="4" width="4" height="16" />
    <rect x="14" y="4" width="4" height="16" />
  {:else if name === "play"}
    <polygon points="6 3 20 12 6 21 6 3" />
  {:else if name === "square"}
    <rect x="5" y="5" width="14" height="14" rx="1" />
  {:else if name === "chevron-down"}
    <polyline points="6 9 12 15 18 9" />
  {:else if name === "chevron-right"}
    <polyline points="9 6 15 12 9 18" />
  {:else if name === "chevron-up"}
    <polyline points="18 15 12 9 6 15" />
  {:else if name === "chevron-left"}
    <polyline points="15 18 9 12 15 6" />
  {:else if name === "x"}
    <line x1="18" y1="6" x2="6" y2="18" />
    <line x1="6" y1="6" x2="18" y2="18" />
  {:else if name === "edit"}
    <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
    <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
  {:else if name === "refresh-cw"}
    <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
  {:else if name === "download"}
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="7 10 12 15 17 10" />
    <line x1="12" y1="15" x2="12" y2="3" />
  {:else if name === "trash-2"}
    <polyline points="3 6 5 6 21 6" />
    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    <line x1="10" y1="11" x2="10" y2="17" />
    <line x1="14" y1="11" x2="14" y2="17" />
  {:else if name === "search"}
    <circle cx="11" cy="11" r="7" />
    <line x1="20" y1="20" x2="16.65" y2="16.65" />
  {:else if name === "check"}
    <polyline points="4 13 9 18 20 6" />
  {:else if name === "heart"}
    <!-- Used as a state marker (favorite); pass filled to fill it. -->
    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" />
  {:else if name === "inbox"}
    <!-- Settings rail: Untracked sessions. -->
    <path d="M22 12h-6l-2 3h-4l-2-3H2" />
    <path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11Z" />
  {:else if name === "brain"}
    <!-- Settings rail: Smart tracking. -->
    <path d="M12 5a3 3 0 0 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z" />
    <path d="M12 5a3 3 0 0 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z" />
    <path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4" />
  {:else if name === "plug"}
    <!-- Settings rail: Stash configuration. -->
    <path d="M12 22v-5" />
    <path d="M9 8V2M15 8V2" />
    <path d="M18 8H6v3a6 6 0 0 0 12 0V8Z" />
  {:else if name === "server"}
    <!-- Settings rail: Server / connection (Climax backend). -->
    <rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
    <rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
    <line x1="6" y1="6" x2="6.01" y2="6" />
    <line x1="6" y1="18" x2="6.01" y2="18" />
  {:else if name === "climax-glyph"}
    <!-- brand mark: chart line + heart at peak (simplified from logo). -->
    <polyline points="3 19 9 13 13 17 18 12" />
    <path d="M16.5 10c-1.2-1.2-1.2-3.1 0-4.3.6-.6 1.6-.6 2.1 0l.4.4.4-.4c.6-.6 1.6-.6 2.1 0 .6.6.6 1.6 0 2.1L19 10 16.5 10Z" />
  {:else if name === "cumshot"}
    <!-- custom three-droplet glyph: large droplet + two smaller. Reads at
         10-24px. Filled or stroked depending on `filled` prop. -->
    <path d="M14 2c-2 2.5-3.5 4.5-3.5 6.5a3.5 3.5 0 0 0 7 0C17.5 6.5 16 4.5 14 2Z" />
    <path d="M7 9c-1.4 1.8-2.5 3.4-2.5 4.8a2.8 2.8 0 0 0 5.5 0c0-1.4-1-3-2.5-4.8Z" />
    <path d="M19 13c-1.2 1.6-2.2 3-2.2 4.2a2.5 2.5 0 0 0 5 0c0-1.2-1-2.6-2.2-4.2Z" />
  {:else if name === "database-backup"}
    <!-- Settings rail: Backup and restore. -->
    <ellipse cx="12" cy="5" rx="9" ry="3" />
    <path d="M3 5V19A9 3 0 0 0 15 21.84" />
    <path d="M21 5V8" />
    <path d="M21 12L18 17h4l-3 5" />
    <path d="M3 12A9 3 0 0 0 14.59 14.87" />
  {/if}
</svg>
