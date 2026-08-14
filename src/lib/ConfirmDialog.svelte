<!--
  Themed confirm dialog. Never use window.confirm() - that renders an OS-native
  popup that breaks the dark theme.
-->
<script lang="ts">
  type Props = {
    title: string;
    message: string;
    confirmLabel?: string;
    cancelLabel?: string;
    variant?: "danger" | "default";
    onConfirm: () => void;
    onCancel: () => void;
    /** Optional third action, rendered set apart on the left (e.g. a "close but
     *  don't commit" escape). Only shown when both label + handler are given. */
    extraLabel?: string;
    onExtra?: () => void;
  };

  let {
    title,
    message,
    confirmLabel = "Confirm",
    cancelLabel = "Cancel",
    variant = "default",
    onConfirm,
    onCancel,
    extraLabel,
    onExtra,
  }: Props = $props();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
<div class="overlay" role="dialog" aria-modal="true" tabindex="-1" onkeydown={(e) => { if (e.key === 'Escape') onCancel(); else if (e.key === 'Enter') onConfirm(); }} onclick={onCancel}>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="dialog" role="document" onclick={(e) => e.stopPropagation()}>
    <h3>{title}</h3>
    <p>{message}</p>
    <div class="actions">
      {#if extraLabel && onExtra}
        <button class="extra" onclick={onExtra}>{extraLabel}</button>
        <span class="spacer"></span>
      {/if}
      <button class="cancel" onclick={onCancel}>{cancelLabel}</button>
      <button class="confirm" class:danger={variant === "danger"} onclick={onConfirm}>{confirmLabel}</button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1400;
    animation: fade-in 140ms ease-out;
  }
  @keyframes fade-in { from { opacity: 0 } to { opacity: 1 } }

  .dialog {
    background: #16181f;
    border: 1px solid #2a2d36;
    border-radius: 12px;
    padding: 22px 26px 18px;
    max-width: 440px;
    width: 92vw;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.55);
    animation: pop-in 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes pop-in {
    from { transform: translateY(6px) scale(0.98); opacity: 0 }
    to { transform: none; opacity: 1 }
  }

  h3 {
    margin: 0 0 8px;
    font-size: 16px;
    font-weight: 600;
    color: #f0f2f5;
  }
  p {
    margin: 0 0 18px;
    font-size: 13px;
    color: #b8bcc4;
    line-height: 1.55;
    white-space: pre-line;
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .spacer { flex: 1 1 auto; }
  .extra:hover { background: #1d1f28; border-color: #3a3e4a; }
  button {
    padding: 8px 16px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    border: 1px solid #2a2d36;
    background: transparent;
    color: #e7e9ed;
    cursor: pointer;
    transition: background 120ms, border-color 120ms;
  }
  .cancel:hover { background: #1d1f28; border-color: #3a3e4a; }
  .confirm {
    background: #2a2d36;
    border-color: #3a3e4a;
  }
  .confirm:hover { background: #353944; }
  .confirm.danger {
    background: #f87171;
    border-color: #f87171;
    color: #fff;
  }
  .confirm.danger:hover { background: #fa8585; border-color: #fa8585; }
</style>
