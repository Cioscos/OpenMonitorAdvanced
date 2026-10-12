import { flushSync } from 'svelte';
import { MOCK_SCHEMA } from '../backend/mock';
import { settings } from '../settings.svelte';
import { FakeBackend } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { theme } from './theme.svelte';

afterEach(() => {
  disconnectSettings();
  document.documentElement.removeAttribute('style');
});

test('load_bumps_version_and_keeps_fallback_flag', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.themeResolved = async () => ({ id: 'builtin-synthwave', vars: { '--accent': '#abcdef' }, fallback: true });
  const before = theme.version;
  await theme.load(backend);
  expect(theme.version).toBe(before + 1);
  expect(theme.fallback).toBe(true);
  expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#abcdef');
  backend.themeResolved = async () => ({ id: 'builtin-midnight', vars: {}, fallback: false });
  await theme.load(backend);
  expect(theme.version).toBe(before + 2);
  expect(theme.fallback).toBe(false);
  expect(document.documentElement.style.getPropertyValue('--accent')).toBe('');
});

test('follow_reloads_only_when_theme_id_changes', async () => {
  const backend = await connectSettings();
  const resolved = vi.spyOn(backend, 'themeResolved');
  const stop = theme.follow(backend, settings);
  try {
    flushSync();
    // The current theme is the one applied before mounting.
    expect(resolved).not.toHaveBeenCalled();
    await settings.update({ general: { language: 'it' } });
    flushSync();
    expect(resolved).not.toHaveBeenCalled();
    await settings.update({ appearance: { theme: 'builtin-daylight' } });
    flushSync();
    expect(resolved).toHaveBeenCalledTimes(1);
    await settings.update({ general: { language: 'en' } });
    flushSync();
    expect(resolved).toHaveBeenCalledTimes(1);
  } finally {
    stop();
  }
  await settings.update({ appearance: { theme: 'builtin-synthwave' } });
  flushSync();
  expect(resolved).toHaveBeenCalledTimes(1);
});
