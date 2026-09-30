import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { canDo, log } from '../../lib/log.svelte';
import type { LogState, LogStatus } from '../../lib/types';
import { FakeBackend, makeLogStatus } from '../../test/fake-backend';
import Recorder from './Recorder.svelte';

const PATH = 'C:\\Users\\test\\Documents\\OpenMonitor Advanced\\logs\\oma-2026-09-30_14-03-12-part2.csv';

let off: (() => void) | undefined;
let hidden = false;

beforeEach(() => {
  i18n.locale = 'en';
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
});

afterEach(() => {
  cleanup();
  off?.();
  off = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/** Connects the app-wide store to a fake backend in `over`'s state and renders the recorder. */
async function setup(over: Partial<LogStatus> = {}, openFolder: () => Promise<void> = async () => {}) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = makeLogStatus({ revision: 1, session: 1, part: 2, path: PATH, ...over });
  off = await log.connect(backend);
  const view = render(Recorder, { openFolder });
  // With the deck closed the recorder is the only button.
  const button = screen.getByRole('button') as HTMLButtonElement;
  return { backend, button, ...view };
}

const deck = () => document.getElementById('log-deck');
const dot = (container: HTMLElement) => container.querySelector('.rec-dot') as HTMLElement | null;

async function openDeck(button: HTMLButtonElement) {
  await fireEvent.click(button);
  expect(deck()).not.toBeNull();
}

const key = (name: string) => screen.getByRole('button', { name: t(name) });

test('idle_shows_the_cassette_and_opens_the_deck_with_the_keyboard', async () => {
  const { button, container } = await setup({ state: 'idle' });
  expect(button.getAttribute('aria-label')).toBe(t('log.recorder.idle'));
  expect(container.querySelector('.cassette-icon')).not.toBeNull();
  expect(dot(container)).toBeNull();
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(deck()).toBeNull();

  // A native <button>: Enter and Space activate it by the platform, which jsdom models as a click.
  expect(button.tagName).toBe('BUTTON');
  expect(button.type).toBe('button');
  button.focus();
  await fireEvent.click(button);
  expect(button.getAttribute('aria-expanded')).toBe('true');
  expect(button.getAttribute('aria-controls')).toBe('log-deck');
  expect(deck()).not.toBeNull();
  // The cassette label names the file.
  expect(screen.getByText('oma-2026-09-30_14-03-12-part2.csv')).toBeTruthy();

  // Esc closes the deck and marks the event handled, so App's handler leaves the settings open.
  let settingsSawEscape = false;
  const settings = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && !event.defaultPrevented) settingsSawEscape = true;
  };
  window.addEventListener('keydown', settings);
  key('log.deck.rec').focus();
  await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  window.removeEventListener('keydown', settings);
  expect(settingsSawEscape).toBe(false);
  expect(deck()).toBeNull();
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(button);
});

test('recording_blinks_at_one_hertz', async () => {
  vi.useFakeTimers();
  const { button, container } = await setup({ state: 'recording', recordedMs: 767_000 });
  expect(button.getAttribute('aria-label')).toBe(t('log.recorder.recording', { time: '00:12:47' }));
  expect(button.textContent).toContain('00:12:47');
  expect(dot(container)!.classList.contains('on')).toBe(true);
  vi.advanceTimersByTime(500);
  flushSync();
  expect(dot(container)!.classList.contains('on')).toBe(false);
  vi.advanceTimersByTime(500);
  flushSync();
  expect(dot(container)!.classList.contains('on')).toBe(true);

  // The reels spin while recording with the deck open.
  await openDeck(button);
  expect(container.querySelectorAll('.reel.spin')).toHaveLength(2);
});

