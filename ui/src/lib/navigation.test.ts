import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import App from '../App.svelte';
import Activity from './Activity.svelte';
import Preferences from './Preferences.svelte';
import type { Job, Status } from './types';

describe('navigation and operation history', () => {
  it('opens Profiles and provides only the four configuration and restore views', () => {
    const { body } = render(App);
    expect(body).toMatch(/aria-label="Profiles" aria-current="page"/);
    for (const name of ['Profiles', 'Repositories', 'Snapshots', 'Settings']) {
      expect(body).toContain(`aria-label="${name}"`);
    }
    expect(body).not.toContain('Overview');
    expect(body).not.toContain('Manage profiles');
    expect(body).not.toContain('Set up your first backup');
  });

  it('shows an explicit empty operation history', () => {
    const { body } = render(Activity, { props: { jobs: [], repositories: [] } });
    expect(body).toContain('No completed operations.');
  });

  it('keeps successful and failed destination evidence in operation history', () => {
    const job: Job = {
      id: 'backup-1', kind: 'backup', label: 'Documents backup', state: 'partial',
      started_at: 1793664000, finished_at: 1793664060,
      profile_id: 'documents', repository_id: null, message: 'One destination failed.',
      progress: null, result: null,
      destinations: [
        { repository_id: 'local', state: 'success', message: 'Snapshot created.', snapshot_id: 'abcdef1234567890' },
        { repository_id: 'remote', state: 'failed', message: 'System keychain is locked.', snapshot_id: null },
      ],
    };
    const { body } = render(Activity, { props: { jobs: [job], repositories: [{ id: 'local', label: 'Local disk', location: '/backup' }] } });
    for (const evidence of ['partial', 'success', 'failed', 'Local disk', 'remote', 'abcdef123456', 'System keychain is locked.']) {
      expect(body).toContain(evidence);
    }
  });
});

describe('startup settings', () => {
  it('shows loading without inventing startup preferences before native settings arrive', () => {
    const status: Status = {
      config: { revision: 0, restic_path: '/usr/bin/restic', repositories: [], profiles: [] },
      active: null, history: [], restic_version: 'restic 0.18.1', setup_error: null,
    };
    const { body } = render(Preferences, { props: { status, busy: false, save: async () => null, theme: 'system', changeTheme: () => {} } });
    expect(body).toContain('Loading startup settings…');
    expect(body).not.toContain('type="checkbox"');
    expect(body).not.toContain('Save startup settings');
    expect(body).toContain('manual launches show the window');
    expect(body).not.toContain('Applies to this running application session');
  });
});
