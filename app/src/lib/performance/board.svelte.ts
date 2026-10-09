import type { Backend, Unsubscribe } from '../backend/backend';
import type { Board, BoardRow, BoardTable } from '../types';

/**
 * The leaderboard table (author rows plus the downloaded community copy) for the Classifica page
 * and the model reference of the gauges. Connected only while the Performance view is on screen;
 * the download itself is the shell's business (once a day, never with the setting off).
 */
class BoardStore {
  table = $state.raw<BoardTable | null>(null);
  loading = $state(false);
  /** The category of the last failed refresh (the table's own `error` is the last download's). */
  error = $state<string | null>(null);
  #backend: Backend | null = null;
  #generation = 0;

  /** Reads the local table; the network stays untouched until `refresh`. */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.#backend = backend;
    this.table = null;
    this.error = null;
    const stop = () => {
      if (this.#generation === generation) {
        this.#generation++;
        this.#backend = null;
        this.loading = false;
        this.table = null;
      }
    };
    try {
      const table = await backend.performanceBoard();
      if (this.#generation === generation) this.table = table;
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /** Asks the shell for a fresh table; it decides whether a download is due (`manual` forces it). */
  async refresh(manual: boolean): Promise<void> {
    const backend = this.#backend;
    if (backend === null) return;
    const generation = this.#generation;
    this.loading = true;
    try {
      const table = await backend.performanceBoardRefresh(manual);
      if (this.#generation === generation) {
        this.table = table;
        this.error = null;
      }
    } catch (error) {
      if (this.#generation === generation) this.error = String(error);
    } finally {
      if (this.#generation === generation) this.loading = false;
    }
  }

  /** The rows of `board` in the current score version, highest first. */
  rowsFor(board: Board): BoardRow[] {
    const t = this.table;
    if (!t) return [];
    const version = t.versions[board === 'disk' ? 'disk' : board.startsWith('gpu') ? 'gpu' : 'cpu'];
    return t.rows.filter((r) => r.board === board && r.scoreVersion === version);
  }
}

export const boardStore = new BoardStore();
