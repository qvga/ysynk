<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { inDesktop } from './client';
  import Dialog from './Dialog.svelte';
  import type { Config, DesktopPreferences, DesktopSettings, Repository, Save, Status } from './types';
  import { copyConfig } from './model';
  let { status, busy, save, theme, changeTheme }: { status: Status; busy: boolean; save: Save; theme: string; changeTheme: (theme: string) => void } = $props();
  let draft = $state<Config | null>(null); let saving = $state(false); let error = $state(''); let trayConfirmed = $state(false); let quitting = $state(false); let trayDialog = $state(false); let quitDialog = $state(false);
  let exportDraft = $state<{ revision: number; repositories: Repository[]; repositoryId: string } | null>(null);
  let exportConfirmed = $state(false); let exporting = $state(false); let exportError = $state(''); let exportNotice = $state('');
  let desktopActual = $state<DesktopSettings | null>(null);
  let desktopDraft = $state<DesktopPreferences | null>(null);
  let desktopLoading = $state(true); let desktopSaving = $state(false);
  let desktopError = $state(''); let desktopNotice = $state('');
  let mounted = true;
  let desktopReadPending = false;
  let desktopPending = $derived(desktopLoading || desktopSaving);
  let desktopChanged = $derived(!!desktopDraft && !!desktopActual && (desktopDraft.start_at_login !== desktopActual.start_at_login || desktopDraft.start_minimized !== desktopActual.start_minimized || desktopDraft.hide_to_tray !== desktopActual.hide_to_tray));
  function acceptDesktop(settings: DesktopSettings) {
    desktopActual = settings;
    desktopDraft = { start_at_login: settings.start_at_login, start_minimized: settings.start_minimized, hide_to_tray: settings.hide_to_tray };
    trayConfirmed = false;
  }
  function desktopProblem(cause: unknown) { return cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : 'The native desktop settings request failed.'; }
  async function loadDesktop() {
    if (desktopSaving || desktopReadPending || !mounted) return;
    desktopReadPending = true;
    desktopLoading = true; desktopError = ''; desktopNotice = '';
    try {
      if (!inDesktop()) throw new Error('Open the desktop application to read startup settings.');
      const actual = await invoke<DesktopSettings>('get_desktop_settings');
      if (mounted) acceptDesktop(actual);
    } catch (cause) {
      if (mounted) { desktopActual = null; desktopDraft = null; desktopError = `${desktopProblem(cause)} Retry loading startup settings.`; }
    } finally { desktopReadPending = false; if (mounted) desktopLoading = false; }
  }
  async function saveDesktop(event: SubmitEvent) {
    event.preventDefault();
    if (!desktopDraft || !desktopActual || desktopPending || !desktopChanged || !mounted) return;
    if (!desktopActual.hide_to_tray && desktopDraft.hide_to_tray && !trayConfirmed) { trayDialog = true; return; }
    desktopSaving = true; desktopError = ''; desktopNotice = '';
    try {
      const actual = await invoke<DesktopSettings>('save_desktop_settings', { settings: { ...desktopDraft } });
      if (mounted) { acceptDesktop(actual); desktopNotice = 'Startup settings saved.'; }
    } catch (cause) {
      if (!mounted) return;
      desktopError = `${desktopProblem(cause)} Check desktop login and tray availability before retrying.`;
      // Registration and file persistence can fail independently; reload native truth.
      desktopActual = null; desktopDraft = null;
      try {
        const actual = await invoke<DesktopSettings>('get_desktop_settings');
        if (mounted) { acceptDesktop(actual); desktopError += ' Current settings reloaded.'; }
      } catch (reloadCause) { if (mounted) desktopError += ` ${desktopProblem(reloadCause)} Retry loading startup settings.`; }
    } finally { if (mounted) desktopSaving = false; }
  }
  function changeHideToTray(enabled: boolean) {
    if (!desktopDraft || !desktopActual || desktopPending) return;
    desktopNotice = '';
    if (enabled && !desktopActual.hide_to_tray) { trayConfirmed = false; trayDialog = true; return; }
    desktopDraft.hide_to_tray = enabled;
    if (!enabled) { desktopDraft.start_minimized = false; trayConfirmed = false; }
  }
  function confirmTray() {
    if (!desktopDraft || !desktopActual?.tray_available || !trayConfirmed || desktopPending) return;
    desktopDraft.hide_to_tray = true; trayDialog = false;
  }
  onMount(() => { void loadDesktop(); return () => { mounted = false; }; });
  function open() { draft = copyConfig(status.config); error = ''; }
  function close() { if (!saving) draft = null; }
  async function submit(event: SubmitEvent) {
    event.preventDefault(); if (!draft) return;
    if (draft.revision !== status.config.revision) { error = 'Configuration changed. Cancel and reopen settings before saving.'; return; }
    saving = true; const problem = await save(draft); saving = false;
    if (problem === null) close(); else error = problem;
  }
  async function quit() { quitting = true; try { await invoke('quit'); } catch { quitting = false; quitDialog = false; error = 'Could not quit safely. Cancel active work and try again.'; } }
  function openExport() {
    exportDraft = { revision: status.config.revision, repositories: status.config.repositories.map(repository => ({ ...repository })), repositoryId: status.config.repositories[0]?.id || '' };
    exportConfirmed = false; exportError = ''; exportNotice = '';
  }
  function closeExport() { if (!exporting) exportDraft = null; }
  async function exportPassword(event: SubmitEvent) {
    event.preventDefault();
    if (!exportDraft || !exportConfirmed || exporting || busy) return;
    if (exportDraft.revision !== status.config.revision) { exportError = 'Configuration changed. Cancel and reopen password export before continuing.'; return; }
    exporting = true; exportError = ''; exportNotice = '';
    try {
      const written = await invoke<boolean>('export_repository_password', { repositoryId: exportDraft.repositoryId, revision: exportDraft.revision });
      if (written) { exportDraft = null; exportNotice = 'Repository password exported. Move the file to secure recovery storage.'; }
      else exportNotice = 'Export cancelled. No password file was created.';
    } catch (cause) { exportError = typeof cause === 'string' ? cause : 'Could not export the repository password. Check keychain access and choose a new file in a private recovery folder.'; }
    finally { exporting = false; }
  }
