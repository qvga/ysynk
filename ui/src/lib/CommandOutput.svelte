<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { atOutputBottom, outputRetentionNotice, outputText } from './output';

  let {
    output = [],
    truncated = false,
    active = false,
  }: { output?: string[]; truncated?: boolean; active?: boolean } = $props();
  let viewport: HTMLPreElement | undefined = $state();
  let following = $state(true);
  let initialized = false;
  let text = $derived(outputText(output));

  $effect.pre(() => {
    // Depend on contents, not length: a full native ring stays at 200 lines.
    text;
    const view = viewport;
    if (!view || !untrack(() => following) || (initialized && !atOutputBottom(view))) return;
    initialized = true;
    let cancelled = false;
    void tick().then(() => {
      if (!cancelled) view.scrollTop = view.scrollHeight;
    });
    return () => {
      cancelled = true;
    };
  });

  async function follow() {
    following = true;
    await tick();
    if (viewport) {
      viewport.scrollTop = viewport.scrollHeight;
      viewport.focus({ preventScroll: true });
    }
  }
</script>

<div class="command-output">
  {#if active}
    <div class="output-heading">
      <strong>Command output</strong>
      {#if !following}
        <button class="text-button" onclick={follow}>Follow output</button>
      {/if}
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scrollable output must be reachable for keyboard scrolling.) -->
  <pre
    class="output-scroll"
    bind:this={viewport}
    tabindex="0"
    role="region"
    aria-label="Command output"
    onscroll={() => {
      if (viewport) following = atOutputBottom(viewport);
    }}>{text ||
      (active ? 'Waiting for restic output…' : 'No command output retained for this operation.')}</pre>
  {#if truncated}
    <p class="output-retention small muted">{outputRetentionNotice}</p>
  {/if}
  {#if !active}
    <p class="small muted">Output is available for this session only.</p>
  {/if}
</div>
