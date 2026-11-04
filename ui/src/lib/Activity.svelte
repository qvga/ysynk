<script lang="ts">
  import type { Job, Repository } from './types';
  import { bytes, date } from './model';
  import CommandOutput from './CommandOutput.svelte';
  let { jobs, repositories }: { jobs: Job[]; repositories: Repository[] } = $props();
  const label = (id: string) => repositories.find(repo => repo.id === id)?.label ?? id;
</script>

{#if jobs.length === 0}
  <p class="muted small">No completed operations.</p>
{:else}
  <div class="activity-list">
    {#each jobs as job (job.id)}
      <article class="activity-row">
        <div class="row-main"><strong>{job.label || job.kind}</strong><span class:bad={['failed', 'partial', 'interrupted'].includes(job.state)} class="badge">{job.state}</span><span class="muted small">{date(job.started_at)}</span></div>
        {#if job.message}<p class="muted">{job.message}</p>{/if}
        {#if job.progress && job.state === 'running'}
          <p class="small muted">{bytes(job.progress.bytes_done)}{job.progress.total_bytes > 0 ? ` of ${bytes(job.progress.total_bytes)}` : ''} · {job.progress.files_done.toLocaleString()} files</p>
        {/if}
        {#if job.destinations.length}
          <ul class="destination-results">{#each job.destinations as destination}<li><strong>{label(destination.repository_id)}</strong><span class:bad={destination.state !== 'success' && destination.state !== 'running'}>{destination.state}</span><span class="muted">{destination.message}</span>{#if destination.snapshot_id}<code>{destination.snapshot_id.slice(0, 12)}</code>{/if}</li>{/each}</ul>
        {/if}
        <details class="history-output"><summary>Command output</summary><CommandOutput output={job.output} truncated={job.output_truncated}/></details>
      </article>
    {/each}
  </div>
{/if}
