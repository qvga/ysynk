<script lang="ts">
  import { request } from './client';

  let { jobId }: { jobId: string } = $props();
  type Cancellation = { phase: 'idle' | 'sending' | 'requested' } | { phase: 'failed'; message: string };
  let cancellation = $state<Cancellation>({ phase: 'idle' });
  let pending = $derived(cancellation.phase === 'sending' || cancellation.phase === 'requested');

  async function cancel() {
    if (pending) return;
    cancellation = { phase: 'sending' };
    try {
      await request<null>({ op: 'cancel_job', job_id: jobId });
      cancellation = { phase: 'requested' };
    } catch (cause) {
      cancellation = {
        phase: 'failed',
        message: cause instanceof Error ? cause.message : 'Cancellation could not be sent. Check activity and retry.',
      };
    }
  }
</script>

<div>
  <button onclick={cancel} disabled={pending} aria-live="polite">
    {cancellation.phase === 'sending'
      ? 'Sending cancellation…'
      : cancellation.phase === 'requested'
        ? 'Cancellation requested'
        : 'Cancel operation'}
  </button>
  {#if cancellation.phase === 'failed'}
    <p class="small bad" role="alert">{cancellation.message}</p>
  {/if}
</div>
