<script lang="ts">
  import Dialog from './Dialog.svelte';
  import type { Config, Repository, Request, RetentionPreview, Run, Save, Status } from './types';
  import { assertSuccess, bytes, copyConfig, date, isCurrent, preview } from './model';
  let { status, busy, ready, save, run, action }: { status: Status; busy: boolean; ready: boolean; save: Save; run: Run; action: (request: Request) => Promise<void> } = $props();
  let draft = $state<Config | null>(null); let repository = $state<Repository | null>(null); let editing = $state(false); let saving = $state(false); let error = $state('');
  let toolId = $state(''); let toolError = $state(''); let toolNotice = $state(''); let loading = $state(false); let profileId = $state(''); let confirmation = $state(''); let pruneConfirmation = $state('');
  let retention = $state<RetentionPreview | null>(null); let retentionScope = $state(''); let previewAt = 0;
  let statistics = $state<Record<string, unknown> | null>(null); let statisticsScope = $state(''); let ticket = 0;
  let initializeRepo = $state<Repository | null>(null);
  let selected = $derived(status.config.repositories.find(repo => repo.id === toolId));
  let scope = $derived(selected ? `${selected.id}:${selected.location}:${status.config.revision}` : '');
  let policyScope = $derived(`${scope}:${profileId}`);
  function open(existing?: Repository) { draft = copyConfig(status.config); editing = !!existing; error = ''; repository = existing ? { ...existing } : { id: crypto.randomUUID(), label: '', location: '' }; }
  function close() { if (!saving) { draft = null; repository = null; } }
  async function submit(event: SubmitEvent) {
    event.preventDefault(); if (!draft || !repository) return;
    if (draft.revision !== status.config.revision) { error = 'Configuration changed. Cancel and reopen this dialog.'; return; }
    const updated = { ...repository, label: repository.label.trim(), location: repository.location.trim() };
    const next = copyConfig(draft);
    next.repositories = editing ? next.repositories.map(repo => repo.id === updated.id ? updated : repo) : [...next.repositories, updated];
    saving = true; const problem = await save(next); saving = false;
    if (problem === null) close(); else error = problem;
  }
  async function remove() {
    if (!draft || !repository) return;
    if (draft.profiles.some(profile => profile.repository_ids.includes(repository!.id))) { error = 'Remove this repository from its backup profiles before removing its configuration.'; return; }
    const next = copyConfig(draft); next.repositories = next.repositories.filter(repo => repo.id !== repository!.id);
    saving = true; const problem = await save(next); saving = false;
    if (problem === null) close(); else error = problem;
  }
  function tools(repo: Repository) { ticket++; toolId = repo.id; toolError = ''; toolNotice = ''; statistics = null; retention = null; confirmation = ''; pruneConfirmation = ''; profileId = status.config.profiles.find(profile => profile.repository_ids.includes(repo.id))?.id ?? ''; }
  function closeTools() { ticket++; toolId = ''; loading = false; }
  async function query(kind: 'stats' | 'retention_preview' | 'check') {
    if (!selected) return;
    const token = ++ticket; const captured = kind === 'retention_preview' ? policyScope : scope;
    loading = true; toolError = ''; toolNotice = '';
    if (kind === 'retention_preview') { retention = null; confirmation = ''; } else if (kind === 'stats') statistics = null;
    try {
      const job = await run(kind === 'retention_preview' ? { op: kind, repository_id: selected.id, profile_id: profileId } : { op: kind, repository_id: selected.id });
      if (!isCurrent(token, ticket, captured, kind === 'retention_preview' ? policyScope : scope)) return;
      if (kind === 'check') { if (job.state !== 'success') throw new Error(job.message || 'Repository check failed. Review activity.'); toolNotice = 'Repository check completed. This is not a full read-data verification.'; }
      else if (kind === 'stats') { const result = assertSuccess(job); if (!result || typeof result !== 'object' || Array.isArray(result)) throw new Error('Statistics are unavailable. Run statistics again.'); statistics = result as Record<string, unknown>; statisticsScope = captured; }
      else { retention = preview(assertSuccess(job)); retentionScope = captured; previewAt = Date.now(); }
    } catch (cause) { if (token === ticket) toolError = cause instanceof Error ? cause.message : 'The query failed. Try again.'; }
    finally { if (token === ticket) loading = false; }
  }
  async function apply(event: SubmitEvent) {
    event.preventDefault(); if (!retention || retentionScope !== policyScope || confirmation !== 'DELETE') return;
    if (Date.now() - previewAt >= 300000) { retention = null; toolError = 'This preview expired. Run a new preview before deleting snapshots.'; return; }
    const token = retention.token; retention = null; confirmation = '';
    await maintenance({ op: 'apply_retention', token, confirmation: 'DELETE' }, 'Previewed snapshots deleted. Space has not been pruned.');
  }
  async function maintenance(operation: Request, message: string) {
    const token = ++ticket; const captured = scope; toolError = ''; toolNotice = ''; loading = true;
    try { const job = await run(operation); if (!isCurrent(token, ticket, captured, scope)) return; if (job.state !== 'success') throw new Error(job.message || `Operation ${job.state}. Review activity.`); toolNotice = message; statistics = null; retention = null; }
    catch (cause) { if (token === ticket) toolError = cause instanceof Error ? cause.message : 'Maintenance failed. Review activity before trying again.'; }
    finally { if (token === ticket) loading = false; }
  }
  async function prune(event: SubmitEvent) { event.preventDefault(); if (!selected || pruneConfirmation !== 'PRUNE') return; const id = selected.id; pruneConfirmation = ''; await maintenance({ op: 'prune', repository_id: id, confirmation: 'PRUNE' }, 'Repository pruning completed.'); }
  async function initialize() { const repo = initializeRepo; initializeRepo = null; if (repo) await action({ op: 'initialize', repository_id: repo.id }); }
