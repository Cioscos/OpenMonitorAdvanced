import { cleanup, render, screen } from '@testing-library/svelte';
import { MOCK_MEMORY_PROPERTIES } from '../../lib/backend/mock';
import { i18n } from '../../lib/i18n/index.svelte';
import type { Device } from '../../lib/types';
import MemoryTimings from './MemoryTimings.svelte';

const device = (properties?: Record<string, string>): Device[] => [{ id: 'memory/0', kind: 'memory', name: 'RAM', properties }];

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

test('shows_in_use_timings_and_modules', () => {
  const { container } = render(MemoryTimings, { devices: device(MOCK_MEMORY_PROPERTIES) });
  const text = container.textContent ?? '';
  expect(text).toContain('36-48-48-104');
  expect(text).toContain('6,000 MT/s');
  expect(text).toContain('943 (170.7 ns)');
  expect(text).toContain('1T');
  expect(text).toContain('Mixed');
  expect(text).toContain('DIMM #1');
  expect(text).toContain('16 GB');
  expect(text).toContain('4,808 MT/s');
  expect(text).toContain('39-39-39-77');
  expect(text).toContain('55 / 85 °C');
  // every acronym is a tooltip term
  expect(container.querySelectorAll('.term').length).toBeGreaterThanOrEqual(12);
});

test('per_channel_columns_when_channels_differ', () => {
  const channels = [
    { channel: 'A', mclkMhz: 3000, dataRate: 6000, cl: 36 },
    { channel: 'C', mclkMhz: 3000, dataRate: 6000, cl: 38 },
  ];
  render(MemoryTimings, { devices: device({ memoryTimings: JSON.stringify({ generation: 'ddr5', channels }) }) });
  expect(screen.getByText('Channel A')).toBeTruthy();
  expect(screen.getByText('Channel C')).toBeTruthy();
});

test('spd_note_explains_jedec', () => {
  render(MemoryTimings, { devices: device(MOCK_MEMORY_PROPERTIES) });
  expect(screen.getByText(/EXPO or XMP on/)).toBeTruthy();
});

test('no_section_without_properties', () => {
  const { container } = render(MemoryTimings, { devices: device({}) });
  expect(container.querySelector('section')).toBeNull();
});

test('slot_names_render_as_text', () => {
  const memoryModules = JSON.stringify([{ slot: '<b>EVIL</b>', capacityBytes: 1073741824 }]);
  const { container } = render(MemoryTimings, { devices: device({ memoryModules }) });
  expect(container.querySelector('b')).toBeNull();
  expect(container.textContent).toContain('<b>EVIL</b>');
});
