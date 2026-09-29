import { MOCK_SCHEMA } from '../lib/backend/mock';
import { settings } from '../lib/settings.svelte';
import type { SettingsPatch } from '../lib/types';
import { FakeBackend } from './fake-backend';

let unsubscribe: (() => void) | undefined;

/** Connects the app-wide settings store to a fresh in-memory backend, optionally seeded with `patch`. */
export async function connectSettings(patch?: SettingsPatch): Promise<FakeBackend> {
  disconnectSettings();
  const backend = new FakeBackend(MOCK_SCHEMA);
  unsubscribe = await settings.connect(backend);
  if (patch) await settings.update(patch);
  return backend;
}

/** Detaches the store and forgets its state, so the next test starts from nothing. */
export function disconnectSettings(): void {
  unsubscribe?.();
  unsubscribe = undefined;
  settings.state = null;
  settings.errors = {};
}
