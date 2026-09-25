import type { SensorStats } from '../types';

interface Acc {
  min: number;
  max: number;
  sum: number;
  count: number;
}

/** Browser-side stand-in for oma-core's `Stats`: min/max/avg per sensor id. */
export class StatsAccumulator {
  #acc = new Map<string, Acc>();

  /** Adds one tick; `values[i]` belongs to `ids[i]`. Null and non-finite values are skipped. */
  push(ids: string[], values: (number | null)[]): void {
    ids.forEach((id, i) => {
      const v = values[i];
      if (v === null || v === undefined || !Number.isFinite(v)) return;
      const acc = this.#acc.get(id);
      if (!acc) {
        this.#acc.set(id, { min: v, max: v, sum: v, count: 1 });
        return;
      }
      acc.min = Math.min(acc.min, v);
      acc.max = Math.max(acc.max, v);
      acc.sum += v;
      acc.count++;
    });
  }

  get(ids: string[]): (SensorStats | null)[] {
    return ids.map((id) => {
      const acc = this.#acc.get(id);
      return acc ? { min: acc.min, max: acc.max, avg: acc.sum / acc.count, count: acc.count } : null;
    });
  }

  reset(ids: string[]): void {
    ids.forEach((id) => this.#acc.delete(id));
  }
}
