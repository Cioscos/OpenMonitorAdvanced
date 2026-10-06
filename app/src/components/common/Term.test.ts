import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import TermHost from '../../test/fixtures/TermHost.svelte';
import Term, { placeTip } from './Term.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

function tooltipOf(term: HTMLElement): HTMLElement {
  return document.getElementById(term.getAttribute('aria-describedby')!)!;
}

test('term_is_focusable_and_described', () => {
  render(Term, { term: 'tjmax' });
  const term = screen.getByText(t('glossary.tjmax.name'));
  expect(term.getAttribute('tabindex')).toBe('0');
  const tooltip = tooltipOf(term);
  expect(tooltip.getAttribute('role')).toBe('tooltip');
  expect(tooltip.textContent).toBe(t('glossary.tjmax'));
  // The tooltip lives under <body> and goes away with the term.
  cleanup();
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  // A term without a `.name` key shows itself.
  render(Term, { term: 'mode.nothing' });
  expect(screen.getByText('mode.nothing').getAttribute('tabindex')).toBe('0');
});

test('tooltip_shows_on_hover_and_focus', async () => {
  vi.useFakeTimers();
  try {
    render(Term, { term: 'whea' });
    const term = screen.getByText(t('glossary.whea.name'));
    const tooltip = tooltipOf(term);
    expect(tooltip.hidden).toBe(true);
    await fireEvent.mouseEnter(term);
    expect(tooltip.hidden).toBe(false);
    await fireEvent.mouseLeave(term);
    // A short grace lets the pointer cross to the tooltip.
    expect(tooltip.hidden).toBe(false);
    await vi.advanceTimersByTimeAsync(500);
    expect(tooltip.hidden).toBe(true);
    await fireEvent.focus(term);
    expect(tooltip.hidden).toBe(false);
    await fireEvent.blur(term);
    expect(tooltip.hidden).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

test('escape_hides_the_tooltip', async () => {
  render(Term, { term: 'fft' });
  const term = screen.getByText(t('glossary.fft.name'));
  const tooltip = tooltipOf(term);
  term.focus();
  await fireEvent.focus(term);
  expect(tooltip.hidden).toBe(false);
  // Esc is taken by the tooltip, so the settings screen behind it stays open.
  expect(await fireEvent.keyDown(term, { key: 'Escape' })).toBe(false);
  expect(tooltip.hidden).toBe(true);
  // Closed, Esc goes on to the page.
  expect(await fireEvent.keyDown(term, { key: 'Escape' })).toBe(true);
});

test('the tooltip flips above and stays inside the window', () => {
  const viewport = { width: 400, height: 300 };
  const size = { width: 200, height: 80 };
  // Room below: under the term, centred.
  expect(placeTip({ left: 150, top: 50, width: 100, height: 20 }, size, viewport)).toEqual({ left: 100, top: 76 });
  // At the bottom edge: above the term.
  expect(placeTip({ left: 150, top: 260, width: 100, height: 20 }, size, viewport)).toEqual({ left: 100, top: 174 });
  // At the right edge: pushed back inside.
  expect(placeTip({ left: 360, top: 50, width: 40, height: 20 }, size, viewport).left).toBe(192);
  // At the left edge: pushed back inside.
  expect(placeTip({ left: 0, top: 50, width: 10, height: 20 }, size, viewport).left).toBe(8);
});

test('a term as the whole content of a block leaves its siblings alone', async () => {
  const { rerender } = render(TermHost, { show: true, items: ['fft', 'ntt', 'fma'] });
  const terms = () => [...document.querySelectorAll('#each-host .term')].map((el) => el.textContent);
  expect(terms()).toEqual(['FFT', 'NTT', 'FMA']);
  expect(document.querySelectorAll('[role="tooltip"]')).toHaveLength(4);

  await rerender({ show: false, items: ['fft', 'ntt', 'fma'] });
  expect(document.getElementById('if-host')?.textContent).toBe('after tail');
  expect(document.querySelectorAll('[role="tooltip"]')).toHaveLength(3);

  // A keyed reorder, then a removal.
  await rerender({ show: false, items: ['fma', 'fft', 'ntt'] });
  expect(terms()).toEqual(['FMA', 'FFT', 'NTT']);
  expect(document.getElementById('after-each')).toBeTruthy();
  await rerender({ show: false, items: ['fma', 'ntt'] });
  expect(terms()).toEqual(['FMA', 'NTT']);
  expect(document.getElementById('each-host')?.lastElementChild?.id).toBe('after-each');
  expect(document.querySelectorAll('[role="tooltip"]')).toHaveLength(2);

  await rerender({ show: true, items: [] });
  expect(document.getElementById('if-host')?.textContent).toBe('FFTafter tail');
  expect(document.getElementById('each-host')?.textContent).toBe('after');
  expect(document.querySelectorAll('[role="tooltip"]')).toHaveLength(1);
});
