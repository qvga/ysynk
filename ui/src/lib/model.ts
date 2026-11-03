import type { Config, Job, Snapshot, FileNode, RetentionPreview } from './types';

export function lines(value: string): string[] { return value.split(/\r?\n/).filter(line => line.length > 0); }
export function copyConfig(config: Config): Config { return JSON.parse(JSON.stringify(config)) as Config; }
export function bytes(value?: number): string {
  if (value == null || !Number.isFinite(value)) return 'Not reported';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(Math.floor(Math.log(Math.max(value, 1)) / Math.log(1024)), units.length - 1);
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: index ? 1 : 0 }).format(value / 1024 ** index)} ${units[index]}`;
}
export function date(value: string | number | null): string {
  if (value == null) return 'Not reported';
  const result = new Date(typeof value === 'number' ? value * 1000 : value);
  return Number.isNaN(result.getTime()) ? 'Not reported' : result.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
export function assertSuccess(job: Job): unknown {
  if (job.state !== 'success') throw new Error(job.message || `Operation ${job.state}. See activity for details.`);
  if (job.result == null) throw new Error('This result is no longer available. Run the query again.');
  return job.result;
}
export function snapshots(result: unknown): Snapshot[] {
  if (!Array.isArray(result) || !result.every(item => typeof item?.id === 'string' && typeof item?.time === 'string')) throw new Error('Snapshot result is unavailable or invalid. Load snapshots again.');
  return result as Snapshot[];
}
export function files(result: unknown): FileNode[] {
  if (!Array.isArray(result) || !result.every(item => typeof item?.path === 'string' && typeof item?.type === 'string')) throw new Error('Directory result is unavailable or invalid. Open the directory again.');
  return result as FileNode[];
}
export function preview(result: unknown): RetentionPreview {
  const data = result as RetentionPreview | null;
  if (!data || typeof data.token !== 'string' || !Number.isInteger(data.keep_count)) throw new Error('Preview is unavailable. Run the preview again.');
  return { ...data, remove: snapshots(data.remove) };
}
export function isCurrent(ticket: number, current: number, scope: string, selection: string): boolean { return ticket === current && scope === selection; }
