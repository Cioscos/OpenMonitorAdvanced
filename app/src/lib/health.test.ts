import { MOCK_SCHEMA } from './backend/mock';
import { FakeBackend } from '../test/fake-backend';
import { bannerText, health } from './health.svelte';
import { translate } from './i18n/index.svelte';
import type { Alert, HealthReport } from './types';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const tIt = (key: string, params?: Record<string, string | number>) => translate('it', key, params);

const GPU = 'gpu/pci-0000:01:00.0';

function alert(over: Partial<Alert> = {}): Alert {
  return {
    ruleId: 'gpu-temp',
    sensorId: `${GPU}/temperature/core`,
    deviceId: GPU,
    unit: 'celsius',
    sensorLabel: { key: 'gpu.temperature.core' },
    level: 'crit',
    value: 92,
    threshold: 90,
    sinceMs: 1000,
    valid: true,
    lastValidMs: 2000,
    messageKey: 'rule.gpu-temp.message',
    params: { device: 'Mock GeForce RTX 4080' },
    ...over,
  };
}

function report(over: Partial<HealthReport> = {}): HealthReport {
  return { level: 'ok', sinceMs: 0, revision: 1, coverage: 'complete', unavailableTargets: [], alerts: [], ...over };
}

interface Opts {
  t?: typeof tEn;
  locale?: string;
  temperature?: 'c' | 'f';
  throughput?: 'bits' | 'bytes';
}

const text = (r: HealthReport, o: Opts = {}) =>
  bannerText(r, MOCK_SCHEMA, o.t ?? tEn, o.locale ?? 'en', o.temperature ?? 'c', o.throughput ?? 'bits');

let off: (() => void) | undefined;
afterEach(() => {
  off?.();
  off = undefined;
});

test('stale_revisions_are_ignored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.health = report({ revision: 5 });
  off = await health.connect(backend);
  expect(health.report?.revision).toBe(5);

  backend.emitHealth(report({ revision: 3, level: 'crit' }));
  expect(health.report?.revision).toBe(5);
  backend.emitHealth(report({ revision: 5, level: 'crit' }));
  expect(health.report?.level).toBe('ok');
  backend.emitHealth(report({ revision: 6, level: 'warn' }));
  expect(health.report?.level).toBe('warn');
});

test('subscribes_before_the_getter', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await health.connect(backend);
  const firstGetter = backend.healthCalls.findIndex((c) => c.startsWith('get'));
  expect(backend.healthCalls.slice(0, firstGetter)).toEqual(['onHealth', 'onHealthClock']);
  expect(backend.healthCalls).toContain('getHealthClock');
});

test('disconnect_removes_listeners', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await health.connect(backend);
  expect(backend.healthListenerCount).toBe(2);
  off();
  off = undefined;
  expect(backend.healthListenerCount).toBe(0);
  backend.emitHealth(report({ revision: 9 }));
  expect(health.report).toBeNull();
});

test('failed_getter_cleans_up', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.healthError = 'boom';
  await expect(health.connect(backend)).rejects.toThrow('boom');
  expect(backend.healthListenerCount).toBe(0);
});

test('clock_for_future_revision_waits_for_report', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.health = report({ revision: 3 });
  off = await health.connect(backend);

  backend.emitHealthClock({ revision: 4, levelElapsedMs: 7000 });
  expect(health.elapsedMs).toBeNull();
  backend.emitHealth(report({ revision: 4 }));
  expect(health.elapsedMs).toBe(7000);
  // A clock of an older revision never applies to the newer report.
  backend.emitHealthClock({ revision: 3, levelElapsedMs: 99_000 });
  expect(health.elapsedMs).toBe(7000);
});

test('duration_does_not_use_wall_clock', async () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  try {
    vi.setSystemTime(10_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    backend.health = report({ revision: 2, sinceMs: 1 });
    backend.healthClock = { revision: 2, levelElapsedMs: 90_000 };
    off = await health.connect(backend);
    expect(health.elapsedMs).toBe(90_000);
    // The system clock jumps (time change, resume): the duration stays the core's.
    vi.setSystemTime(99_000_000);
    expect(health.elapsedMs).toBe(90_000);
  } finally {
    vi.useRealTimers();
  }
});

test('banner_text_for_one_alert', () => {
  expect(text(report({ level: 'crit', alerts: [alert()] }))).toEqual({
    title: 'Mock GeForce RTX 4080 overheating (92 °C)',
    items: [],
  });
});

test('banner_text_for_many_alerts', () => {
  const ram = alert({
    ruleId: 'ram-used',
    sensorId: 'memory/0/load/used',
    deviceId: 'memory/0',
    unit: 'percent',
    level: 'warn',
    value: 91,
    messageKey: 'rule.ram-used.message',
    params: {},
  });
  const out = text(report({ level: 'crit', alerts: [alert(), ram] }));
  expect(out.title).toBe('2 problems');
  // Same order as `alerts`.
  expect(out.items).toEqual(['Mock GeForce RTX 4080 overheating (92 °C)', 'Memory almost full (91%)']);
});

