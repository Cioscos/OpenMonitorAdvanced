import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { HistorySeed, Schema, Snapshot, StartupStatus } from '../types';
import type { Backend } from './backend';

/** Command and event names are defined in app/src-tauri (commands.rs, main.rs). */
export function createTauriBackend(): Backend {
  return {
    getSchema: () => invoke<Schema>('get_schema'),
    getHistory: (ids, seconds) => invoke<HistorySeed>('get_history', { ids, seconds }),
    onSchema: (cb) => listen<Schema>('oma:schema', (e) => cb(e.payload)),
    onSnapshot: (cb) => listen<Snapshot>('oma:snapshot', (e) => cb(e.payload)),
    getStartupStatus: () => invoke<StartupStatus>('get_startup_status'),
    enableVendorLibraries: () => invoke<StartupStatus>('enable_vendor_libraries'),
  };
}
