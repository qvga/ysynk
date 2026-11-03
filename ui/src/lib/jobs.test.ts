import { describe, expect, it } from 'vitest';
import { JobCompletions } from './jobs';
import type { Job } from './types';

const job = (id: string, state = 'success') => ({ id, state } as Job);

describe('job completion reconciliation', () => {
  it('ignores a status request started before admission even when it arrives later', () => {
    const pending = new JobCompletions();
    void pending.wait('a', 4);
    pending.accept({ active: null, history: [] }, 4);
    expect(pending.size).toBe(1);
    pending.accept({ active: job('a', 'running'), history: [] }, 5);
    expect(pending.size).toBe(1);
  });

  it('rejects an evicted completion on a post-admission status and permits another job', async () => {
    const pending = new JobCompletions();
    const first = pending.wait('a', 4);
    const rejected = expect(first).rejects.toThrow('no longer available');
    pending.accept({ active: null, history: Array.from({ length: 100 }, (_, index) => job(`new-${index}`)) }, 5);
    await rejected;
    expect(pending.size).toBe(0);
    const next = pending.wait('b', 5);
    const completed = job('b');
    pending.accept({ active: null, history: [completed] }, 6);
    await expect(next).resolves.toBe(completed);
  });

  it('accepts a quick completion from a post-admission status', async () => {
    const pending = new JobCompletions();
    const result = pending.wait('a', 1);
    const completed = job('a');
    pending.accept({ active: null, history: [completed] }, 2);
    await expect(result).resolves.toBe(completed);
    expect(pending.size).toBe(0);
  });

  it('rejects pending jobs when the interface closes', async () => {
    const pending = new JobCompletions();
    const result = pending.wait('a', 1);
    const rejected = expect(result).rejects.toThrow('interface closed');
    pending.close();
    await rejected;
    expect(pending.size).toBe(0);
  });
});
