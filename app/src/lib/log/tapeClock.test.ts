import { flushSync } from 'svelte';
import { makeLogStatus } from '../../test/fake-backend';
import type { LogStatus } from '../types';
import { createTapeClock } from './tapeClock.svelte';

let hidden = false;
let revision = 0;

beforeEach(() => {
  hidden = false;
  revision = 0;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  vi.useFakeTimers({ now: 0 });
});

afterEach(() => {
  vi.useRealTimers();
});

const status = (over: Partial<LogStatus>): LogStatus =>
  makeLogStatus({ revision: ++revision, session: 1, state: 'recording', ...over });

/** A clock on the fake `Date.now()`, with the 1 s interval's gap threshold (L3: 5 s). */
function clock(maxGapMs = 5_000) {
  return createTapeClock({ now: () => Date.now(), maxGapMs: () => maxGapMs });
}

/** Moves the fake time to `ms` and returns the counter's whole seconds. */
function secondsAt(tape: ReturnType<typeof clock>, ms: number): number {
  vi.advanceTimersByTime(ms - Date.now());
  flushSync();
  return Math.floor(tape.recordedMs / 1000);
}

test('holds_until_the_core_reports_advancing_time_then_runs_each_second', () => {
  const tape = clock();
  // The start reply and the first tick carry 0 ms: the tick clock has not counted anything yet.
  tape.update(status({ recordedMs: 0 }));
  expect(secondsAt(tape, 900)).toBe(0);
  expect(vi.getTimerCount()).toBe(0);
  tape.update(status({ recordedMs: 0, rows: 1 }));
  expect(secondsAt(tape, 1_000)).toBe(0);
  // The first tick that adds time starts the counter from it.
  tape.update(status({ recordedMs: 998 }));
  expect(vi.getTimerCount()).toBe(1);
  expect(secondsAt(tape, 1_001)).toBe(0);
  expect(secondsAt(tape, 1_003)).toBe(1);
  // No status for two seconds (the core's pacer skipped one): the counter still advances.
  expect(secondsAt(tape, 2_010)).toBe(2);
  expect(secondsAt(tape, 3_010)).toBe(3);
  tape.update(status({ recordedMs: 3_013 }));
  expect(secondsAt(tape, 3_990)).toBe(3);
  expect(secondsAt(tape, 3_998)).toBe(4);
  tape.destroy();
  expect(vi.getTimerCount()).toBe(0);
});

test('a_status_behind_the_shown_value_never_moves_it_back', () => {
  const tape = clock();
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 1_000 }));
  vi.advanceTimersByTime(1_010);
  flushSync();
  expect(tape.recordedMs).toBe(2_000);
  // A late tick: its recorded time is behind what the counter shows.
  tape.update(status({ recordedMs: 1_990 }));
  flushSync();
  expect(tape.recordedMs).toBe(2_000);
  vi.advanceTimersByTime(1_000);
  flushSync();
  expect(Math.floor(tape.recordedMs / 1000)).toBe(2);
  vi.advanceTimersByTime(10);
  flushSync();
  expect(Math.floor(tape.recordedMs / 1000)).toBe(3);
  tape.destroy();
});

test('stops_at_the_gap_threshold_without_a_new_status', () => {
  const tape = clock(5_000);
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 1_000 }));
  // The sampler stalls: the counter never runs further than one gap past the last status.
  expect(secondsAt(tape, 20_000)).toBe(6);
  expect(tape.recordedMs).toBe(6_000);
  expect(vi.getTimerCount()).toBe(0);
  tape.destroy();
});

test('paused_freezes_without_a_timer_and_resume_waits_for_the_core', () => {
  const tape = clock();
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 1_000 }));
  expect(secondsAt(tape, 1_600)).toBe(2);
  // The core recorded 1 s (L3 counts the time between ticks only); what was shown stays.
  tape.update(status({ state: 'paused', recordedMs: 1_000 }));
  expect(vi.getTimerCount()).toBe(0);
  expect(secondsAt(tape, 9_000)).toBe(2);
  expect(tape.recordedMs).toBe(2_600);
  // Resumed: the reply and the first tick after it add nothing, so the counter holds.
  tape.update(status({ recordedMs: 1_000 }));
  expect(secondsAt(tape, 10_000)).toBe(2);
  tape.update(status({ recordedMs: 1_000, rows: 3 }));
  expect(secondsAt(tape, 10_900)).toBe(2);
  tape.update(status({ recordedMs: 1_998 }));
  expect(secondsAt(tape, 10_903)).toBe(2);
  expect(secondsAt(tape, 11_000)).toBe(2);
  expect(secondsAt(tape, 11_900)).toBe(2);
  expect(secondsAt(tape, 11_903)).toBe(3);
  tape.destroy();
});

test('stop_and_error_show_the_final_value', () => {
  const tape = clock();
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 1_000 }));
  vi.advanceTimersByTime(1_200);
  flushSync();
  expect(tape.recordedMs).toBe(2_000);
  tape.update(status({ state: 'idle', recordedMs: 1_000 }));
  flushSync();
  expect(tape.recordedMs).toBe(1_000);
  expect(vi.getTimerCount()).toBe(0);

  tape.update(status({ session: 2, recordedMs: 0 }));
  tape.update(status({ session: 2, recordedMs: 4_000 }));
  vi.advanceTimersByTime(500);
  tape.update(status({ session: 2, state: 'error', recordedMs: 4_000 }));
  flushSync();
  expect(tape.recordedMs).toBe(4_000);
  expect(vi.getTimerCount()).toBe(0);
  tape.destroy();
});

test('a_new_session_starts_from_its_own_value', () => {
  const tape = clock();
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 7_000 }));
  vi.advanceTimersByTime(500);
  tape.update(status({ state: 'idle', recordedMs: 7_000 }));
  tape.update(status({ session: 2, recordedMs: 0 }));
  flushSync();
  expect(tape.recordedMs).toBe(0);
  tape.destroy();
});

test('no_timer_while_hidden_and_the_counter_catches_up_when_shown', () => {
  const tape = clock();
  tape.update(status({ recordedMs: 0 }));
  tape.update(status({ recordedMs: 1_000 }));
  expect(vi.getTimerCount()).toBe(1);
  hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  expect(vi.getTimerCount()).toBe(0);
  vi.advanceTimersByTime(2_500);
  hidden = false;
  document.dispatchEvent(new Event('visibilitychange'));
  flushSync();
  expect(tape.recordedMs).toBe(3_500);
  expect(vi.getTimerCount()).toBe(1);
  tape.destroy();
  document.dispatchEvent(new Event('visibilitychange'));
  expect(vi.getTimerCount()).toBe(0);
});

test('no_status_shows_zero', () => {
  const tape = clock();
  tape.update(null);
  expect(tape.recordedMs).toBe(0);
  tape.update(status({ state: 'paused', recordedMs: 12_000 }));
  expect(tape.recordedMs).toBe(12_000);
  tape.update(null);
  expect(tape.recordedMs).toBe(0);
  tape.destroy();
});
