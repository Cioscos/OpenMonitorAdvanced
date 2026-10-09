import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { drawerGroups } from '../../lib/advanced/drawer';
import { sensorLabel } from '../../lib/advanced/labels';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { Sensor, Unit } from '../../lib/types';
import SeriesDrawer from './SeriesDrawer.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const TEMP = `${GPU}/temperature/core`;
const HOTSPOT = `${GPU}/temperature/hotspot`;
const gpuSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU);
const byId = (id: string) => MOCK_SCHEMA.sensors.find((s) => s.id === id)!;
const label = (s: Sensor) => sensorLabel(s, t);
const unitTitle = (unit: Unit) => `title:${unit}`;

function renderDrawer(filter = '', chosen = [LOAD, TEMP]) {
  const ontoggle = vi.fn();
  const onsetmany = vi.fn();
  const onclose = vi.fn();
  const groups = drawerGroups(gpuSensors, ['percent', 'celsius'], filter, label);
  render(SeriesDrawer, {
    groups,
    chosen,
    filter,
    valueText: (id: string) => `now:${id === LOAD ? '42%' : id === TEMP ? '61 °C' : '—'}`,
    unitTitle,
    ontoggle,
    onsetmany,
    onclose,
  });
  return { ontoggle, onsetmany, onclose };
}

const group = (unit: Unit) => screen.getByRole('group', { name: unitTitle(unit) });

beforeEach(() => { i18n.locale = 'en'; });
afterEach(cleanup);
afterEach(() => vi.unstubAllGlobals());

test('rows_show_the_current_value', async () => {
  const { ontoggle } = renderDrawer();
  const load = screen.getByLabelText(label(byId(LOAD))) as HTMLInputElement;
  const hotspot = screen.getByLabelText(label(byId(HOTSPOT))) as HTMLInputElement;
  expect(load.checked).toBe(true);
  expect(hotspot.checked).toBe(false);
  // Name and value now on the row, as text.
  expect(load.closest('li')!.textContent).toContain(label(byId(LOAD)));
  expect(load.closest('li')!.querySelector('.value')!.textContent).toBe('now:42%');
  expect(screen.getByLabelText(label(byId(TEMP))).closest('li')!.querySelector('.value')!.textContent).toBe('now:61 °C');
  // Groups read like the legend titles, strips' units first.
  expect(screen.getAllByRole('group').map((g) => g.getAttribute('aria-label')).slice(0, 2)).toEqual([unitTitle('percent'), unitTitle('celsius')]);
  expect(group('celsius').textContent).toContain(unitTitle('celsius'));

  await fireEvent.click(hotspot);
  expect(ontoggle).toHaveBeenCalledWith(HOTSPOT);
});

test('all_ticks_only_filtered_rows', async () => {
  // «hotspot» leaves one temperature row of two.
  const { onsetmany } = renderDrawer('hotspot');
  expect(screen.queryByLabelText(label(byId(TEMP)))).toBeNull();
  await fireEvent.click(within(group('celsius')).getByRole('button', { name: t('advanced.chart.all') }));
  expect(onsetmany).toHaveBeenCalledExactlyOnceWith([HOTSPOT], true);
});

test('none_unticks_the_filtered_rows_of_the_group', async () => {
  const { onsetmany } = renderDrawer('temp');
  await fireEvent.click(within(group('celsius')).getByRole('button', { name: t('advanced.chart.none') }));
  expect(onsetmany).toHaveBeenCalledExactlyOnceWith([TEMP, HOTSPOT], false);
});

test('two_columns_from_520px', () => {
  const callbacks: ResizeObserverCallback[] = [];
  vi.stubGlobal('ResizeObserver', class {
    constructor(cb: ResizeObserverCallback) { callbacks.push(cb); }
    observe() {}
    disconnect() {}
  });
  renderDrawer();
  const columns = () => (document.querySelector('.groups') as HTMLElement).style.gridTemplateColumns;
  const resize = (width: number) => {
    for (const cb of callbacks) cb([{ contentRect: { width } } as ResizeObserverEntry], {} as ResizeObserver);
    flushSync();
  };
  expect(columns()).toBe('minmax(0, 1fr)');
  resize(519);
  expect(columns()).toBe('minmax(0, 1fr)');
  resize(520);
  expect(columns()).toBe('repeat(2, minmax(0, 1fr))');
  resize(400);
  expect(columns()).toBe('minmax(0, 1fr)');
});

test('escape_asks_to_close', async () => {
  const { onclose } = renderDrawer();
  await fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
  expect(onclose).toHaveBeenCalledOnce();
});
