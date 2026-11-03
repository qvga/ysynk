<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { inDesktop } from './client';
  import Dialog from './Dialog.svelte';
  import type { Config, Repository, Save, Status } from './types';
  import { copyConfig } from './model';
  let { status, busy, save, theme, changeTheme, tray, changeTray }: { status: Status; busy: boolean; save: Save; theme: string; changeTheme: (theme: string) => void; tray: boolean; changeTray: (enabled: boolean) => void } = $props();
  let draft = $state<Config | null>(null); let saving = $state(false); let error = $state(''); let trayConfirmed = $state(false); let quitting = $state(false); let trayDialog = $state(false); let quitDialog = $state(false);
  let exportDraft = $state<{ revision: number; repositories: Repository[]; repositoryId: string } | null>(null);
  let exportConfirmed = $state(false); let exporting = $state(false); let exportError = $state(''); let exportNotice = $state('');
  function open() { draft = copyConfig(status.config); error = ''; }
  function close() { if (!saving) draft = null; }
  async function submit(event: SubmitEvent) {
    event.preventDefault(); if (!draft) return;
    if (draft.revision !== status.config.revision) { error = 'Configuration changed. Cancel and reopen settings before saving.'; return; }
    saving = true; const problem = await save(draft); saving = false;
    if (problem === null) close(); else error = problem;
  }
  async function setTray(enabled: boolean) {
    error = '';
    try { await invoke('set_hide_to_tray', { enabled }); changeTray(enabled); trayDialog = false; trayConfirmed = false; }
    catch { error = 'Could not change close behavior. The desktop tray may be unavailable. Leave hide-to-tray disabled.'; }
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
<section><h2>Window and background operation</h2><p class="hint">By default, closing the window quits ysynk safely. An active operation is cancelled and drained before exit. Scheduled backups run only while the application remains running.</p><label class="check"><input type="checkbox" checked={tray} disabled={!inDesktop()} onchange={(event) => { if (event.currentTarget.checked) { trayConfirmed = false; trayDialog = true; } else void setTray(false); }}/><span>Hide to tray when the window closes<span class="field-hint">Enable only after confirming the ysynk tray icon is visible. Applies to this running application session.</span></span></label><div class="actions"><button disabled={!inDesktop() || quitting} onclick={() => quitDialog = true}>Quit ysynk…</button></div></section>

{#if draft}<Dialog title="Engine settings" {close}><form onsubmit={submit}>{#if error}<p class="message error" role="alert">{error}</p>{/if}<label>Restic executable path<input required class="path" bind:value={draft.restic_path} placeholder="/usr/bin/restic"/><span class="field-hint">Use a trusted installed executable, version 0.17.1 or newer.</span></label><div class="dialog-actions"><button type="button" disabled={saving} onclick={close}>Cancel</button><button class="primary" disabled={busy || saving}>{saving ? 'Saving…' : 'Save settings'}</button></div></form></Dialog>{/if}
{#if trayDialog}<Dialog title="Confirm tray visibility" close={() => trayDialog = false}><p class="dialog-intro">A hidden window must remain reachable. Check that the ysynk icon is actually visible in your desktop's tray before enabling this option.</p><label class="check"><input type="checkbox" bind:checked={trayConfirmed}/><span>I can see and access the ysynk tray icon.</span></label><div class="dialog-actions"><button onclick={() => trayDialog = false}>Cancel</button><button class="primary" disabled={!trayConfirmed} onclick={() => setTray(true)}>Enable hide to tray</button></div></Dialog>{/if}
{#if quitDialog}<Dialog title="Quit ysynk?" close={() => { if (!quitting) quitDialog = false; }}><p class="dialog-intro">{status.active ? 'The active operation will be cancelled and its processes drained before ysynk exits. An incomplete restore output will be preserved.' : 'Background scheduling stops when the app exits. Repository passwords remain in the system keychain.'}</p><div class="dialog-actions"><button disabled={quitting} onclick={() => quitDialog = false}>Keep running</button><button class="danger" disabled={quitting} onclick={quit}>{quitting ? 'Finishing shutdown…' : 'Quit ysynk'}</button></div></Dialog>{/if}
{#if exportDraft}<Dialog title="Export repository password" close={closeExport}><form onsubmit={exportPassword}>{#if exportError}<p class="message error" role="alert">{exportError}</p>{/if}{#if exportNotice}<p class="message" role="status">{exportNotice}</p>{/if}<label>Repository<select required bind:value={exportDraft.repositoryId} disabled={exporting} onchange={() => { exportConfirmed = false; exportError = ''; exportNotice = ''; }}>{#each exportDraft.repositories as repository}<option value={repository.id}>{repository.label} — {repository.location}</option>{/each}</select></label><p class="message warning">The exported file is not encrypted. Anyone who can read it and access the repository can decrypt backups. The exported password remains usable after the system keychain is locked.</p><p class="hint">Choose a new file in a private recovery folder outside backup sources, repositories and application data. Existing files will not be overwritten. On Windows, the file inherits the destination folder's permissions. Move it to offline storage or a password manager, then remove any unneeded plaintext copies.</p><label class="check"><input type="checkbox" bind:checked={exportConfirmed} disabled={exporting}/><span>I understand that this exports the repository password as plaintext.</span></label><div class="dialog-actions"><button type="button" disabled={exporting} onclick={closeExport}>Cancel</button><button class="primary" disabled={!exportConfirmed || !exportDraft.repositoryId || busy || exporting}>{exporting ? 'Exporting…' : 'Choose file and export…'}</button></div></form></Dialog>{/if}
