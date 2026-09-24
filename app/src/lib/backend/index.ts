import { isTauri } from '@tauri-apps/api/core';
import type { Backend } from './backend';
import { createMockBackend } from './mock';
import { createTauriBackend } from './tauri';

export type { Backend, Unsubscribe } from './backend';

export function createBackend(): Backend {
  return isTauri() ? createTauriBackend() : createMockBackend();
}
