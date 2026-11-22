<script lang="ts">
  import type { Config, Job } from './types';
  import { date, jobTitle } from './model';
  import HistoryOutput from './HistoryOutput.svelte';
  let { jobs, config }: { jobs: Job[]; config: Pick<Config, 'repositories' | 'profiles'> } = $props();
  const label = (id: string) => config.repositories.find((repo) => repo.id === id)?.label ?? id;
</script>

{#if jobs.length === 0}
  <p class="muted small">No completed operations.</p>
{:else}
  <div class="activity-list">
    {#each jobs as job (job.id)}
      <article class="activity-row">
        <div class="row-main">
          <strong>{jobTitle(job, config)}</strong>
          <span
            class:bad={['failed', 'partial', 'interrupted'].includes(job.state)}
            class:neutral={job.state === 'cancelled'}
            class="badge"
          >
            {job.state}
          </span>
          <span class="muted small">{date(job.started_at)}</span>
        </div>
        {#if job.message}
          <p class="muted">{job.message}</p>
        {/if}
        {#if job.destinations.length}
          <ul class="destination-results">
            {#each job.destinations as destination}<li>
                <strong>{label(destination.repository_id)}</strong>
                <span class:bad={destination.state !== 'success' && destination.state !== 'running'}>
                  {destination.state}
                </span>
                <span class="muted">{destination.message}</span>
                {#if destination.snapshot_id}<code>{destination.snapshot_id.slice(0, 12)}</code>{/if}
              </li>{/each}
          </ul>
        {/if}
        <HistoryOutput output={job.output} truncated={job.output_truncated} />
      </article>
    {/each}
  </div>
{/if}