</script>

<div class="list-toolbar"><span class="muted">{status.config.repositories.length} {status.config.repositories.length === 1 ? 'repository' : 'repositories'}</span><button class="primary" disabled={busy} onclick={() => open()}>Add repository</button></div>
{#if !status.config.repositories.length}<div class="empty"><h2>No repositories configured</h2><p>Add a local path or a supported restic backend location, then initialize an empty repository. ysynk generates its password natively and stores it in the system keychain.</p></div>{/if}
<div class="list">{#each status.config.repositories as repo}<article class="repository-row"><div class="list-row"><div class="grow"><h2>{repo.label}</h2><p class="path">{repo.location}</p></div><button disabled={busy} onclick={() => open(repo)}>Edit</button></div><div class="repository-bottom"><div class="actions"><button disabled={busy || !ready} onclick={() => initializeRepo = repo}>Initialize / retry</button><button disabled={busy || !ready} onclick={() => tools(repo)}>Inspect & maintain</button></div></div></article>{/each}</div>
<p class="hint">Saving a location does not confirm initialization or keychain access. Review operation results and load snapshots for backup evidence. Status polling does not access repositories or the keychain.</p>

{#if draft && repository}<Dialog title={editing ? 'Edit repository' : 'Add repository'} {close}><form onsubmit={submit}>
  {#if error}<p class="message error" role="alert">{error}</p>{/if}
  <label>Repository name<input required maxlength="120" bind:value={repository.label}/></label>
  <label>Repository location<input required readonly={editing} class="path" bind:value={repository.location} placeholder="/mnt/backup/ysynk"/><span class="field-hint">Native local path or restic backend location. No embedded passwords, credential query strings or shell commands.</span></label>
  <p class="hint">Saving this location does not initialize a repository. ysynk generates its password during initialization; you never enter a restic password.</p>
  {#if editing}<p class="message warning">Keychain records are bound to the repository location. Add a separate repository for another destination; editing a label does not move backups or change its password.</p><details><summary>Remove configuration</summary><p class="hint">Remove this repository from profiles first. This does not delete backups, its system keychain record or remote files.</p><button type="button" class="danger" disabled={busy || saving} onclick={remove}>Remove repository configuration</button></details>{/if}
  <div class="dialog-actions"><button type="button" disabled={saving} onclick={close}>Cancel</button><button class="primary" disabled={busy || saving}>{saving ? 'Saving…' : editing ? 'Save repository' : 'Add repository'}</button></div>
</form></Dialog>{/if}

{#if initializeRepo}<Dialog title="Initialize or retry repository" close={() => initializeRepo = null}><p class="dialog-intro"><strong>{initializeRepo.label}</strong></p><p class="path">{initializeRepo.location}</p><p class="hint">ysynk generates a random password natively and saves it in the system keychain before initializing this location. A retry reuses the original saved password and checks the repository; it never generates a replacement password for an existing record.</p><p class="message warning">The system keychain is usually unlocked with your desktop login. If it is locked or unavailable, restore access using your desktop's keychain tools, then retry. There is no plaintext fallback.</p><p class="hint">Use an empty location for a new repository. This is not an import flow for an existing repository with a different password. Initialization is only confirmed by the operation result.</p><div class="dialog-actions"><button onclick={() => initializeRepo = null}>Cancel</button><button class="primary" disabled={busy || !ready} onclick={initialize}>Initialize / retry</button></div></Dialog>{/if}

{#if selected}<Dialog title={selected.label} close={closeTools}><p class="path dialog-intro">{selected.location}</p>
  {#if toolError}<p class="message error" role="alert">{toolError}</p>{/if}{#if toolNotice}<p class="message success" role="status">{toolNotice}</p>{/if}
  <section><h3>Repository inspection</h3><p class="hint">Commands run only when you request them. Keychain access is checked by the operation, never by status polling.</p><div class="actions"><button disabled={busy || !ready || loading} onclick={() => query('stats')}>Load raw-data statistics</button><button disabled={busy || !ready || loading} onclick={() => query('check')}>Check repository</button></div>{#if statistics && statisticsScope === scope}<dl class="stats"><div><dt>Raw-data size</dt><dd>{bytes(typeof statistics.total_size === 'number' ? statistics.total_size : undefined)}</dd></div><div><dt>Files reported</dt><dd>{typeof statistics.total_file_count === 'number' ? statistics.total_file_count.toLocaleString() : 'Not reported'}</dd></div></dl><p class="hint">Restic raw-data mode; not physical disk usage.</p><details><summary>Full statistics</summary><pre>{JSON.stringify(statistics, null, 2)}</pre></details>{/if}</section>
  <section><h3>Retention preview</h3><p class="hint">Applies only to this profile's tagged snapshots, grouped by host and source paths. Previewing does not delete data.</p><label>Profile<select bind:value={profileId} onchange={() => { ticket++; retention = null; confirmation = ''; loading = false; }}>{#if !status.config.profiles.some(profile => profile.repository_ids.includes(selected!.id))}<option value="">No profiles use this repository</option>{/if}{#each status.config.profiles.filter(profile => profile.repository_ids.includes(selected!.id)) as profile}<option value={profile.id}>{profile.label}</option>{/each}</select></label><button disabled={busy || !ready || !profileId || loading} onclick={() => query('retention_preview')}>Preview retention</button>
  {#if retention && retentionScope === policyScope}<div class="preview"><p><strong>{retention.remove.length} snapshots proposed for deletion</strong> · {retention.keep_count} kept</p>{#if retention.remove.length}<ul class="snapshot-preview">{#each retention.remove as snapshot}<li><code>{snapshot.id.slice(0, 12)}</code><span>{date(snapshot.time)}</span></li>{/each}</ul><p class="message warning">Deletion is irreversible. The server token expires after five minutes and is single-use. External restic operations are not transactional with this preview. No automatic prune follows.</p><form onsubmit={apply}><label>Type DELETE to delete only these snapshots<input required bind:value={confirmation} autocomplete="off" spellcheck="false" pattern="DELETE"/></label><button class="danger" disabled={busy || loading || !ready || confirmation !== 'DELETE'}>Delete previewed snapshots</button></form>{:else}<p class="hint">This policy proposes no deletions.</p>{/if}</div>{/if}</section>
  <section><details><summary>Prune unreferenced data</summary><p class="message warning">Repository-wide maintenance. Prune can remove data no longer referenced by any snapshot and may take time. It is separate from retention; it does not remove stale locks automatically.</p><form onsubmit={prune}><label>Type PRUNE for this repository<input bind:value={pruneConfirmation} required pattern="PRUNE" autocomplete="off" spellcheck="false"/></label><button class="danger" disabled={busy || loading || !ready || pruneConfirmation !== 'PRUNE'}>Prune repository</button></form></details></section>
  {#if loading}<p class="hint" role="status">Operation running. You can close this dialog and follow activity.</p>{/if}<div class="dialog-actions"><button onclick={closeTools}>Close</button></div>
</Dialog>{/if}