test('banner_text_for_partial_coverage', () => {
  expect(text(report({ coverage: 'partial' })).title).toBe(tEn('health.partial'));
  expect(text(report()).title).toBe(tEn('health.allClear'));
});

test('banner_text_for_neutral', () => {
  expect(text(report({ level: 'neutral' }))).toEqual({ title: 'Monitoring active', items: [] });
});

test('invalid_alert_shows_unavailable_value', () => {
  const out = text(report({ level: 'crit', alerts: [alert({ valid: false, value: 92 })] }));
  expect(out.title).toBe('Mock GeForce RTX 4080 overheating (data unavailable)');
  const never = text(report({ level: 'crit', alerts: [alert({ valid: true, value: null })] }));
  expect(never.title).toContain('data unavailable');
});

test('retained_alert_formats_after_sensor_disappears', () => {
  // The sensor is in no schema: unit and label come from the alert itself.
  const gone = alert({
    ruleId: 'custom-0b0c7c6e-1f0a-4a7b-9d55-000000000001',
    sensorId: 'motherboard/gone/fan/fan-1',
    deviceId: 'motherboard/gone',
    unit: 'rpm',
    sensorLabel: { key: 'lhm.raw', arg: 'Fan #1' },
    level: 'warn',
    value: 300,
    threshold: 500,
    messageKey: 'rule.custom.below',
    params: { device: 'Board' },
  });
  const out = text(report({ level: 'warn', alerts: [gone] }));
  expect(out.title).toBe('Fan #1 below 500 RPM (300 RPM)');
});

test('unit_and_language_changes_reformat_without_health_event', () => {
  const r = report({ level: 'crit', alerts: [alert()] });
  expect(text(r, { temperature: 'c' }).title).toContain('92 °C');
  expect(text(r, { temperature: 'f' }).title).toContain('198 °F');
  expect(text(r, { t: tIt, locale: 'it' }).title).toBe('Mock GeForce RTX 4080 surriscaldata (92 °C)');

  const net = alert({
    ruleId: 'custom-0b0c7c6e-1f0a-4a7b-9d55-000000000002',
    unit: 'bytes_per_second',
    sensorId: 'network/mock-eth/throughput/down',
    deviceId: 'network/mock-eth',
    sensorLabel: { key: 'network.down' },
    messageKey: 'rule.custom.above',
    level: 'warn',
    value: 125_000_000,
    threshold: 100_000_000,
  });
  const nr = report({ level: 'warn', alerts: [net] });
  expect(text(nr, { throughput: 'bits' }).title).toContain('1.0 Gbit/s');
  expect(text(nr, { throughput: 'bytes' }).title).toContain('MB/s');
});

test('throughput_follows_the_setting_on_network_devices_only', () => {
  const net = alert({
    ruleId: 'custom-0b0c7c6e-1f0a-4a7b-9d55-000000000003',
    unit: 'bytes_per_second',
    sensorId: 'network/mock-eth/throughput/down',
    deviceId: 'network/mock-eth',
    sensorLabel: { key: 'network.down' },
    messageKey: 'rule.custom.above',
    level: 'warn',
    value: 12_500_000,
    threshold: 10_000_000,
    params: { device: 'Ethernet' },
  });
  const one = (a: Alert, throughput: 'bits' | 'bytes') => text(report({ level: 'warn', alerts: [a] }), { throughput }).title;
  expect(one(net, 'bits')).toBe('Download above 80 Mbit/s (100 Mbit/s)');
  expect(one(net, 'bytes')).toBe('Download above 9.5 MB/s (11.9 MB/s)');
  // Disks show bytes, like their page in the Advanced view and the tray.
  const disk = { ...net, sensorId: 'storage/device-mock-ssd/throughput/read', deviceId: 'storage/device-mock-ssd', sensorLabel: { key: 'storage.read' } };
  expect(one(disk, 'bits')).toBe(`${tEn('sensor.storage.read')} above 9.5 MB/s (11.9 MB/s)`);
  // A network device gone from the schema is still recognized by its id.
  const gone = { ...net, sensorId: 'network/gone/throughput/down', deviceId: 'network/gone' };
  expect(one(gone, 'bits')).toBe('Download above 80 Mbit/s (100 Mbit/s)');
});

test('volume_used_message_names_the_volume', () => {
  const volume = alert({
    ruleId: 'volume-used',
    sensorId: 'storage/device-mock-ssd/percent/volume-mock-guid',
    deviceId: 'storage/device-mock-ssd',
    unit: 'percent',
    sensorLabel: { key: 'storage.volumeUsed', arg: 'C:' },
    level: 'warn',
    value: 95,
    threshold: 90,
    messageKey: 'rule.volume-used.message',
    params: { device: 'Disk 0 (C:)' },
  });
  const r = report({ level: 'warn', alerts: [volume] });
  expect(text(r).title).toBe('Volume C: used almost full (95%)');
  expect(text(r, { t: tIt, locale: 'it' }).title).toBe('Volume C: occupato quasi pieno (95%)');
});
