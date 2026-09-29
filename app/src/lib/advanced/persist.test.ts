import { MOCK_SCHEMA } from '../backend/mock';
import { settings } from '../settings.svelte';
import { FakeBackend } from '../../test/fake-backend';
import { loadSection, loadSeries, loadWindow, saveSection, saveSeries, saveWindow } from './persist';

let backend: FakeBackend;
let unsubscribe: () => void;

beforeEach(async () => {
  backend = new FakeBackend(MOCK_SCHEMA);
  unsubscribe = await settings.connect(backend);
});
afterEach(() => {
  unsubscribe();
  settings.state = null;
  settings.errors = {};
  vi.restoreAllMocks();
});

const advanced = () => settings.state!.settings.advanced;

test('persist reads and writes through settings', async () => {
  const setItem = vi.spyOn(Storage.prototype, 'setItem');
  const getItem = vi.spyOn(Storage.prototype, 'getItem');

  expect(loadSection()).toBeNull();
  expect(loadWindow()).toBeNull();
  expect(loadSeries('cpu/0')).toBeNull();

  saveSection('gpu/pci-0000:01:00.0');
  saveWindow(1800);
  saveSeries('cpu/0', ['cpu/0/load/total', 'cpu/0/clock/effective']);
  await vi.waitFor(() => expect(advanced().series['cpu/0']).toBeDefined());

  expect(advanced().section).toBe('gpu/pci-0000:01:00.0');
  expect(loadSection()).toBe('gpu/pci-0000:01:00.0');
  expect(loadWindow()).toBe(1800);
  expect(loadSeries('cpu/0')).toEqual(['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(loadSeries('memory/0')).toBeNull();
  expect(setItem).not.toHaveBeenCalled();
  expect(getItem).not.toHaveBeenCalled();
});

test('saves send the patches the core expects', async () => {
  const seen: unknown[] = [];
  const original = backend.updateSettings.bind(backend);
  backend.updateSettings = (patch) => {
    seen.push(patch);
    return original(patch);
  };

  saveSection('cpu/0');
  saveWindow(60);
  saveSeries('cpu/0', ['a']);
  await vi.waitFor(() => expect(seen).toHaveLength(3));

  expect(seen).toEqual([
    { advanced: { section: 'cpu/0' } },
    { advanced: { window: 60 } },
    { advanced: { series: { 'cpu/0': ['a'] } } },
  ]);
});

test('only the four chart windows are read', async () => {
  for (const seconds of [60, 300, 1800, 3600] as const) {
    saveWindow(seconds);
    await vi.waitFor(() => expect(loadWindow()).toBe(seconds));
  }
  // The core validates on write; a value outside the set (a hand-edited file) reads as unset.
  settings.state = {
    ...settings.state!,
    settings: { ...settings.state!.settings, advanced: { ...advanced(), window: 120 } },
  };
  expect(loadWindow()).toBeNull();
});

test('an empty section reads as null', () => {
  settings.state = {
    ...settings.state!,
    settings: { ...settings.state!.settings, advanced: { ...advanced(), section: '' } },
  };
  expect(loadSection()).toBeNull();
});

test('without settings the loaders read as unset and the savers do nothing', () => {
  settings.state = null;
  expect(loadSection()).toBeNull();
  expect(loadWindow()).toBeNull();
  expect(loadSeries('cpu/0')).toBeNull();
  expect(() => saveSection('cpu/0')).not.toThrow();
});

test('a rejected save never breaks the view', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  backend.updateSettings = async () => {
    throw new Error('boom');
  };
  expect(() => saveWindow(60)).not.toThrow();
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(loadWindow()).toBeNull();
});