test('counter_advances_every_second_between_paced_statuses', async () => {
  vi.useFakeTimers();
  const { backend, button } = await setup({ state: 'recording', recordedMs: 0 });
  const counter = () => button.querySelector('.counter')!.textContent;
  // The statuses of the recording of 20:26:21 (M5c) as the core's pacer emitted them: ticks
  // 0.986–1.012 s apart, a status at most once a second, so the ticks at +3.01 s, +5.01 s and
  // +7.01 s were not emitted. The first tick adds nothing to the recorded time.
  const emitted = [0, 1_007, 2_012, 4_011, 6_009, 8_015];
  let revision = 1;
  let now = 0;
  const seen: string[] = [];
  for (let second = 0; second < 9; second++) {
    for (const at of emitted.filter((ms) => ms >= now && ms < second * 1000 + 500)) {
      vi.advanceTimersByTime(at - now);
      now = at;
      backend.emitLogStatus(makeLogStatus({ revision: ++revision, session: 1, part: 2, path: PATH, state: 'recording', recordedMs: at }));
    }
    vi.advanceTimersByTime(second * 1000 + 500 - now);
    now = second * 1000 + 500;
    flushSync();
    seen.push(counter()!);
  }
  expect(seen).toEqual(['00:00:00', '00:00:01', '00:00:02', '00:00:03', '00:00:04', '00:00:05', '00:00:06', '00:00:07', '00:00:08']);
});

test('blink_stops_when_hidden', async () => {
  vi.useFakeTimers();
  const { container } = await setup({ state: 'recording' });
  hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  expect(vi.getTimerCount()).toBe(0);
  vi.advanceTimersByTime(1500);
  flushSync();
  expect(dot(container)!.classList.contains('on')).toBe(true);

  // Shown again, it blinks from the lit half.
  hidden = false;
  document.dispatchEvent(new Event('visibilitychange'));
  flushSync();
  expect(vi.getTimerCount()).toBe(1);
  vi.advanceTimersByTime(500);
  flushSync();
  expect(dot(container)!.classList.contains('on')).toBe(false);
});

test('reduced_motion_keeps_the_dot_on', async () => {
  vi.useFakeTimers();
  const real = window.matchMedia;
  vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => {
    const list = real(query);
    return query.includes('prefers-reduced-motion') ? ({ ...list, matches: true } as MediaQueryList) : list;
  });
  const { container } = await setup({ state: 'recording' });
  expect(vi.getTimerCount()).toBe(0);
  for (let i = 0; i < 4; i++) {
    vi.advanceTimersByTime(500);
    flushSync();
    expect(dot(container)!.classList.contains('on')).toBe(true);
  }
});

test('paused_shows_the_fixed_pause_symbol_and_rec_resumes', async () => {
  vi.useFakeTimers();
  const { backend, button, container } = await setup({ state: 'paused', recordedMs: 5_000 });
  expect(button.getAttribute('aria-label')).toBe(t('log.recorder.paused', { time: '00:00:05' }));
  expect(container.querySelector('.pause-icon')).not.toBeNull();
  expect(dot(container)).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
  vi.useRealTimers();

  await openDeck(button);
  expect(container.querySelectorAll('.reel.spin')).toHaveLength(0);
  expect(key('log.deck.pause').getAttribute('aria-pressed')).toBe('true');
  backend.logStatus = makeLogStatus({ revision: 2, session: 1, state: 'recording', path: PATH });
  await fireEvent.click(key('log.deck.resume'));
  expect(backend.logCalls).toContain('logResume');
  await vi.waitFor(() => expect(log.status?.state).toBe('recording'));
});

test('error_shows_the_reason_and_rec_restarts', async () => {
  const { backend, button, container } = await setup({
    state: 'error',
    error: { key: 'log.error.other', detail: 'The device is not ready.' },
  });
  const reason = t('log.error.other', { detail: 'The device is not ready.' });
  expect(button.getAttribute('aria-label')).toBe(t('log.recorder.error', { reason }));
  expect(container.querySelector('.error-icon')).not.toBeNull();
  expect(dot(container)).toBeNull();

  await openDeck(button);
  expect(screen.getByText(reason)).toBeTruthy();
  backend.logStatus = makeLogStatus({ revision: 2, session: 2, state: 'recording', path: PATH });
  await fireEvent.click(key('log.deck.rec'));
  expect(backend.logCalls).toContain('logStart');
  await vi.waitFor(() => expect(log.status?.state).toBe('recording'));
});

