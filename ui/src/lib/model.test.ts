import { describe, expect, it } from 'vitest';
import { assertSuccess, files, isCurrent, lines, newestSnapshots, preview, snapshots, snapshotStats } from './model';
import type { Job } from './types';

describe('query results', () => {
  it('shows reported snapshot totals and keeps zero file counts', () => {
    expect(snapshotStats({ id: 'one', time: '', summary: { total_bytes_processed: 2048, total_files_processed: 3, files_new: 0, files_changed: 1, files_unmodified: 2, data_added: 1024, data_added_packed: 512, backup_start: '2026-11-03T12:00:00Z', backup_end: '2026-11-03T12:01:05Z' } })).toEqual([
      { label: 'Data processed', value: '2 KiB' }, { label: 'Files processed', value: '3' },
      { label: 'New files', value: '0' }, { label: 'Changed files', value: '1' }, { label: 'Unchanged files', value: '2' },
      { label: 'Data added (uncompressed)', value: '1 KiB' }, { label: 'Data added (compressed)', value: '512 B' },
      { label: 'Backup duration', value: '1 min 5 s' },
    ]);
  });
  it('omits absent and invalid snapshot statistics', () => {
    expect(snapshotStats({ id: 'old', time: '' })).toEqual([]);
    expect(snapshotStats({ id: 'old', time: '', summary: null })).toEqual([]);
    expect(snapshotStats({ id: 'partial', time: '', summary: { files_new: -1, total_bytes_processed: NaN, total_files_processed: Infinity, backup_start: 'invalid', backup_end: 'invalid' } })).toEqual([]);
    expect(snapshotStats({ id: 'partial', time: '', summary: { files_changed: 0, backup_start: '2026-11-03T12:01:00Z', backup_end: '2026-11-03T12:00:00Z' } })).toEqual([{ label: 'Changed files', value: '0' }]);
  });
  it('preserves spaces in source paths and exclusion patterns when separating lines', () => {
    expect(lines('/home/user/Documents \r\n\n **/draft \n  ')).toEqual(['/home/user/Documents ', ' **/draft ', '  ']);
  });
  it('rejects a completion when the repository or request has changed', () => {
    expect(isCurrent(2, 2, 'repo-a:snapshot-a:/', 'repo-b:snapshot-a:/')).toBe(false);
    expect(isCurrent(2, 3, 'repo-a:snapshot-a:/', 'repo-a:snapshot-a:/')).toBe(false);
    expect(isCurrent(2, 2, 'repo-a:snapshot-a:/', 'repo-a:snapshot-a:/')).toBe(true);
  });
  it('rejects failed queries and completed queries without a retained result', () => {
    expect(() => assertSuccess({ state: 'failed', message: 'System keychain is unavailable. Restore access and retry.', result: [] } as unknown as Job)).toThrow('System keychain is unavailable. Restore access and retry.');
    expect(() => assertSuccess({ state: 'success', result: null } as Job)).toThrow('no longer available');
    expect(assertSuccess({ state: 'success', result: [] } as unknown as Job)).toEqual([]);
  });
  it('orders snapshots by the instant of backup across host time zones', () => {
    const entries = [
      { id: 'unknown', time: 'invalid' },
      { id: 'earlier', time: '2026-11-03T13:15:00+02:00' },
      { id: 'later', time: '2026-11-03T07:45:00-05:00' },
    ];
    expect(newestSnapshots(entries).map(item => item.id)).toEqual(['later', 'earlier', 'unknown']);
    expect(entries[0].id).toBe('unknown');
  });

  it('handles optional snapshot metadata before rendering it', () => {
    const snapshot = { id: 'old', time: '2026-11-03T12:00:00Z', paths: null, tags: null };
    expect(snapshots([snapshot])).toEqual([snapshot]);
    expect(() => snapshots([{ ...snapshot, tags: 'documents' }])).toThrow('invalid');
    expect(() => snapshots([{ ...snapshot, paths: [null] }])).toThrow('invalid');
  });
  it('preserves literal file paths and requires a server retention token', () => {
    const nodes = [{ type: 'file', name: '[draft]*.txt', path: '/Documents/[draft]*.txt', size: 17 }];
    expect(files(nodes)[0].path).toBe('/Documents/[draft]*.txt');
    expect(preview({ token: 'server-token', remove: [], keep_count: 4 })).toEqual({ token: 'server-token', remove: [], keep_count: 4 });
    expect(() => preview({ remove: [], keep_count: 4 })).toThrow('unavailable');
    expect(() => snapshots({ error: 'not an array' })).toThrow('unavailable');
  });
});
