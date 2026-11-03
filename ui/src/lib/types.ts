export interface Repository { id: string; label: string; location: string }
export interface DesktopPreferences { start_at_login: boolean; start_minimized: boolean; hide_to_tray: boolean }
export interface DesktopSettings extends DesktopPreferences { tray_available: boolean; autostart_supported: boolean }
export interface Retention { keep_last: number; keep_daily: number; keep_weekly: number; keep_monthly: number }
export interface Profile { id: string; label: string; sources: string[]; excludes: string[]; repository_ids: string[]; schedule_minutes: number; retention: Retention }
export interface Config { revision: number; restic_path: string; repositories: Repository[]; profiles: Profile[] }
export interface Progress { percent: number | null; bytes_done: number; total_bytes: number; files_done: number; total_files: number }
export interface DestinationResult { repository_id: string; state: string; message: string; snapshot_id: string | null }
export interface Job { id: string; kind: string; label: string; state: string; started_at: number; finished_at: number | null; profile_id: string | null; repository_id: string | null; message: string; progress: Progress | null; destinations: DestinationResult[]; result: unknown | null }
export interface Status { config: Config; active: Job | null; history: Job[]; restic_version: string | null; setup_error: string | null }
export interface Snapshot { id: string; time: string; paths?: string[]; tags?: string[]; hostname?: string; summary?: { total_bytes_processed?: number } }
export interface FileNode { type: string; name: string; path: string; size?: number }
export interface RetentionPreview { token: string; remove: Snapshot[]; keep_count: number }
export type Request =
  | { op: 'status' | 'cancel' }
  | { op: 'save_config'; config: Config }
  | { op: 'initialize' | 'snapshots' | 'stats' | 'check'; repository_id: string }
  | { op: 'backup'; profile_id: string }
  | { op: 'list_files'; repository_id: string; snapshot_id: string; path: string }
  | { op: 'restore'; repository_id: string; snapshot_id: string; paths: string[]; target: string }
  | { op: 'retention_preview'; repository_id: string; profile_id: string }
  | { op: 'apply_retention'; token: string; confirmation: string }
  | { op: 'prune'; repository_id: string; confirmation: string };
export type Run = (request: Request) => Promise<Job>;
// null means saved; a message belongs in the active dialog, not behind it.
export type Save = (config: Config) => Promise<string | null>;
