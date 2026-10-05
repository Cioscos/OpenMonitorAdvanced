/** Undo steps kept per editor session (§7.2). */
export const HISTORY_STEPS = 100;

/**
 * Undo and redo over immutable states. `present` is the current state; `push` makes a new one.
 * Between `begin()` and `commit()` (a drag) pushes only replace the present, and the commit
 * records the whole gesture as one step.
 */
export class History<T> {
  canUndo = $state(false);
  canRedo = $state(false);
  #past: T[] = [];
  #future: T[] = [];
  #present: T;
  /** The present when `begin()` was called, while a gesture is open. */
  #start: { state: T } | null = null;

  constructor(initial: T) {
    this.#present = initial;
  }

  get present(): T {
    return this.#present;
  }

  /** Forgets every step and starts again from `state`. */
  reset(state: T): void {
    this.#past = [];
    this.#future = [];
    this.#start = null;
    this.#present = state;
    this.#sync();
  }

  push(state: T): void {
    if (this.#start !== null) {
      this.#present = state;
      return;
    }
    this.#past.push(this.#present);
    if (this.#past.length > HISTORY_STEPS) this.#past.shift();
    this.#future = [];
    this.#present = state;
    this.#sync();
  }

  begin(): void {
    this.#start ??= { state: this.#present };
  }

  commit(): void {
    if (this.#start === null) return;
    const { state } = this.#start;
    this.#start = null;
    if (state === this.#present) return;
    const end = this.#present;
    this.#present = state;
    this.push(end);
  }

  /** The state to show after undoing, or undefined when there is nothing to undo. */
  undo(): T | undefined {
    const previous = this.#past.pop();
    if (previous === undefined) return undefined;
    this.#future.push(this.#present);
    this.#present = previous;
    this.#sync();
    return previous;
  }

  redo(): T | undefined {
    const next = this.#future.pop();
    if (next === undefined) return undefined;
    this.#past.push(this.#present);
    this.#present = next;
    this.#sync();
    return next;
  }

  #sync() {
    this.canUndo = this.#past.length > 0;
    this.canRedo = this.#future.length > 0;
  }
}
