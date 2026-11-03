import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Request } from './types';

export async function request<T>(request: Request): Promise<T> {
  if (!isTauri()) throw new Error('Open the ysynk desktop application to connect to the local backup engine. Browser access cannot run backups or read repositories.');
  try { return await invoke<T>('request', { request }); }
  catch (cause) { throw cause instanceof Error ? cause : new Error(typeof cause === 'string' ? cause : 'The local engine could not complete this request. Check its status and try again.'); }
}

export const inDesktop = isTauri;
