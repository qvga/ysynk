import { describe, expect, it } from 'vitest';
import { assertSuccess, files, isCurrent, lines, preview, snapshots } from './model';
import type { Job } from './types';

describe('query results', () => {
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
  it('preserves literal file paths and requires a server retention token', () => {
    const nodes = [{ type: 'file', name: '[draft]*.txt', path: '/Documents/[draft]*.txt', size: 17 }];
    expect(files(nodes)[0].path).toBe('/Documents/[draft]*.txt');
    expect(preview({ token: 'server-token', remove: [], keep_count: 4 })).toEqual({ token: 'server-token', remove: [], keep_count: 4 });
    expect(() => preview({ remove: [], keep_count: 4 })).toThrow('unavailable');
    expect(() => snapshots({ error: 'not an array' })).toThrow('unavailable');
  });
});
