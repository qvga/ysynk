<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { request, inDesktop } from './lib/client';
  import { JobCompletions } from './lib/jobs';
  import type { Config, Job, Request, Status } from './lib/types';
  import Icon from './lib/Icon.svelte';
  import Activity from './lib/Activity.svelte';
  import CommandOutput from './lib/CommandOutput.svelte';
  import { bytes } from './lib/model';
  import Profiles from './lib/Profiles.svelte';
  import Repositories from './lib/Repositories.svelte';
  import Snapshots from './lib/Snapshots.svelte';
  import Preferences from './lib/Preferences.svelte';

  const pages = ['Profiles', 'Repositories', 'Snapshots', 'Settings'] as const;
  type Page = typeof pages[number];
  let page = $state<Page>('Profiles');
  let status = $state<Status | null>(null);
  let connection = $state('');
  let error = $state('');
  let notice = $state('');
  let sending = $state(false);
  let waiting = $state(false);
  let theme = $state('system');
  let polling = $state(false);
  let generation = 0;
  let statusSequence = 0;
  let alive = true;
  const pending = new JobCompletions();
  let busy = $derived(sending || waiting || !!status?.active || !!connection);
  let ready = $derived(!!status && !status.setup_error && !!status.restic_version);
  let config = $derived(status?.config);
  function accept(next: Status, sequence = 0) {
    if (!alive) return;
    status = next; connection = '';
    pending.accept(next, sequence);
    waiting = pending.size > 0;
  }
  async function refresh() {
    if (polling || !alive) return;
    polling = true;
    const stamp = generation;
    const sequence = ++statusSequence;
    try { const next = await request<Status>({ op: 'status' }); if (stamp === generation) accept(next, sequence); }
    catch (cause) { if (alive && stamp === generation) connection = cause instanceof Error ? cause.message : 'The local engine is unavailable. Reopen ysynk and try again.'; }
    finally { polling = false; }
  }
  async function run(operation: Request): Promise<Job> {
    if (busy) throw new Error('Another operation is running or the engine is unavailable. Wait for it to finish, or cancel it in the activity bar.');
    sending = true; error = ''; notice = '';
    try {
      const response = await request<{ job_id: string }>(operation);
      if (!response.job_id) throw new Error('The engine did not return a job ID. Reconnect before trying again.');
      const result = pending.wait(response.job_id, statusSequence);
      waiting = true;
      // A pre-admission poll may still be in flight. Only a later poll can
      // establish that this job completed or fell out of retained history.
      void refresh();
      return await result;
    } finally { sending = false; }
  }
  async function action(operation: Request) {
    try {
      const job = await run(operation);
      if (job.state !== 'success') throw new Error(job.message || `Operation ${job.state}. Review activity for per-repository results.`);
      notice = `${job.label || job.kind} completed.`;
    } catch (cause) { error = cause instanceof Error ? cause.message : 'The operation failed. Review activity and try again.'; }
  }
  async function save(next: Config): Promise<string | null> {
    if (busy) return 'Wait for the active operation before saving configuration.';
    sending = true; error = ''; notice = '';
    try {
      const updated = await request<Status>({ op: 'save_config', config: next });
      generation++; accept(updated); notice = 'Configuration saved.'; return null;
    } catch (cause) { return cause instanceof Error ? cause.message : 'Configuration could not be saved. Reload and try again.'; }
    finally { sending = false; }
  }
  async function selectEngine() {
    if (busy || !config) return;
    sending = true; error = ''; notice = '';
    try {
      const updated = await invoke<Status | null>('select_restic_executable', { revision: config.revision });
      if (updated) { generation++; accept(updated); notice = 'Restic executable saved.'; }
    } catch (cause) { error = cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : 'Could not select the restic executable.'; }
    finally { sending = false; }
  }
  async function cancel() {
    try { await request<null>({ op: 'cancel' }); notice = 'Cancellation requested. Wait for the operation to stop.'; }
    catch { error = 'Cancellation could not be sent. Reconnect and check activity.'; }
  }
  function setTheme(value: string) { theme = value; try { localStorage.setItem('ysynk-theme', value); } catch { /* Desktop preferences can be read-only. */ } }
  onMount(() => {
    try { const value = localStorage.getItem('ysynk-theme'); if (value && ['light', 'dark', 'system'].includes(value)) theme = value; } catch { /* Use the system theme. */ }
    void refresh();
    const timer = setInterval(() => { if (inDesktop()) void refresh(); }, 1000);
    return () => { alive = false; clearInterval(timer); pending.close(); };
  });
