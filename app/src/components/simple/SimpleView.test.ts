import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatRate, formatTemperature } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import type { Schema, Sensor } from '../../lib/types';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { sectionForTile } from '../../lib/advanced/nav';
import tileSource from './Tile.svelte?raw';
import viewSource from './SimpleView.svelte?raw';
import SimpleView from './SimpleView.svelte';

beforeEach(() => { i18n.locale = 'en'; });
afterEach(() => { cleanup(); disconnectSettings(); });

function setup(onOpenAdvanced: (section: string | null) => void = () => {}, schema: Schema = MOCK_SCHEMA) {
  const store = new LiveStore();
  store.applySchema(schema);
  const values = mockValues(1);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values });
  render(SimpleView, { store, onOpenAdvanced });
  const valueOf = (id: string) => values[schema.sensors.findIndex((s) => s.id === id)] as number;
  return { valueOf };
}

const NIC = 'network/mock-eth';
const DISK = 'storage/device-mock-ssd';
const GPU = 'gpu/pci-0000:01:00.0';
const text = () => document.body.textContent ?? '';

test('the network tile follows the throughput setting and the disk line stays in bytes', async () => {
  await connectSettings({ general: { throughputUnit: 'bits' } });
  const { valueOf } = setup();
  const down = valueOf(`${NIC}/throughput/down`);
  const read = valueOf(`${DISK}/throughput/read`);
  expect(text()).toContain(`↓ ${formatRate(down, 'bits', 'en')}`);

  await settings.update({ general: { throughputUnit: 'bytes' } });
  await vi.waitFor(() => expect(text()).toContain(`↓ ${formatRate(down, 'bytes', 'en')}`));
  expect(text()).not.toContain('bit/s');
  // Disks read the same in both modes.
  expect(text()).toContain(formatRate(read, 'bytes', 'en'));
  await settings.update({ general: { throughputUnit: 'bits' } });
  await vi.waitFor(() => expect(text()).toContain(`↓ ${formatRate(down, 'bits', 'en')}`));
  expect(text()).toContain(formatRate(read, 'bytes', 'en'));
});

test('the GPU tile shows its temperature in the chosen unit', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  const { valueOf } = setup();
  const celsius = valueOf(`${GPU}/temperature/core`);
  expect(text()).toContain(formatTemperature(celsius, 'en', 'f'));
  expect(text()).toContain('°F');

  await settings.update({ general: { temperatureUnit: 'c' } });
  await vi.waitFor(() => expect(text()).toContain(formatTemperature(celsius, 'en', 'c')));
  expect(text()).not.toContain('°F');
});

test('the health banner lives in the strip under the top bar, not in the page', () => {
  setup();
  expect(screen.queryByRole('status')).toBeNull();
});

const GPU2 = 'gpu/pci-0000:02:00.0';
/** MOCK_SCHEMA plus a second discrete GPU: five tiles. */
const fiveTiles: Schema = {
  ...MOCK_SCHEMA,
  devices: [
    ...MOCK_SCHEMA.devices,
    { id: GPU2, kind: 'gpu', name: 'Second GPU', vendor: 'NVIDIA', properties: { integrated: 'false' } },
  ],
  sensors: [
    ...MOCK_SCHEMA.sensors,
    { id: `${GPU2}/load/core`, deviceId: GPU2, kind: 'load', unit: 'percent', label: { key: 'gpu.load.core' }, source: 'mock', category: 'load' },
  ],
};
const grid = () => document.querySelector<HTMLElement>('.grid')!;
const simple = () => document.querySelector<HTMLElement>('.simple')!;

test('the_whole_tile_opens_the_advanced_page', async () => {
  const opened: Array<string | null> = [];
  setup((section) => opened.push(section));
  const button = screen.getByRole('button', { name: 'GPU' });
  expect(button.classList.contains('open')).toBe(true);
  expect(button.closest('.tile')?.tagName).toBe('DIV');
  await fireEvent.click(button);
  expect(opened).toEqual([sectionForTile('gpu', GPU)]);
});

test('five_tiles_span_two_and_three_columns', () => {
  setup(() => {}, fiveTiles);
  expect(grid().style.getPropertyValue('--cols')).toBe('6');
  const spans = [...document.querySelectorAll<HTMLElement>('.tile')].map((tile) => tile.style.getPropertyValue('--span'));
  expect(spans).toEqual(['2', '2', '2', '3', '3']);
});

test('tiles_keep_natural_height_and_the_block_is_centred', () => {
  setup();
  expect(simple().classList.contains('fill')).toBe(false);
  expect(viewSource).not.toMatch(/simple-top|grid-template-rows|flex: 1/);
  expect(viewSource).toMatch(/\.host\s*\{[^}]*margin-block: auto;[^}]*padding-block: 12px;/);
});

