import { MOCK_SCHEMA } from '../backend/mock';
import type { Schema, Sensor } from '../types';
import { resolveSection, sectionForTile, sidebarEntries } from './nav';

const GPU = 'gpu/pci-0000:01:00.0';
const IGPU = 'gpu/pci-0000:11:00.0';
const HDD = 'storage/device-hdd';

const loadSensor = (deviceId: string): Sensor => ({
  id: `${deviceId}/load/x`,
  deviceId,
  kind: 'load',
  unit: 'percent',
  label: { key: 'x' },
  source: 'mock',
  category: 'load',
});

/** MOCK_SCHEMA plus an iGPU, a second disk, a battery listed first and a NIC without sensors. */
const schema: Schema = {
  ...MOCK_SCHEMA,
  devices: [
    { id: 'battery/0', kind: 'battery', name: 'Battery' },
    ...MOCK_SCHEMA.devices,
    { id: IGPU, kind: 'gpu', name: 'AMD Radeon(TM) Graphics', properties: { integrated: 'true' } },
    { id: HDD, kind: 'storage', name: 'Disk 1 (D:)' },
    { id: 'network/wifi', kind: 'network', name: 'Wi-Fi' },
  ],
  sensors: [...MOCK_SCHEMA.sensors, loadSensor(IGPU), loadSensor(HDD), loadSensor('battery/0')],
};

test('entries follow the spec order and include integrated GPUs', () => {
  expect(sidebarEntries(schema).map((e) => e.id)).toEqual([
    'cpu/0',
    GPU,
    IGPU,
    'memory/0',
    'storage/device-mock-ssd',
    HDD,
    'network/mock-eth',
    'battery/0',
  ]);
});

test('entries carry kind, devices, label key and device name', () => {
  const [cpu, gpu, , memory] = sidebarEntries(schema);
  expect(cpu).toEqual({
    id: 'cpu/0',
    kind: 'cpu',
    deviceIds: ['cpu/0'],
    labelKey: 'advanced.section.cpu',
    labelArg: 'Mock Ryzen 7 7800X3D',
  });
  expect(gpu).toEqual({ id: GPU, kind: 'gpu', deviceIds: [GPU], labelKey: 'advanced.section.gpu', labelArg: 'Mock GeForce RTX 4080' });
  expect(memory).toEqual({ id: 'memory/0', kind: 'memory', deviceIds: ['memory/0'], labelKey: 'advanced.section.memory', labelArg: undefined });
});

test('devices without sensors have no entry', () => {
  expect(sidebarEntries(schema).some((e) => e.id === 'network/wifi')).toBe(false);
  expect(sidebarEntries({ revision: 1, devices: MOCK_SCHEMA.devices, sensors: [] })).toEqual([]);
});

test('simple view tiles map to sections', () => {
  expect(sectionForTile('cpu')).toBe('cpu/0');
  expect(sectionForTile('gpu', GPU)).toBe(GPU);
  expect(sectionForTile('memory')).toBe('memory/0');
  expect(sectionForTile('storage', HDD)).toBe(HDD);
  expect(sectionForTile('network', 'network/mock-eth')).toBe('network/mock-eth');
  expect(sectionForTile('gpu')).toBeNull();
  expect(sectionForTile('network')).toBeNull();
});

test('resolveSection keeps an existing section, else falls back to the first entry', () => {
  const entries = sidebarEntries(schema);
  expect(resolveSection(entries, IGPU)).toBe(IGPU);
  expect(resolveSection(entries, 'gpu/pci-gone')).toBe('cpu/0');
  expect(resolveSection(entries, null)).toBe('cpu/0');
  expect(resolveSection([], 'cpu/0')).toBeNull();
});
