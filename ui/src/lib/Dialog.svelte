<script lang="ts">
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  let { title, close, children }: { title: string; close: () => void; children: Snippet } = $props();
  let dialog: HTMLDialogElement;
  const id = $props.id();
  function keepFocus(event: KeyboardEvent) {
    if (event.key !== 'Tab') return;
    const controls = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex="0"]')].filter(element => element.getClientRects().length);
    const first = controls[0]; const last = controls.at(-1);
    if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog)) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
  onMount(() => {
    const trigger = document.activeElement as HTMLElement | null;
    dialog.showModal();
    dialog.querySelector<HTMLElement>('input:not([type="checkbox"]), textarea, select')?.focus();
    const revealInvalid = (event: Event) => { const field = event.target as HTMLElement; const details = field.closest('details'); if (details) details.open = true; };
    dialog.addEventListener('invalid', revealInvalid, true);
    return () => { dialog.removeEventListener('invalid', revealInvalid, true); dialog.close(); if (trigger?.isConnected) trigger.focus(); else document.querySelector<HTMLElement>('main .list-toolbar button')?.focus(); };
  });
</script>

<dialog bind:this={dialog} aria-labelledby={id} onkeydown={keepFocus} oncancel={(event) => { event.preventDefault(); close(); }}>
  <div class="dialog-heading"><h2 id={id}>{title}</h2><button type="button" class="quiet" onclick={close} aria-label="Close dialog">Close</button></div>
  {@render children()}
</dialog>