test('buttons_follow_can_do', async () => {
  const states: LogState[] = ['idle', 'recording', 'paused', 'error'];
  for (const state of states) {
    const { button } = await setup({ state });
    await openDeck(button);
    const allowed = canDo(state);
    const rec = key(state === 'paused' ? 'log.deck.resume' : 'log.deck.rec');
    expect(rec.getAttribute('aria-disabled'), `${state} rec`).toBe(String(!allowed.rec));
    expect(key('log.deck.pause').getAttribute('aria-disabled'), `${state} pause`).toBe(String(!allowed.pause));
    expect(key('log.deck.stop').getAttribute('aria-disabled'), `${state} stop`).toBe(String(!allowed.stop));
    expect(rec.getAttribute('aria-pressed'), `${state} rec pressed`).toBe(String(state === 'recording'));
    expect(key('log.deck.pause').getAttribute('aria-pressed'), `${state} pause pressed`).toBe(String(state === 'paused'));
    cleanup();
    off?.();
    off = undefined;
  }

  // A disabled key does nothing when pressed.
  const { backend, button } = await setup({ state: 'idle' });
  await openDeck(button);
  await fireEvent.click(key('log.deck.stop'));
  await fireEvent.click(key('log.deck.pause'));
  expect(backend.logCalls).toEqual(['onLogStatus', 'getLogStatus']);
});

test('keys_are_disabled_while_a_command_is_in_flight', async () => {
  const { backend, button } = await setup({ state: 'recording' });
  let release!: (s: LogStatus) => void;
  vi.spyOn(backend, 'logStop').mockReturnValue(new Promise((resolve) => (release = resolve)));
  await openDeck(button);
  await fireEvent.click(key('log.deck.stop'));
  expect(key('log.deck.pause').getAttribute('aria-disabled')).toBe('true');
  expect(key('log.deck.stop').getAttribute('aria-disabled')).toBe('true');
  release(makeLogStatus({ revision: 2, session: 1, state: 'idle', path: PATH }));
  await vi.waitFor(() => expect(key('log.deck.rec').getAttribute('aria-disabled')).toBe('false'));
});

test('open_folder_calls_the_backend', async () => {
  const openFolder = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
  const { button } = await setup({ state: 'idle' }, openFolder);
  await openDeck(button);
  await fireEvent.click(key('log.deck.openFolder'));
  expect(openFolder).toHaveBeenCalledTimes(1);

  // A known key is translated; any other text is the system's message, shown as it is.
  openFolder.mockRejectedValueOnce('log.error.folderMissing');
  await fireEvent.click(key('log.deck.openFolder'));
  expect(await screen.findByText(t('log.error.folderMissing'))).toBeTruthy();
  openFolder.mockRejectedValueOnce('The network path was not found.');
  await fireEvent.click(key('log.deck.openFolder'));
  expect(await screen.findByText('The network path was not found.')).toBeTruthy();
  expect(screen.queryByText(t('log.error.folderMissing'))).toBeNull();
});

test('click_outside_closes_the_deck', async () => {
  const { button } = await setup({ state: 'recording' });
  await openDeck(button);
  // A click inside the deck keeps it open.
  await fireEvent.pointerDown(deck()!);
  expect(deck()).not.toBeNull();
  await fireEvent.pointerDown(document.body);
  expect(deck()).toBeNull();
  expect(button.getAttribute('aria-expanded')).toBe('false');
  // The button toggles it too.
  await fireEvent.click(button);
  await fireEvent.click(button);
  expect(deck()).toBeNull();
});
