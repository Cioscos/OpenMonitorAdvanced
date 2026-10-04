import { cleanup, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import type { KpiDef } from '../../lib/advanced/pages';
import { SERVICE_MOCK_SCHEMA } from '../../lib/backend/mock';
import { DASH } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import KpiRow from './KpiRow.svelte';

const TEMP = 'storage/device-mock-hdd/temperature/drive';
const schema = SERVICE_MOCK_SCHEMA;
const index = schema.sensors.findIndex((s) => s.id === TEMP);

const TEMPERATURE: KpiDef = {
  id: 'temperature',
  labelKey: 'advanced.kpi.temperature',
  unit: 'celsius',
  value: (valueOf) => valueOf(TEMP),
  sensorId: TEMP,
};
// A historical statistic, not a measure: it has no sensor of its own.
const PEAK: KpiDef = {
  id: 'peakLoad',
  labelKey: 'advanced.kpi.peakLoad',
  unit: 'celsius',
  value: () => 61,
};

let seq = 0;
function feed(store: LiveStore, value: number | null, quality: 0 | 1 | 2) {
  const values = schema.sensors.map(() => null as number | null);
  values[index] = value;
  const codes = schema.sensors.map(() => 0);
  codes[index] = quality;
  store.applySnapshot({ revision: schema.revision, seq: ++seq, timestampMs: 1000 + seq, values, quality: codes });
  flushSync();
}

function setup(kpis: KpiDef[] = [TEMPERATURE, PEAK]) {
  const store = new LiveStore();
  store.applySchema(schema);
  const view = render(KpiRow, {
    kpis,
    valueOf: (id: string) => store.value(id),
    qualityOf: (id: string) => store.quality(id),
    statsOf: () => null,
  });
  return { store, view };
}

const tiles = () => [...document.querySelectorAll('.kpi')] as HTMLElement[];
const last = (tile: HTMLElement) => tile.querySelector('.last');
const muted = (tile: HTMLElement) => tile.querySelector('.value')!.classList.contains('stale');

beforeEach(() => {
  seq = 0;
  i18n.locale = 'en';
});
afterEach(cleanup);

test('a suspended temperature is muted and labelled as the last reading', () => {
  const { store } = setup();
  feed(store, 41, 2);
  const [temperature, peak] = tiles();
  expect(muted(temperature)).toBe(true);
  expect(last(temperature)?.textContent).toBe(t('value.lastReading'));
  expect(temperature.querySelector('.value')!.textContent).toContain('41');
  // The peak is a statistic, not a measure: it never reads as a last reading.
  expect(muted(peak)).toBe(false);
  expect(last(peak)).toBeNull();
});

test('a held temperature looks like a fresh one', () => {
  const { store } = setup();
  feed(store, 41, 1);
  const [temperature] = tiles();
  expect(muted(temperature)).toBe(false);
  expect(last(temperature)).toBeNull();
});

test('a suspended temperature without a value shows the dash and no last reading', () => {
  const { store } = setup();
  feed(store, null, 2);
  const [temperature] = tiles();
  expect(temperature.querySelector('.value')!.textContent).toBe(DASH);
  expect(muted(temperature)).toBe(false);
  expect(last(temperature)).toBeNull();
});

test('the temperature follows Fresh, Held, Suspended and Fresh again', () => {
  const { store } = setup();
  const seen: boolean[] = [];
  for (const quality of [0, 1, 2, 0] as const) {
    feed(store, 41, quality);
    const [temperature] = tiles();
    seen.push(muted(temperature));
    expect(last(temperature) !== null).toBe(quality === 2);
  }
  expect(seen).toEqual([false, false, true, false]);
});

test('the last reading joins the secondary text of the tile', () => {
  const { store } = setup([{ ...TEMPERATURE, secondary: () => 'of 1 TB' }]);
  feed(store, 41, 2);
  const [tile] = tiles();
  expect(tile.textContent).toContain('of 1 TB');
  expect(last(tile)?.textContent).toBe(t('value.lastReading'));
});
