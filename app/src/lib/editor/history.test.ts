import { History, HISTORY_STEPS } from './history.svelte';

test('history keeps 100 steps', () => {
  const h = new History(0);
  for (let i = 1; i <= 150; i++) h.push(i);
  let undone = 0;
  while (h.undo() !== undefined) undone++;
  expect(HISTORY_STEPS).toBe(100);
  expect(undone).toBe(100);
  expect(h.present).toBe(50);
  expect(h.canUndo).toBe(false);
  expect(h.canRedo).toBe(true);
});

test('push after undo drops the redo branch', () => {
  const h = new History('a');
  h.push('b');
  h.push('c');
  expect(h.undo()).toBe('b');
  expect(h.canRedo).toBe(true);
  h.push('d');
  expect(h.canRedo).toBe(false);
  expect(h.redo()).toBeUndefined();
  expect(h.undo()).toBe('b');
  expect(h.undo()).toBe('a');
});

test('a drag is one step', () => {
  const h = new History({ x: 0 });
  h.begin();
  h.push({ x: 1 });
  h.push({ x: 2 });
  h.push({ x: 3 });
  h.commit();
  expect(h.present).toEqual({ x: 3 });
  expect(h.undo()).toEqual({ x: 0 });
  expect(h.canUndo).toBe(false);
  expect(h.redo()).toEqual({ x: 3 });
});

test('a drag that changes nothing adds no step', () => {
  const h = new History({ x: 0 });
  h.begin();
  h.commit();
  expect(h.canUndo).toBe(false);
});
