import type { Job, Status } from './types';

/** Reconcile only polls initiated after the native job admission response. */
export class JobCompletions {
  private pending = new Map<string, { after: number; resolve: (job: Job) => void; reject: (error: Error) => void }>();

  get size() { return this.pending.size; }

  wait(id: string, after: number): Promise<Job> {
    return new Promise((resolve, reject) => this.pending.set(id, { after, resolve, reject }));
  }

  accept(status: Pick<Status, 'active' | 'history'>, sequence: number) {
    for (const [id, waiter] of this.pending) {
      if (sequence <= waiter.after) continue;
      const job = status.history.find(job => job.id === id);
      if (job && job.state !== 'running') {
        this.pending.delete(id);
        waiter.resolve(job);
      } else if (!job && status.active?.id !== id) {
        this.pending.delete(id);
        waiter.reject(new Error('The operation result is no longer available. Inspect operation history before trying again.'));
      }
    }
  }

  close() {
    for (const waiter of this.pending.values()) waiter.reject(new Error('The interface closed before the operation completed.'));
    this.pending.clear();
  }
}