test('sparkline_is_34px_and_80px_in_the_two_row_grid', () => {
  expect(viewSource).toMatch(/\.spark\s*\{[^}]*height: 34px;/);
  expect(viewSource).toMatch(/@container simple \(min-width: 900px\)[\s\S]*\.spark\s*\{\s*height: 80px;/);
});

const chipTexts = () => [...document.querySelectorAll('.chip')].map((c) => c.textContent?.trim() ?? '');

test('missing_sensors_hide_their_chips', () => {
  // MOCK_SCHEMA has no fan, no throttle, no power limit and no DIMM temperature.
  setup();
  const chips = chipTexts();
  expect(chips.length).toBeGreaterThan(0);
  expect(chips.some((c) => /fan/i.test(c) || /dimm/i.test(c) || /limited/i.test(c))).toBe(false);
  expect(chips.some((c) => c.includes('—'))).toBe(false);
  expect(chips.some((c) => c.startsWith('Hotspot'))).toBe(true);
});

const DISK2 = 'storage/device-mock-hdd';
function twoDisks(): { store: LiveStore; schema: Schema } {
  const sensor = (key: string, kind: 'throughput' | 'percent', unit: 'bytes_per_second' | 'percent', arg?: string): Sensor => ({
    id: `${DISK2}/${kind}/${key}`,
    deviceId: DISK2,
    kind,
    unit,
    label: { key, arg },
    source: 'mock',
    category: kind,
  });
  const schema: Schema = {
    ...MOCK_SCHEMA,
    devices: [...MOCK_SCHEMA.devices, { id: DISK2, kind: 'storage', name: 'Disk 1 (D:)' }],
    sensors: [
      ...MOCK_SCHEMA.sensors,
      sensor('storage.read', 'throughput', 'bytes_per_second'),
      sensor('storage.write', 'throughput', 'bytes_per_second'),
      sensor('storage.volumeUsed', 'percent', 'percent', 'D:'),
    ],
  };
  const store = new LiveStore();
  store.applySchema(schema);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values: [...mockValues(1), 4_000_000, 5_000_000, 41] });
  return { store, schema };
}

test('standby_disk_shows_standby_only', () => {
  const { store } = twoDisks();
  store.setDiskStates([{ deviceId: DISK2, power: 'standby' }]);
  render(SimpleView, { store, onOpenAdvanced: () => {} });
  const rows = [...document.querySelectorAll('.disk-row')];
  expect(rows).toHaveLength(2);
  expect(rows[1].textContent).toContain('In standby');
  expect(rows[1].textContent).not.toContain('MB');
  expect(rows[1].textContent).not.toContain('—');
  // The awake disk keeps its values.
  expect(rows[0].textContent).not.toContain('In standby');
  expect(rows[0].textContent).toContain('/s');
});

test('thread_bars_have_their_value_in_a_tooltip', () => {
  setup();
  const bars = [...document.querySelectorAll<HTMLElement>('.threads i')];
  expect(bars).toHaveLength(8);
  expect(bars.every((b) => /^\d+%$/.test(b.title) && b.style.height.endsWith('%'))).toBe(true);
});

test('technical_labels_use_term', () => {
  setup();
  const terms = [...document.querySelectorAll('.term')].map((t) => t.textContent);
  expect(terms).toEqual(expect.arrayContaining(['Hotspot', 'Load per thread', 'VRAM', 'Activity', 'Link']));
  // A Term is focusable, so it never sits inside the tile's button, and only Terms are lifted above its overlay.
  expect(document.querySelectorAll('button .term')).toHaveLength(0);
  expect(tileSource).toMatch(/\.tile :global\(\.term\)\s*\{[^}]*position: relative;[^}]*z-index: 1;/);
});

test('clock_chips_use_their_terms', () => {
  setup();
  const clocks = [...document.querySelectorAll<HTMLElement>('.chip .term')].filter((term) => term.textContent === t('tile.clock'));
  const tips = clocks.map((term) => document.getElementById(term.getAttribute('aria-describedby')!)?.textContent);
  expect(tips).toEqual([t('glossary.clock'), t('glossary.gpuClock')]);
});

test('sparklines_let_clicks_through_and_bars_with_tooltips_sit_above_the_overlay', () => {
  // The sparkline never takes the click meant for the tile; the bars and disk names keep their tooltips.
  expect(viewSource).toMatch(/\.spark\s*\{[^}]*pointer-events: none;/);
  expect(viewSource).toMatch(/\.threads,\s*\.disks \[title\]\s*\{[^}]*position: relative;[^}]*z-index: 1;/);
});

test('bars_expose_their_values_as_meters', () => {
  const { valueOf } = setup();
  const vram = document.querySelector<HTMLElement>('.bar[role="meter"][aria-label="VRAM"]')!;
  expect(vram).toBeTruthy();
  const used = valueOf(`${GPU}/data/memory-dedicated-used`);
  const total = valueOf(`${GPU}/data/memory-dedicated-total`);
  expect(Number(vram.getAttribute('aria-valuenow'))).toBeCloseTo(Math.min(100, (used / total) * 100));
  expect(vram.getAttribute('aria-valuemax')).toBe('100');
  const threads = [...document.querySelectorAll<HTMLElement>('.threads i')];
  expect(threads.every((bar) => bar.getAttribute('role') === 'meter' && bar.getAttribute('aria-valuenow') !== null)).toBe(true);
  expect(threads[0].getAttribute('aria-label')).toBe(t('sensor.cpu.load.thread', { arg: '0' }));
  const activity = [...document.querySelectorAll<HTMLElement>('.disks .act')];
  expect(activity.length).toBeGreaterThan(0);
  expect(activity.every((bar) => bar.getAttribute('role') === 'meter' && bar.getAttribute('aria-valuenow') !== null && bar.getAttribute('aria-label')?.startsWith(t('tile.disk.activity')))).toBe(true);
});

test('fan_rpm_uses_the_locale_separators', () => {
  i18n.locale = 'it';
  const rpm: Sensor = { id: `${GPU}/fan/rpm`, deviceId: GPU, kind: 'fan', unit: 'rpm', label: { key: 'gpu.fan.rpm' }, source: 'mock', category: 'fan' };
  const schema: Schema = { ...MOCK_SCHEMA, sensors: [...MOCK_SCHEMA.sensors, rpm] };
  const store = new LiveStore();
  store.applySchema(schema);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values: [...mockValues(1), 1500] });
  render(SimpleView, { store, onOpenAdvanced: () => {} });
  expect(chipTexts().some((chip) => chip.includes('1.500 RPM'))).toBe(true);
});
