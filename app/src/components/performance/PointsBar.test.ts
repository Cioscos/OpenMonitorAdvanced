import { cleanup, render, screen } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import PointsBar from './PointsBar.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

const bar = () => screen.getByRole('meter', { name: t('performance.score.disk.pointsBar') });

test('full_scale_is_the_next_500_above_the_marks', () => {
  const view = render(PointsBar, { value: 1012, reference: 995, referenceLabel: 'x' });
  expect(bar().getAttribute('aria-valuemax')).toBe('1500');
  view.unmount();
  render(PointsBar, { value: 300, reference: null, referenceLabel: null });
  expect(bar().getAttribute('aria-valuemax')).toBe('1000');
});

test('mark_has_its_label_for_screen_readers', () => {
  render(PointsBar, { value: 1012, reference: 995, referenceLabel: 'Fanxiang S880 2TB · Author' });
  expect(screen.getByRole('img', { name: 'Fanxiang S880 2TB · Author: 995' })).toBeTruthy();
});

test('shows_label_big_value_and_the_mark_caption', () => {
  render(PointsBar, { value: 1012, reference: 995, referenceLabel: 'Fanxiang S880 2TB · Author' });
  expect(screen.getByText('Points')).toBeTruthy();
  expect(screen.getByText('1,012')).toBeTruthy();
  expect(screen.getByText('995', { selector: '.num' }).parentElement?.textContent).toMatch(/▲\s*995 points · Fanxiang S880 2TB · Author/);
});

test('dash_without_points', () => {
  render(PointsBar, { value: null, reference: null, referenceLabel: null });
  expect(screen.getByText('–')).toBeTruthy();
});
