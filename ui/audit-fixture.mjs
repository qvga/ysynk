// Offline browser fixture: bundles the real UI with an in-memory IPC substitute.
// Run after npm run build; never connects to a native application or user data.
import { readFileSync, readdirSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { verifyAuditUi } from './audit-interactions.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const assets = resolve(root, 'dist/assets');
const readAsset = extension => readFileSync(resolve(assets, readdirSync(assets).find(name => name.endsWith(extension))), 'utf8');
const mock = () => {
  const repository = { id: 'local', label: 'Audit repository', location: '/audit/repository ' };
  const profile = { id: 'documents', label: 'Audit documents', sources: ['/audit/Documents '], excludes: [' **/draft '], repository_ids: ['local'], schedule_minutes: 0, retention: { keep_last: 5, keep_daily: 7, keep_weekly: 4, keep_monthly: 6 } };
  const status = { config: { revision: 1, restic_path: '/usr/bin/restic', repositories: [repository], profiles: [profile] }, active: null, history: [], restic_version: 'restic 0.18.1 (mock)', setup_error: null };
  const requests = [];
  let serial = 0;
  let heldStatus = null;
  let holdNext = false;
  let nextCancellation = null;
  const clone = value => JSON.parse(JSON.stringify(value));
  function finish(state = 'success', result = null) {
    if (!status.active) throw new Error('No mock operation is active');
    status.history.unshift({ ...status.active, state, result, finished_at: 1793664060, message: state === 'success' ? 'Completed (mock)' : 'Cancelled (mock); incomplete output preserved' });
    status.active = null;
  }
  globalThis.audit = {
    status, requests, finish,
    appendOutput(lines, truncated = false) {
      if (!status.active) throw new Error('No mock operation is active');
      status.active.output = [...status.active.output, ...lines].slice(-200);
      status.active.output_truncated ||= truncated;
    },
    holdNextStatus() { holdNext = true; },
    releaseStatus() { if (!heldStatus) throw new Error('No status held'); const held = heldStatus; heldStatus = null; held.resolve(held.value); },
    get statusHeld() { return !!heldStatus; },
    deferNextCancellation() {
      let release, fail;
      nextCancellation = new Promise((resolve, reject) => {
        release = resolve;
        fail = () => reject(new Error('Cancellation request failed (mock).'));
      });
      return { release, fail };
    },
    evict() { status.active = null; status.history = Array.from({ length: 100 }, (_, index) => ({ id: `new-${index}`, kind: 'backup', label: 'Scheduled backup (mock)', state: 'success', started_at: 1793664000, finished_at: 1793664060, profile_id: null, repository_id: 'local', message: 'Completed (mock)', progress: null, destinations: [], result: null, output: [], output_truncated: false })); },
  };
  globalThis.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = { async invoke(command, args) {
    requests.push(clone({ command, args }));
    if (command === 'get_desktop_settings') return { start_at_login: false, start_minimized: false, hide_to_tray: false, tray_available: true, autostart_supported: false };
    if (command !== 'request') throw new Error(`Unsupported fixture command: ${command}`);
    const request = args.request;
    if (request.op === 'status') {
      const value = clone(status);
      if (holdNext) { holdNext = false; return new Promise(resolve => { heldStatus = { resolve, value }; }); }
      return value;
    }
    if (request.op === 'cancel') { finish('cancelled'); return null; }
    if (request.op === 'cancel_job') {
      const deferred = nextCancellation;
      nextCancellation = null;
      if (deferred) await deferred;
      if (status.active?.id !== request.job_id) throw new Error('The selected operation is no longer active.');
      return null;
    }
    if (request.op === 'save_config') { status.config = clone(request.config); status.config.revision++; return clone(status); }
    if (status.active) throw new Error('Another mock job is running');
    const id = `audit-${++serial}`;
    status.active = { id, kind: request.op, label: `${request.op} (mock)`, state: 'running', started_at: 1793664000, finished_at: null, profile_id: null, repository_id: 'local', message: 'Operation running (mock)', progress: null, destinations: [], result: null, output: [], output_truncated: false };
    if (request.op === 'snapshots') finish('success', [
      { id: 'abcdef1234567890', time: '2026-11-03T12:00:00Z', paths: ['/audit/Documents '], hostname: 'fixture', tags: ['documents'], summary: { total_bytes_processed: 2048, total_files_processed: 3, files_new: 0, files_changed: 1, files_unmodified: 2, data_added: 1024, data_added_packed: 512, backup_start: '2026-11-03T12:00:00Z', backup_end: '2026-11-03T12:01:05Z' } },
      { id: 'older1234567890', time: '2026-11-02T12:00:00Z', paths: ['/audit/Older'], hostname: 'older-host' },
    ]);
    if (request.op === 'list_files') finish('success', [{ type: 'file', name: 'draft.txt ', path: '/draft.txt ', size: 128 }]);
    return { job_id: id };
  } };
};

const output = resolve(root, 'target/audit-ui', process.argv[2] || 'index.html');
mkdirSync(resolve(root, 'target/audit-ui'), { recursive: true });
const script = `(${mock.toString()})();\nglobalThis.verifyAuditUi = ${verifyAuditUi.toString()};\n${readAsset('.js')}`.replace(/<\/script/gi, '<\\/script');
writeFileSync(output, `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>ysynk — isolated mock audit</title><style>${readAsset('.css')}</style></head><body><div id="app"></div><script type="module">${script}</script></body></html>`);
console.log(output);
