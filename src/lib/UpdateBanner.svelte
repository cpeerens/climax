<script lang="ts">
  /**
   * Says when a newer Climax has been released.
   *
   * Stash, which this otherwise follows closely, has no equivalent: it writes one
   * line to a startup log and adds a note to a settings page you have to go
   * looking for. So in practice nobody finds out. The check itself is identical
   * either way - the result is already fetched and cached - so showing it costs
   * nothing on the wire.
   *
   * Dismissal is remembered PER RELEASE, which is what keeps this from becoming
   * a nag: wave away 0.2.0 and it stays quiet until 0.3.0 exists.
   *
   * Renders nothing when up to date, when the release has been dismissed, or
   * before the first load resolves.
   */
  import { updateCheck } from "$lib/update-check.svelte";
  import Icon from "$lib/Icon.svelte";

  interface Props {
    /** Opens the release page. The host supplies this because opening an
     *  external link differs between the desktop app and a browser tab. */
    onView: (url: string) => void;
    /** Opens Settings on the version page, for the details. */
    onDetails?: () => void;
  }
  let { onView, onDetails }: Props = $props();
</script>

{#if updateCheck.unseen && updateCheck.latest}
  {@const release = updateCheck.latest}
  <div class="update-banner">
    <Icon name="download" size={15} />
    <!-- The rule: "v" appears only where a version is a VALUE - the mono column
         in Settings and the sidebar version line. In a sentence or on a button
         it reads as a part number, so prose and labels go without. -->
    <span class="msg">
      Climax {release.version} is out. You are on {updateCheck.myVersion}.
    </span>
    <button class="link" onclick={() => onView(release.url)}>View the release</button>
    {#if onDetails}
      <button class="link" onclick={onDetails}>Details</button>
    {/if}
    <button
      class="dismiss"
      onclick={() => updateCheck.dismiss()}
      title="Hide this until a newer version is released"
      aria-label="Hide this until a newer version is released"
    >
      <Icon name="x" size={13} />
    </button>
  </div>
{/if}

<style>
  .update-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    background: var(--bg-card);
    color: var(--fg);
    border-radius: var(--radius-md, 8px);
    font-size: 12px;
    line-height: 1.4;
  }
  /* Coral marks it as Climax's own news. Deliberately NOT the danger treatment
     the port-bind banner uses - nothing is broken here. */
  .update-banner :global(svg) { color: var(--accent); flex-shrink: 0; }
  .msg { flex: 1; min-width: 0; }
  .link {
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover { color: var(--accent-hover, #f58895); }
  .dismiss {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 3px;
    border: none;
    background: none;
    color: var(--fg-subtle);
    cursor: pointer;
    border-radius: var(--radius-sm, 6px);
  }
  .dismiss:hover { color: var(--fg); background: var(--bg-card-hover); }
</style>