</script>

{#if error && !draft}<p class="message error" role="alert">{error}</p>{/if}
<section><div class="section-heading"><h2>Local engine</h2><button disabled={busy} onclick={open}>Edit engine settings</button></div><dl class="settings-list"><div><dt>Restic executable</dt><dd class="path">{status.config.restic_path}</dd></div><div><dt>Detected version</dt><dd>{status.restic_version || 'Unavailable'} <span class="muted small">· requires 0.17.1 or newer</span></dd></div></dl><p class="hint">Configuration contains repository locations and backup profiles, never repository passwords.</p></section>
<section><h2>System keychain</h2><p class="hint">Repository passwords are generated and stored in the system keychain. If access fails, unlock the keychain using the desktop's keychain tools and retry. Removing a repository from ysynk does not delete its keychain record.</p><p class="hint">Export saves a selected repository password to a plaintext file. Keep a separate recovery copy; losing the keychain and all password copies makes the repository inaccessible.</p><div class="actions"><button disabled={busy || !inDesktop() || !status.config.repositories.length || exporting} onclick={openExport}>Export repository password…</button></div>{#if !status.config.repositories.length}<p class="hint">No repositories configured.</p>{/if}{#if exportNotice && !exportDraft}<p class="message" role="status">{exportNotice}</p>{/if}</section>
<section><h2>Appearance</h2><label class="short-field">Theme<select value={theme} onchange={(event) => changeTheme(event.currentTarget.value)}><option value="system">Follow system</option><option value="light">Light</option><option value="dark">Dark</option></select></label></section>
<section><h2>Startup and window behavior</h2>
  <p class="hint">Start at login runs ysynk after desktop login, not at system boot. Start minimized applies only to login launches; manual launches show the window. Backups require the app to remain running. Jobs cannot run during sleep, network backups require a connection, and locked or unavailable keychain access causes operations to fail.</p>
  {#if desktopError}<p class="message error" role="alert">{desktopError}</p>{/if}
  {#if desktopNotice}<p class="message success" role="status">{desktopNotice}</p>{/if}
  {#if desktopPending}<p class="hint" role="status">{desktopSaving ? 'Saving startup settings…' : 'Loading startup settings…'}</p>{/if}
  {#if desktopDraft && desktopActual}
    <form onsubmit={saveDesktop} aria-busy={desktopPending}>
      <label class="check"><input type="checkbox" bind:checked={desktopDraft.start_at_login} disabled={desktopPending || !desktopActual.autostart_supported} onchange={(event) => { desktopNotice = ''; if (desktopDraft && !event.currentTarget.checked) desktopDraft.start_minimized = false; }}/><span>Start at login</span></label>
      {#if !desktopActual.autostart_supported}<p class="field-hint">Start at login is unavailable for this application instance.</p>{/if}
      <label class="check"><input type="checkbox" bind:checked={desktopDraft.start_minimized} disabled={desktopPending || !desktopDraft.start_at_login || !desktopDraft.hide_to_tray || !desktopActual.tray_available} onchange={() => desktopNotice = ''}/><span>Start minimized<span class="field-hint">Login launches only. Requires Start at login, Hide to tray and an accessible tray icon.</span></span></label>
      <label class="check"><input type="checkbox" checked={desktopDraft.hide_to_tray} disabled={desktopPending || (!desktopActual.tray_available && !desktopDraft.hide_to_tray)} onchange={(event) => { changeHideToTray(event.currentTarget.checked); event.currentTarget.checked = desktopDraft?.hide_to_tray ?? false; }}/><span>Hide to tray when the window closes</span></label>
      {#if !desktopActual.tray_available}<p class="field-hint">No accessible tray host detected. The window cannot be hidden; closing it quits the application.</p>{/if}
      <div class="actions"><button class="primary" disabled={desktopPending || !desktopChanged}>{desktopSaving ? 'Saving…' : 'Save startup settings'}</button></div>
    </form>
  {:else if !desktopPending}<button onclick={loadDesktop}>Retry loading startup settings</button>{/if}
  <div class="actions"><button disabled={!inDesktop() || quitting || desktopPending} onclick={() => quitDialog = true}>Quit ysynk…</button></div>
</section>

{#if draft}<Dialog title="Engine settings" {close}><form onsubmit={submit}>{#if error}<p class="message error" role="alert">{error}</p>{/if}<label>Restic executable path<input required class="path" bind:value={draft.restic_path} placeholder="/usr/bin/restic"/><span class="field-hint">Use a trusted installed executable, version 0.17.1 or newer.</span></label><div class="dialog-actions"><button type="button" disabled={saving} onclick={close}>Cancel</button><button class="primary" disabled={busy || saving}>{saving ? 'Saving…' : 'Save settings'}</button></div></form></Dialog>{/if}
{#if trayDialog}<Dialog title="Confirm tray visibility" close={() => trayDialog = false}><p class="dialog-intro">Check that the ysynk icon is visible and accessible in the desktop tray. Confirm here, then save startup settings to persist this choice.</p><label class="check"><input type="checkbox" bind:checked={trayConfirmed}/><span>I can see and access the ysynk tray icon.</span></label><div class="dialog-actions"><button onclick={() => trayDialog = false}>Cancel</button><button class="primary" disabled={!trayConfirmed || !desktopActual?.tray_available || desktopPending} onclick={confirmTray}>Confirm tray visibility</button></div></Dialog>{/if}
{#if quitDialog}<Dialog title="Quit ysynk?" close={() => { if (!quitting) quitDialog = false; }}><p class="dialog-intro">{status.active ? 'The active operation will be cancelled and its processes drained before ysynk exits. An incomplete restore output will be preserved.' : 'Background scheduling stops when the app exits. Repository passwords remain in the system keychain.'}</p><div class="dialog-actions"><button disabled={quitting} onclick={() => quitDialog = false}>Keep running</button><button class="danger" disabled={quitting} onclick={quit}>{quitting ? 'Finishing shutdown…' : 'Quit ysynk'}</button></div></Dialog>{/if}
{#if exportDraft}<Dialog title="Export repository password" close={closeExport}><form onsubmit={exportPassword}>{#if exportError}<p class="message error" role="alert">{exportError}</p>{/if}{#if exportNotice}<p class="message" role="status">{exportNotice}</p>{/if}<label>Repository<select required bind:value={exportDraft.repositoryId} disabled={exporting} onchange={() => { exportConfirmed = false; exportError = ''; exportNotice = ''; }}>{#each exportDraft.repositories as repository}<option value={repository.id}>{repository.label} — {repository.location}</option>{/each}</select></label><p class="message warning">The exported file is not encrypted. Anyone who can read it and access the repository can decrypt backups. The exported password remains usable after the system keychain is locked.</p><p class="hint">Choose a new file in a private recovery folder outside backup sources, repositories and application data. Existing files will not be overwritten. On Windows, the file inherits the destination folder's permissions. Move it to offline storage or a password manager, then remove any unneeded plaintext copies.</p><label class="check"><input type="checkbox" bind:checked={exportConfirmed} disabled={exporting}/><span>I understand that this exports the repository password as plaintext.</span></label><div class="dialog-actions"><button type="button" disabled={exporting} onclick={closeExport}>Cancel</button><button class="primary" disabled={!exportConfirmed || !exportDraft.repositoryId || busy || exporting}>{exporting ? 'Exporting…' : 'Choose file and export…'}</button></div></form></Dialog>{/if}
