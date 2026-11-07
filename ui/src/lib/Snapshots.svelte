<script lang="ts">
  import Dialog from './Dialog.svelte';
  import type { FileNode, Run, Snapshot, Status } from './types';
  import { assertSuccess, bytes, date, files, isCurrent, snapshots, snapshotStats } from './model';
  let { status, busy, ready, run }: { status: Status; busy: boolean; ready: boolean; run: Run } = $props();
  let repositoryId = $state(''); let snapshot = $state<Snapshot | null>(null); let path = $state('/'); let pathInput = $state('/');
  let items = $state<Snapshot[] | null>(null); let nodes = $state<FileNode[] | null>(null); let selectedPaths = $state<string[]>([]);
  let listScope = $state(''); let directoryScope = $state(''); let snapshotScope = $state('');
  let error = $state(''); let notice = $state(''); let loading = $state(false); let listing = $state(false); let ticket = 0;
  let restore = $state<{ repository_id: string; snapshot_id: string; paths: string[]; scope: string } | null>(null); let target = $state(''); let restoring = $state(false); let restoreError = $state('');
  let repository = $derived(status.config.repositories.find(repo => repo.id === repositoryId));
  let scope = $derived(repository ? `${repository.id}:${repository.location}:${status.config.revision}` : '');
  let fileScope = $derived(`${scope}:${snapshot?.id ?? ''}:${path}`);
  let stats = $derived(snapshot ? snapshotStats(snapshot) : []);
  function reset() { ticket++; snapshot = null; items = null; nodes = null; selectedPaths = []; error = ''; notice = ''; loading = false; listing = false; path = '/'; pathInput = '/'; }
  async function load() {
    if (!repository) return;
    const stamp = ++ticket; const captured = scope; const id = repository.id;
    items = null; snapshot = null; nodes = null; selectedPaths = []; error = ''; notice = ''; listing = true;
    try { const job = await run({ op: 'snapshots', repository_id: id }); if (!isCurrent(stamp, ticket, captured, scope)) return; items = snapshots(assertSuccess(job)).slice().sort((a, b) => b.time.localeCompare(a.time)); listScope = captured; }
    catch (cause) { if (stamp === ticket) error = cause instanceof Error ? cause.message : 'Snapshots could not be loaded. Try again.'; }
    finally { if (stamp === ticket) listing = false; }
  }
  function choose(item: Snapshot) { ticket++; snapshot = item; snapshotScope = scope; nodes = null; selectedPaths = []; path = '/'; pathInput = '/'; loading = false; error = ''; notice = ''; }
  async function browse(next: string) {
    if (!repository || !snapshot || snapshotScope !== scope) return;
    path = next || '/'; pathInput = path; nodes = null; error = ''; loading = true;
    const stamp = ++ticket; const captured = fileScope;
    try { const job = await run({ op: 'list_files', repository_id: repository.id, snapshot_id: snapshot.id, path }); if (!isCurrent(stamp, ticket, captured, fileScope)) return; nodes = files(assertSuccess(job)); directoryScope = captured; }
    catch (cause) { if (stamp === ticket) error = cause instanceof Error ? cause.message : 'Directory could not be loaded. Try again.'; }
    finally { if (stamp === ticket) loading = false; }
  }
  function toggle(node: FileNode) { selectedPaths = selectedPaths.includes(node.path) ? selectedPaths.filter(item => item !== node.path) : [...selectedPaths, node.path]; }
  function openRestore(full: boolean) { if (!repository || !snapshot || snapshotScope !== scope) return; restore = { repository_id: repository.id, snapshot_id: snapshot.id, paths: full ? [] : [...selectedPaths], scope }; target = ''; restoreError = ''; }
  function closeRestore() { restore = null; }
  async function submitRestore(event: SubmitEvent) {
    event.preventDefault(); if (!restore || restoring) return;
    if (restore.scope !== scope || restore.snapshot_id !== snapshot?.id) { restoreError = 'Repository or snapshot selection changed. Close this dialog and choose the restore again.'; return; }
    restoring = true; restoreError = ''; notice = '';
    const captured = restore; const output = target;
    try {
      const job = await run({ op: 'restore', repository_id: captured.repository_id, snapshot_id: captured.snapshot_id, paths: captured.paths, target: output });
      if (job.state !== 'success') throw new Error(`${job.message || `Restore ${job.state}.`} Any incomplete output is preserved at the requested target. Inspect it before trying a different new target.`);
      if (captured.scope === scope && captured.snapshot_id === snapshot?.id) notice = `Restore completed to ${output}.`;
      if (restore === captured) restore = null;
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : 'Restore failed. Incomplete output is preserved; inspect the requested target before retrying.';
      if (restore === captured) restoreError = message; else error = message;
    }
    finally { restoring = false; }
  }
  function parent(path: string) { return path.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/'; }
</script>

<div class="query-toolbar"><label>Repository<select bind:value={repositoryId} onchange={reset}><option value="">Choose a repository</option>{#each status.config.repositories as repo}<option value={repo.id}>{repo.label}</option>{/each}</select></label><button class="primary" disabled={busy || !ready || !repository} onclick={load}>{listing ? 'Loading…' : 'Load snapshots'}</button></div>
{#if error}<p class="message error" role="alert">{error}</p>{/if}{#if notice}<p class="message success" role="status">{notice}</p>{/if}
{#if !repository}<div class="empty"><h2>No repository selected</h2><p>Select a repository, then load its snapshots. Queries run only on request.</p></div>
{:else if listing}<div class="empty" aria-busy="true"><h2>Reading snapshots…</h2><p>Progress appears in the current operation bar.</p></div>
{:else if items && listScope === scope}
  <section><div class="section-heading"><h2>Snapshots</h2><span class="muted small">{items.length} returned · refresh on demand</span></div>
  {#if !items.length}<div class="empty"><h3>No snapshots reported</h3><p>Run a backup profile for this repository, then load snapshots again.</p></div>{/if}
  <div class="snapshot-list">{#each items as item}<button class:selected={snapshot?.id === item.id} class="snapshot-row" aria-pressed={snapshot?.id === item.id} onclick={() => choose(item)}><span><strong>{date(item.time)}</strong><span class="muted small">{item.hostname || 'Host not reported'} · {(item.paths ?? []).join(', ') || 'Sources not reported'}</span></span><code>{item.id.slice(0, 12)}</code></button>{/each}</div></section>
{:else}<div class="empty"><h2>Load snapshots on demand</h2><p>Load snapshots to see what restic reports. Repository and keychain access are checked when the query runs.</p></div>{/if}

{#if snapshot && snapshotScope === scope}<section class="browser"><div class="section-heading"><div><h2>{date(snapshot.time)}</h2><p class="muted path">{snapshot.id}</p></div><button disabled={busy || !ready} onclick={() => openRestore(true)}>Restore snapshot…</button></div>
  <dl class="snapshot-details">
    <div><dt>Host</dt><dd>{snapshot.hostname || 'Not reported'}</dd></div>
    <div><dt>Source paths</dt><dd class="path">{#each snapshot.paths ?? [] as source}<div>{source}</div>{:else}Not reported{/each}</dd></div>
    {#if snapshot.tags?.length}<div><dt>Tags</dt><dd>{snapshot.tags.join(', ')}</dd></div>{/if}
    {#each stats as stat}<div><dt>{stat.label}</dt><dd>{stat.value}</dd></div>{/each}
  </dl>
  <p class="hint">{stats.length ? 'Statistics recorded when this backup was created; data added is not the snapshot’s restore size or current repository usage.' : 'No backup statistics were reported for this snapshot.'}</p>
  <form class="query-toolbar" onsubmit={(event) => { event.preventDefault(); void browse(pathInput); }}><label>Snapshot directory<input class="path" bind:value={pathInput} required placeholder="/"/></label><button type="button" disabled={busy || !ready || path === '/'} onclick={() => browse(parent(path))}>Up</button><button disabled={busy || !ready}>{loading ? 'Reading…' : 'Open directory'}</button></form>
  {#if loading}<p class="hint" aria-busy="true">Reading this directory…</p>
  {:else if nodes && directoryScope === fileScope}<div class="file-list">{#if !nodes.length}<p class="empty">No entries reported in this directory.</p>{/if}{#each nodes as node}<div class="file-row"><input type="checkbox" aria-label={`Select ${node.path}`} checked={selectedPaths.includes(node.path)} onchange={() => toggle(node)}/><div class="grow">{#if node.type === 'dir'}<button class="text-button" disabled={busy || !ready} onclick={() => browse(node.path)}>{node.name || node.path}/</button>{:else}<strong>{node.name || node.path}</strong>{/if}<p class="path muted">{node.path}</p></div><span class="muted small">{node.type === 'dir' ? 'Directory' : node.type === 'file' ? bytes(node.size) : node.type}</span></div>{/each}</div>
  {:else}<p class="hint">Open a directory to browse its immediate entries. Paths are displayed and restored literally, not expanded as patterns.</p>{/if}
  {#if selectedPaths.length}<div class="selection-bar"><strong>{selectedPaths.length} literal paths selected</strong><div class="actions"><button class="quiet" onclick={() => selectedPaths = []}>Clear selection</button><button class="primary" disabled={busy || !ready} onclick={() => openRestore(false)}>Restore selected…</button></div></div><details><summary>Selected paths</summary><ul>{#each selectedPaths as selectedPath}<li class="path">{selectedPath}</li>{/each}</ul></details>{/if}
</section>{/if}

{#if restore}<Dialog title={restore.paths.length ? 'Restore selected paths' : 'Restore full snapshot'} close={closeRestore}><form onsubmit={submitRestore}>
  {#if restoreError}<p class="message error" role="alert">{restoreError}</p>{/if}
  <p class="dialog-intro">{repository?.label} · <code>{restore.snapshot_id.slice(0, 12)}</code></p>
  {#if restore.paths.length}<ul class="restore-paths">{#each restore.paths as selectedPath}<li class="path">{selectedPath}</li>{/each}</ul><p class="hint">Exact snapshot paths only. Wildcards in names remain literal.</p>{:else}<p class="hint">Restore all files in this snapshot.</p>{/if}
  <label>New output directory<input required disabled={restoring} class="path" bind:value={target} placeholder="/home/you/Restored/backup-2026-11-02"/><span class="field-hint">An absolute native path that does not exist, under a safe existing parent directory. No existing output is overwritten.</span></label>
  <p class="message warning">ysynk creates a new private output directory and verifies the restore. If the operation fails or is cancelled, incomplete output is kept for inspection. A retry needs a different new target.</p>
  {#if restoring}<p class="hint" role="status">The restore continues when you close this dialog. Use Cancel operation in the activity bar to stop it.</p>{/if}
  <div class="dialog-actions"><button type="button" onclick={closeRestore}>{restoring ? 'Close dialog' : 'Cancel'}</button><button class="primary" disabled={busy || restoring || !ready}>{restoring ? 'Restoring…' : 'Restore to new directory'}</button></div>
</form></Dialog>{/if}
