<script lang="ts">
  import Dialog from './Dialog.svelte';
  import type { Config, Profile, Request, Save, Status } from './types';
  import { copyConfig, lines } from './model';
  let {
    status,
    busy,
    ready,
    save,
    action,
    openRepositories,
  }: {
    openRepositories: () => void;
    status: Status;
    busy: boolean;
    ready: boolean;
    save: Save;
    action: (request: Request) => Promise<void>;
  } = $props();
  let draft = $state<Config | null>(null);
  let profile = $state<Profile | null>(null);
  let sources = $state('');
  let excludes = $state('');
  let editing = $state(false);
  let saving = $state(false);
  let error = $state('');
  function open(existing?: Profile) {
    draft = copyConfig(status.config);
    editing = !!existing;
    error = '';
    profile = existing
      ? { ...existing, repository_ids: [...existing.repository_ids], retention: { ...existing.retention } }
      : {
          id: crypto.randomUUID(),
          label: '',
          sources: [],
          excludes: [],
          repository_ids: [],
          schedule_minutes: 0,
          retention: { keep_last: 5, keep_daily: 7, keep_weekly: 4, keep_monthly: 6 },
        };
    sources = profile.sources.join('\n');
    excludes = profile.excludes.join('\n');
  }
  function close() {
    if (!saving) {
      draft = null;
      profile = null;
    }
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!draft || !profile) return;
    if (!lines(sources).length || !profile.repository_ids.length) {
      error = 'Add at least one source path and select a repository.';
      return;
    }
    if (draft.revision !== status.config.revision) {
      error = 'Configuration changed while this dialog was open. Cancel and reopen it before saving.';
      return;
    }
    const updated = {
      ...profile,
      label: profile.label.trim(),
      sources: lines(sources),
      excludes: lines(excludes),
    };
    const next = copyConfig(draft);
    next.profiles = editing
      ? next.profiles.map((item) => (item.id === updated.id ? updated : item))
      : [...next.profiles, updated];
    saving = true;
    const problem = await save(next);
    saving = false;
    if (problem === null) close();
    else error = problem;
  }
  async function remove() {
    if (!draft || !profile) return;
    const next = copyConfig(draft);
    next.profiles = next.profiles.filter((item) => item.id !== profile!.id);
    saving = true;
    const problem = await save(next);
    saving = false;
    if (problem === null) close();
    else error = problem;
  }
</script>

<div class="list-toolbar">
  <span class="muted">
    {status.config.profiles.length}
    {status.config.profiles.length === 1 ? 'profile' : 'profiles'}
  </span>
  <button class="primary" disabled={busy || !status.config.repositories.length} onclick={() => open()}>
    Add profile
  </button>
</div>

{#if !status.config.profiles.length}
  <div class="empty">
    <h2>No backup profiles yet</h2>
    {#if status.config.repositories.length}
      <p>Add a profile to choose which folders to back up and where to store them.</p>
    {:else}
      <p>
        Start by adding a repository for your backups. Then create a profile to choose your folders and
        schedule.
      </p>
      <button class="text-button" onclick={openRepositories}>Go to repositories</button>
    {/if}
  </div>
{/if}

<div class="list">
  {#each status.config.profiles as item (item.id)}
    <article class="list-row">
      <div class="grow">
        <h2>{item.label}</h2>
        <p>
          {item.sources.length} source{item.sources.length === 1 ? '' : 's'} · {item.repository_ids.length}
          {item.repository_ids.length === 1 ? 'repository' : 'repositories'} · {item.schedule_minutes === 0
            ? 'Manual only'
            : item.schedule_minutes === 1
              ? 'Every minute'
              : `Every ${item.schedule_minutes} minutes`}
        </p>
        <details>
          <summary>Sources and destinations</summary>
          <ul>
            {#each item.sources as path}<li class="path">{path}</li>{/each}
          </ul>
          <p>
            {item.repository_ids
              .map((id) => status.config.repositories.find((repo) => repo.id === id)?.label ?? id)
              .join(', ')}
          </p>
        </details>
      </div>
      <div class="actions">
        <button aria-label={`Edit ${item.label}`} disabled={busy} onclick={() => open(item)}>Edit</button>
        <button
          class="primary"
          aria-label={`Back up now: ${item.label}`}
          disabled={busy || !ready}
          onclick={() => action({ op: 'backup', profile_id: item.id })}
        >
          Back up now
        </button>
      </div>
    </article>
  {/each}
</div>

{#if draft && profile}
  <Dialog title={editing ? 'Edit profile' : 'Add profile'} {close}>
    <form onsubmit={submit}>
      {#if error}
        <p class="message error" role="alert">{error}</p>
      {/if}
      <label>
        Profile name
        <input required maxlength="120" bind:value={profile.label} />
      </label>
      <label>
        Source paths
        <textarea
          required
          rows="3"
          class="path"
          bind:value={sources}
          placeholder="/home/you/Documents"
        ></textarea>
        <span class="field-hint">One absolute native path per line.</span>
      </label>
      <fieldset>
        <legend>Repositories</legend>
        {#each draft.repositories as repo}<label class="check">
            <input type="checkbox" value={repo.id} bind:group={profile.repository_ids} />
            <span>
              {repo.label}
              <span class="field-hint path">{repo.location}</span>
            </span>
          </label>
        {/each}
      </fieldset>
      <label>
        Schedule interval (minutes)
        <input type="number" required min="0" max="525600" step="1" bind:value={profile.schedule_minutes} />
        <span class="field-hint">0 is manual only. The app must remain running.</span>
      </label>
      <details>
        <summary>Exclusions and retention policy</summary>
        <label>
          Exclude patterns
          <textarea rows="3" class="path" bind:value={excludes} placeholder="**/node_modules"></textarea>
          <span class="field-hint">One restic exclusion pattern per line. No shell commands.</span>
        </label>
        <p class="hint">
          Saving a retention policy never deletes snapshots. Preview and apply it explicitly in Repositories.
        </p>
        <div class="form-grid">
          <label>
            Keep last
            <input type="number" required min="1" step="1" bind:value={profile.retention.keep_last} />
          </label>
          <label>
            Keep daily
            <input type="number" required min="0" step="1" bind:value={profile.retention.keep_daily} />
          </label>
          <label>
            Keep weekly
            <input type="number" required min="0" step="1" bind:value={profile.retention.keep_weekly} />
          </label>
          <label>
            Keep monthly
            <input type="number" required min="0" step="1" bind:value={profile.retention.keep_monthly} />
          </label>
        </div>
      </details>
      {#if editing}
        <details>
          <summary>Remove profile</summary>
          <p class="hint">
            Stops this profile's schedule. Existing snapshots and repository data are not deleted.
          </p>
          <button type="button" class="danger" disabled={busy || saving} onclick={remove}>
            Remove profile
          </button>
        </details>
      {/if}
      <div class="dialog-actions">
        <button type="button" disabled={saving} onclick={close}>Cancel</button>
        <button class="primary" disabled={busy || saving}>
          {saving ? 'Saving…' : editing ? 'Save profile' : 'Add profile'}
        </button>
      </div>
    </form>
  </Dialog>
{/if}
