import type { Config, Job, Snapshot, FileNode, RetentionPreview } from './types';

export function lines(value: string): string[] { return value.split(/\r?\n/).filter(line => line.length > 0); }
export function copyConfig(config: Config): Config { return JSON.parse(JSON.stringify(config)) as Config; }
const wholeNumber = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
const decimalNumber = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const timestamp = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

export function bytes(value?: number): string {
  if (value == null || !Number.isFinite(value) || value < 0) return 'Not reported';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(Math.floor(Math.log(Math.max(value, 1)) / Math.log(1024)), units.length - 1);
  const formatter = index === 0 ? wholeNumber : decimalNumber;
  return `${formatter.format(value / 1024 ** index)} ${units[index]}`;
}
export function date(value: string | number | null): string {
  if (value == null) return 'Not reported';
  const result = new Date(typeof value === 'number' ? value * 1000 : value);
  return Number.isNaN(result.getTime()) ? 'Not reported' : timestamp.format(result);
}
export function snapshotStats(snapshot: Snapshot): { label: string; value: string }[] {
  const summary = snapshot.summary;
  if (!summary) return [];
  const fields = [
    ['total_bytes_processed', 'Data processed', bytes],
    ['total_files_processed', 'Files processed', wholeNumber.format],
    ['files_new', 'New files', wholeNumber.format],
    ['files_changed', 'Changed files', wholeNumber.format],
    ['files_unmodified', 'Unchanged files', wholeNumber.format],
    ['data_added', 'Data added (uncompressed)', bytes],
    ['data_added_packed', 'Data added (compressed)', bytes],
  ] as const;
  const stats: { label: string; value: string }[] = fields.flatMap(([key, label, format]) => {
    const value = summary[key];
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? [{ label, value: format(value) }] : [];
  });
  const start = Date.parse(summary.backup_start ?? '');
  const end = Date.parse(summary.backup_end ?? '');
  if (Number.isFinite(start) && Number.isFinite(end) && end >= start) {
    const seconds = Math.round((end - start) / 1000);
    const duration = seconds < 60 ? `${seconds} s` : seconds < 3600 ? `${Math.floor(seconds / 60)} min ${seconds % 60} s` : `${Math.floor(seconds / 3600)} h ${Math.floor(seconds % 3600 / 60)} min`;
    stats.push({ label: 'Backup duration', value: duration });
  }
  return stats;
}
export function jobTitle(job: Job, config: Pick<Config, 'profiles' | 'repositories'>): string {
  const label = (job.label || job.kind).replaceAll('_', ' ');
  const operation = label.charAt(0).toUpperCase() + label.slice(1);
  const subject = job.profile_id
    ? config.profiles.find(profile => profile.id === job.profile_id)?.label ?? job.profile_id
    : config.repositories.find(repo => repo.id === job.repository_id)?.label ?? job.repository_id;
  return subject && subject !== label ? `${operation} · ${subject}` : operation;
}

export function assertSuccess(job: Job): unknown {
  if (job.state !== 'success') throw new Error(job.message || `Operation ${job.state}. See activity for details.`);
  if (job.result == null) throw new Error('This result is no longer available. Run the query again.');
  return job.result;
}
export function snapshots(result: unknown): Snapshot[] {
  const stringList = (value: unknown) => value == null ||
    (Array.isArray(value) && value.every(item => typeof item === 'string'));
  if (!Array.isArray(result) || !result.every(item =>
    typeof item?.id === 'string' &&
    typeof item.time === 'string' &&
    (item.hostname == null || typeof item.hostname === 'string') &&
    stringList(item.paths) && stringList(item.tags)
  )) {
    throw new Error('Snapshot result is unavailable or invalid. Load snapshots again.');
  }
  return result as Snapshot[];
}

export function newestSnapshots(result: unknown): Snapshot[] {
  return snapshots(result)
    .map(snapshot => {
      const instant = Date.parse(snapshot.time);
      return { snapshot, time: Number.isFinite(instant) ? instant : -Infinity };
    })
    .sort((a, b) => b.time - a.time)
    .map(item => item.snapshot);
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
