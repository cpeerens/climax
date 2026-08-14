<!--
  Climax dropdown. Replaces the native <select>, which on macOS renders as a
  system POPUP BUTTON: the OS draws the open menu ITSELF, centred over the
  control with the selected row under the cursor, rather than dropping below.
  That is the platform convention, so no amount of CSS reaches it - the closed
  state can be styled, the open menu cannot. Windows happens to drop downward,
  which is why this never looked wrong before the macOS port.

  Visual language matches FilterMenu (the browse filter chips) so every picker
  in the app behaves the same way on every platform.

  The menu is position: FIXED against the trigger's viewport rect, not absolute
  - these live inside the Settings modal and inside scrolling panes, and an
  absolutely positioned menu gets clipped by an ancestor's overflow. Same fix
  the DateTimePicker needed (see CLAUDE.md).
-->
<script lang="ts">
  type Option = { value: string; label: string };
  type Props = {
    value: string;
    options: Option[];
    onChange: (v: string) => void;
    /** Stretch to the container. Settings rows want this; toolbar chips do not. */
    fullWidth?: boolean;
    disabled?: boolean;
    ariaLabel?: string;
    /** Lets a <label for=...> associate with the trigger; a button is a
     *  labelable element, so this is real association. */
    id?: string;
  };
  let { value, options, onChange, fullWidth = false, disabled = false, ariaLabel, id }: Props = $props();

  let open = $state(false);
  let trigger = $state<HTMLButtonElement | undefined>();
  let menuEl = $state<HTMLDivElement | undefined>();
  let pos = $state({ top: 0, left: 0, width: 0, flipped: false });
  /** Keyboard cursor. Seeded to the current value each time the menu opens. */
  let activeIndex = $state(0);

  const selected = $derived(options.find((o) => o.value === value));
  const MAX_MENU = 320;

  function reposition() {
    const el = trigger;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 8;
    const wanted = Math.min(MAX_MENU, options.length * 30 + 8);
    // Flip up only when there is genuinely more room above, so a menu near the
    // bottom of a short window still lands somewhere it can be read.
    const flipped = below < wanted && r.top - 8 > below;
    pos = {
      top: flipped ? Math.max(8, r.top - 4 - wanted) : r.bottom + 4,
      left: r.left,
      width: r.width,
      flipped,
    };
  }

  function openMenu() {
    if (disabled) return;
    activeIndex = Math.max(0, options.findIndex((o) => o.value === value));
    reposition();
    open = true;
  }

  function choose(v: string) {
    open = false;
    if (v !== value) onChange(v);
    trigger?.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        openMenu();
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      open = false;
      trigger?.focus();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      activeIndex = (activeIndex + 1) % options.length;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      activeIndex = (activeIndex - 1 + options.length) % options.length;
    } else if (e.key === "Home") {
      e.preventDefault();
      activeIndex = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      activeIndex = options.length - 1;
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const opt = options[activeIndex];
      if (opt) choose(opt.value);
    }
  }

  // Keep the menu glued to its trigger while the page moves underneath. Capture
  // phase so it also fires for scrolls inside nested panes, not just the window.
  $effect(() => {
    if (!open) return;
    const handler = () => reposition();
    window.addEventListener("scroll", handler, true);
    window.addEventListener("resize", handler);
    return () => {
      window.removeEventListener("scroll", handler, true);
      window.removeEventListener("resize", handler);
    };
  });

  // Keep the highlighted row in view when arrowing through a long list.
  $effect(() => {
    if (!open || !menuEl) return;
    const row = menuEl.querySelectorAll<HTMLElement>("[data-opt]")[activeIndex];
    row?.scrollIntoView({ block: "nearest" });
  });
</script>

<button
  bind:this={trigger}
  {id}
  type="button"
  class="cx-sel-trigger"
  class:full={fullWidth}
  class:is-open={open}
  {disabled}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-label={ariaLabel}
  onclick={() => (open ? (open = false) : openMenu())}
  onkeydown={onKeydown}
>
  <span class="cx-sel-label">{selected?.label ?? ""}</span>
  <svg class="cx-sel-chev" width="12" height="12" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
    aria-hidden="true"><polyline points="6 9 12 15 18 9" /></svg>
</button>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="cx-sel-scrim" onmousedown={() => (open = false)}></div>
  <div
    bind:this={menuEl}
    class="cx-sel-menu"
    role="listbox"
    tabindex="-1"
    aria-label={ariaLabel}
    style="top:{pos.top}px; left:{pos.left}px; min-width:{pos.width}px; max-height:{MAX_MENU}px;"
    onkeydown={onKeydown}
  >
    {#each options as opt, i (opt.value)}
      <button
        data-opt
        type="button"
        role="option"
        aria-selected={opt.value === value}
        class="cx-sel-opt"
        class:cursor={i === activeIndex}
        class:chosen={opt.value === value}
        onmouseenter={() => (activeIndex = i)}
        onclick={() => choose(opt.value)}
      >{opt.label}</button>
    {/each}
  </div>
{/if}

<style>
  .cx-sel-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--bg-input, var(--bg-elevated));
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 6px);
    color: var(--fg);
    font-family: inherit;
    font-size: 13px;
    padding: 8px 10px 8px 12px;
    min-width: 150px;
    cursor: pointer;
    text-align: left;
    transition: border-color var(--dur-2, 120ms);
  }
  .cx-sel-trigger.full { width: 100%; }
  .cx-sel-trigger:hover:not(:disabled) { border-color: var(--border-strong); }
  .cx-sel-trigger:focus-visible,
  .cx-sel-trigger.is-open { outline: none; border-color: var(--accent); }
  .cx-sel-trigger:disabled { opacity: 0.5; cursor: default; }

  .cx-sel-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cx-sel-chev { flex-shrink: 0; color: var(--fg-muted); }

  .cx-sel-scrim {
    position: fixed;
    inset: 0;
    z-index: 1500;
    background: transparent;
  }
  /* Geometry, type and motion below are copied from DateRangePicker's .picker
     / .preset so every dropdown in the app is visually identical. If that one
     changes, change this to match. */
  .cx-sel-menu {
    position: fixed;
    /* Above the scrim, and above the Settings modal it can be opened from. */
    z-index: 1501;
    overflow-y: auto;
    /* Pin the cross axis: overflow-y:auto computes overflow-x to auto too, and
       a sub-pixel-wide row would surface a stray horizontal scrollbar. */
    overflow-x: hidden;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong, var(--border));
    border-radius: var(--radius-md, 8px);
    box-shadow: var(--shadow-md);
    padding: 4px;
    display: flex;
    flex-direction: column;
    animation: cx-sel-pop 120ms var(--ease-out, ease-out);
  }
  @keyframes cx-sel-pop {
    from { transform: translateY(-4px); opacity: 0; }
    to   { transform: none; opacity: 1; }
  }
  .cx-sel-opt {
    width: 100%;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm, 6px);
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    padding: 7px 10px;
    text-align: left;
    cursor: pointer;
    transition: background var(--dur-2, 120ms), color var(--dur-2, 120ms);
  }
  /* Pointer hover and the keyboard cursor are the same affordance. */
  .cx-sel-opt.cursor { background: var(--bg-card-hover); }
  /* Selected wins over the cursor, exactly as .preset.active does. */
  .cx-sel-opt.chosen {
    color: var(--accent);
    background: var(--accent-soft, var(--bg-card-hover));
    font-weight: 600;
  }
</style>