</script>

<div class="app" data-theme={theme}>
  <a class="skip-link" href="#main">Skip to content</a>
  <aside class="sidebar">
    <div class="brand"><Icon name="shield" size={27}/><span>ysynk</span></div>
    <nav aria-label="Main navigation">{#each pages as item}<button aria-label={item} aria-current={page === item ? 'page' : undefined} class:current={page === item} onclick={() => { page = item; error = ''; notice = ''; }}><Icon name={item}/><span>{item}</span></button>{/each}</nav>
    <div class="sidebar-footer"><span class:online={!!status && !connection} class="connection-dot"></span><div><strong>{connection ? 'Not connected' : status ? 'Local engine' : 'Connecting'}</strong><span>Restic backup</span></div></div>
  </aside>
  <div class="workspace">
    <main id="main" tabindex="-1">
       <div class="page-heading"><h1>{page}</h1></div>
      {#if connection}<div class="message warning" role="status"><strong>Local engine unavailable</strong><p>{connection}</p><button onclick={refresh} disabled={polling}>Reconnect</button></div>{/if}
      {#if error}<div class="message error" role="alert"><p>{error}</p><button class="quiet" onclick={() => error = ''}>Dismiss</button></div>{/if}
      {#if notice}<p class="message success" role="status">{notice}</p>{/if}
       {#if status?.setup_error}<div class="message warning" role="status"><strong>Restic unavailable</strong><p>{status.setup_error}</p><p>Configure the restic executable in Settings.</p></div>{/if}
      {#if !status && !connection}<div class="empty" aria-busy="true"><h2>Connecting to the local engine…</h2><p>Reading configuration and recent activity.</p></div>{/if}
      {#if status && config}
         {#if page === 'Profiles'}<Profiles {status} {busy} {ready} {save} {action}/><section class="profile-history" aria-labelledby="activity-heading"><div class="section-heading"><h2 id="activity-heading">Operation history</h2></div><Activity jobs={status.history.filter(job => job.id !== status?.active?.id)} repositories={config.repositories}/></section>
        {:else if page === 'Repositories'}<Repositories {status} {busy} {ready} {save} {run} {action}/>
        {:else if page === 'Snapshots'}<Snapshots {status} {busy} {ready} {run}/>
        {:else}<Preferences {status} {busy} {selectEngine} {theme} changeTheme={setTheme}/>{/if}
      {/if}
    </main>
    {#if status?.active}
      <aside class="jobbar" aria-label="Current operation">
        <div class="job-heading"><div class="grow"><strong>{status.active.label || status.active.kind}</strong><p class="small muted">{status.active.message || 'Operation running. Other repository operations wait until this finishes.'}</p></div><button onclick={cancel}>Cancel operation</button></div>
        {#key status.active.id}<CommandOutput output={status.active.output} truncated={status.active.output_truncated} active/>{/key}
        {#if status.active.progress}<p class="small muted">{bytes(status.active.progress.bytes_done)} processed · {status.active.progress.files_done.toLocaleString()} files</p>{/if}
        {#if status.active.destinations.length}<details><summary>Live destination results</summary><ul class="destination-results">{#each status.active.destinations as destination}<li><strong>{status.config.repositories.find(repo => repo.id === destination.repository_id)?.label ?? destination.repository_id}</strong><span class:bad={['failed', 'partial', 'interrupted'].includes(destination.state)}>{destination.state}</span><span class="muted">{destination.message}</span></li>{/each}</ul></details>{/if}
      </aside>
    {/if}
  </div>
</div>
