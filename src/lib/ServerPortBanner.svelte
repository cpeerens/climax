<script lang="ts">
  /**
   * Says so when the app's own backend couldn't bind its port.
   *
   * That failure used to be invisible: boot logged it and carried on, so the app
   * opened looking completely normal and the only symptom was the bridge never
   * connecting - nothing tracked, nothing on screen explaining why. Any fixed
   * port can be swallowed (another program, or a system-reserved range), so this
   * is a real state to design for, not a theoretical one.
   *
   * Renders nothing in the healthy case, in client mode (an external server owns
   * the port, and this app binds nothing), and in a browser (no local listener to
   * report on).
   */
  import { onMount } from "svelte";
  import { api } from "$lib/api";
  import { isTauri } from "$lib/transport";
  import Icon from "$lib/Icon.svelte";

  interface Props {
    /** Where to send the user to fix it. Omit for no button - the message still
     *  shows, which is the point. */
    onFix?: () => void;
    fixLabel?: string;
  }
  let { onFix, fixLabel = "Open Settings" }: Props = $props();

  let message = $state<string | null>(null);

  onMount(async () => {
    if (!isTauri()) return;
    try {
      const info = await api.serverPortGet();
      if (!info.ok) message = info.error;
    } catch (e) {
      // If we can't even ask, stay quiet: an unexplained warning is worse than
      // none, and a failure here means something larger is already wrong.
      console.error("read port status failed", e);
    }
  });
</script>

{#if message}
  <div class="port-banner" role="alert">
    <Icon name="alert-triangle" size={15} />
    <span class="msg">{message}</span>
    {#if onFix}
      <button class="fix" onclick={onFix}>{fixLabel}</button>
    {/if}
  </div>
{/if}

<style>
  .port-banner {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid rgba(242, 107, 107, 0.25);
    background: rgba(242, 107, 107, 0.07);
    color: var(--danger, #f26b6b);
    border-radius: var(--radius-md, 8px);
    font-size: 12px;
    line-height: 1.4;
  }
  .msg { flex: 1; }
  .fix {
    flex-shrink: 0;
    padding: 3px 8px;
    border: 1px solid rgba(242, 107, 107, 0.35);
    border-radius: var(--radius-sm, 6px);
    background: transparent;
    color: var(--danger, #f26b6b);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .fix:hover { background: rgba(242, 107, 107, 0.12); }
</style>
