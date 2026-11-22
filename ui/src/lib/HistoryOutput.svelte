<script lang="ts">
  import CommandOutput from './CommandOutput.svelte';

  let { output, truncated }: { output: string[]; truncated: boolean } = $props();
  let open = $state(false);
</script>

<details class="history-output" bind:open>
  <summary>Command output</summary>
  {#if open}
    <CommandOutput {output} {truncated} />
  {/if}
</details>
