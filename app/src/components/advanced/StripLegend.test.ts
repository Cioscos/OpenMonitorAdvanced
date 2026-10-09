import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import StripLegend from './StripLegend.svelte';

type Row = {
  id: string;
  label: string;
  color: string;
  dash: readonly number[];
  now: string;
  min: string;
  max: string;
  hidden: boolean;
};

const rows: Row[] = [
  { id: 'a', label: 'GPU load', color: '#ff4fd8', dash: [], now: '42%', min: '10%', max: '90%', hidden: false },
  { id: 'b', label: 'Thread 9', color: '#ff4fd8', dash: [6, 4], now: '—', min: '—', max: '—', hidden: true },
];

beforeEach(() => { i18n.locale = 'en'; });
afterEach(cleanup);

function renderLegend(ontoggle = vi.fn(), onremove = vi.fn()) {
  render(StripLegend, { title: t('advanced.chart.unit.percent'), rows, ontoggle, onremove });
  return { ontoggle, onremove };
}

test('name_button_toggles_and_reports_pressed', async () => {
  const { ontoggle } = renderLegend();
  expect(screen.getByText(t('advanced.chart.unit.percent'))).toBeTruthy();
  const shown = screen.getByRole('button', { name: 'GPU load' });
  const hidden = screen.getByRole('button', { name: 'Thread 9' });
  // Pressed means the line is visible.
  expect(shown.getAttribute('aria-pressed')).toBe('true');
  expect(hidden.getAttribute('aria-pressed')).toBe('false');
  await fireEvent.click(hidden);
  expect(ontoggle).toHaveBeenCalledWith('b');
  // A hidden row stays, dimmed, with its values.
  const row = hidden.closest<HTMLElement>('.row')!;
  expect(row.classList.contains('off')).toBe(true);
  const first = shown.closest<HTMLElement>('.row')!;
  expect(first.querySelector('.now')!.textContent).toBe('42%');
  expect(first.querySelector('.min')!.textContent).toBe('10%');
  expect(first.querySelector('.max')!.textContent).toBe('90%');
});

test('remove_has_an_accessible_name', async () => {
  const { onremove } = renderLegend();
  const remove = screen.getByRole('button', { name: t('advanced.chart.remove', { name: 'GPU load' }) });
  expect(remove.textContent?.trim()).toBe('×');
  await fireEvent.click(remove);
  expect(onremove).toHaveBeenCalledWith('a');
  expect(t('advanced.chart.remove', { name: 'GPU load' })).toBe('Remove GPU load');
  i18n.locale = 'it';
  expect(t('advanced.chart.remove', { name: 'GPU load' })).toBe('Togli GPU load');
});

test('dashed_swatch_matches_the_series', () => {
  renderLegend();
  const swatch = (name: string) => screen.getByRole('button', { name }).closest('.row')!.querySelector('.swatch line')!;
  expect(swatch('GPU load').getAttribute('stroke')).toBe('#ff4fd8');
  expect(swatch('GPU load').getAttribute('stroke-dasharray')).toBeNull();
  expect(swatch('Thread 9').getAttribute('stroke-dasharray')).toBe('6 4');
});
