// The data the editor's canvas draws: frame metrics and frame times from `overlay-editor-data`
// (DD14), sensor values and history from the `LiveStore`.

import type { Backend, Unsubscribe } from '../lib/backend/backend';
import { sensorLabel } from '../lib/advanced/labels';
import type { FrameMetric, Source, Stat } from '../lib/editor/profile';
import type { Translate } from '../lib/i18n/index.svelte';
import type { LiveStore } from '../lib/live.svelte';
import type { EditorData, FrameMetrics, WireFrameTime } from '../lib/types';
import { statOf, type Readout } from './draw';

/** Longest chart range and statistic window of the format (300 s). */
const KEEP_S = 300;
// ponytail: a frame-count cap rather than a per-profile window; 60 000 frames are 300 s at 200 FPS.
const MAX_FRAMES = 60_000;

export function metricValue(m: FrameMetrics, metric: FrameMetric): number | null {
  switch (metric) {
    case 'fps-displayed':
      return m.fps_displayed;
    case 'fps-rendered':
      return m.fps_rendered;
    case 'fps-presented':
      return m.fps_presented;
    case 'frametime-displayed':
      return m.frametime_displayed_ms;
    case 'frametime-app':
      return m.frametime_app_ms;
    case 'fg-multiplier':
      return m.fg_multiplier;
    case 'stutter':
      return m.stutter_count;
    case 'latency-pc':
      return m.latency_pc_ms;
    case 'latency-display':
      return m.latency_display_ms;
    default:
      return null;
  }
}

/** The frame data of the last 300 s; `version` moves at every change. */
export class FrameFeed {
  metrics = $state.raw<FrameMetrics | null>(null);
  version = $state(0);
  frameTimes: WireFrameTime[] = [];
  #ring: { t: number; m: FrameMetrics }[] = [];

  push(data: EditorData, nowS = performance.now() / 1000): void {
    this.metrics = data.metrics;
    this.#ring.push({ t: nowS, m: data.metrics });
    while (this.#ring.length > 0 && this.#ring[0].t < nowS - KEEP_S) this.#ring.shift();
    if (data.frameTimes.length > 0) {
      this.frameTimes.push(...data.frameTimes);
      const newest = this.frameTimes[this.frameTimes.length - 1].t_s;
      let drop = 0;
      while (drop < this.frameTimes.length && this.frameTimes[drop].t_s < newest - KEEP_S) drop++;
      drop = Math.max(drop, this.frameTimes.length - MAX_FRAMES);
      if (drop > 0) this.frameTimes.splice(0, drop);
    }
    this.version++;
  }

  /** `[t_s, value]` of a metric, oldest first. */
  samples(metric: FrameMetric): [number, number][] {
    const out: [number, number][] = [];
    for (const { t, m } of this.#ring) {
      const v = metricValue(m, metric);
      if (v !== null && Number.isFinite(v)) out.push([t, v]);
    }
    return out;
  }

  connect(backend: Backend): Promise<Unsubscribe> {
    return backend.onOverlayEditorData((data) => this.push(data));
  }
}

/**
 * A `Readout` over the live sensors and the frame feed, for one repaint: the samples of each
 * source are copied once and kept for the blocks that read them again.
 */
export function makeReadout(live: LiveStore, feed: FrameFeed, t: Translate, format: Readout['format']): Readout {
  const metrics = feed.metrics?.state === 'running' ? feed.metrics : null;
  const schema = new Map((live.schema?.sensors ?? []).map((s) => [s.id, s]));
  let times: number[] | null = null;
  const cache = new Map<string, [number, number][]>();
  const read = (source: Source): [number, number][] => {
    if ('sensor' in source) {
      times ??= live.seriesTimestampsMs();
      const out: [number, number][] = [];
      live.series(source.sensor).forEach((v, i) => {
        if (v !== null && Number.isFinite(v)) out.push([times![i] / 1000, v]);
      });
      return out;
    }
    return 'frames' in source && metrics !== null ? feed.samples(source.frames) : [];
  };
  const samples = (source: Source): [number, number][] => {
    if ('text' in source) return [];
    const key = 'sensor' in source ? `s:${source.sensor}` : `f:${source.frames}`;
    let found = cache.get(key);
    if (found === undefined) {
      found = read(source);
      cache.set(key, found);
    }
    return found;
  };
  const value = (source: Source, stat: Stat): number | null => {
    if ('text' in source) return null;
    if ('sensor' in source) return stat.op === 'current' ? live.value(source.sensor) : statOf(samples(source), stat);
    if (metrics === null) return null;
    const m = source.frames;
    if (m === 'low-1' || m === 'low-01') {
      const low = metrics.lows.find((l) => l.window_s === stat.window && l.definition === stat.definition);
      return (m === 'low-1' ? low?.one_percent : low?.point_one_percent) ?? null;
    }
    return stat.op === 'current' ? metricValue(metrics, m) : statOf(samples(source), stat);
  };
  return {
    value: (source, stat) => {
      const v = value(source, stat);
      return v !== null && Number.isFinite(v) ? v : null;
    },
    samples,
    frameTimes: feed.frameTimes,
    metrics,
    sensor: (id) => {
      const s = schema.get(id);
      return s === undefined ? undefined : { label: sensorLabel(s, t), unit: s.unit };
    },
    t,
    format,
  };
}
