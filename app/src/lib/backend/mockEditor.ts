// The overlay editor's side of the mock backend: an in-memory profile store, fake frames for the
// canvas and a fixed benchmark history, so `pnpm dev` with `?window=overlay-editor` shows the editor.

import { uniqueName } from '../editor/ops';
import { newBlock, withDefaults, type Profile, type Source } from '../editor/profile';
import { t } from '../i18n/index.svelte';
import type { BenchmarkEntry, CommandError, EditableProfile, EditorData, OverlayProfileEntry } from '../types';

export const MOCK_BUILTIN_IDS = ['builtin-minimal-fps', 'builtin-gaming', 'builtin-full', 'builtin-bar'] as const;
export const MOCK_USER_PROFILE_ID = '6f1c2a9e-3b47-4d8a-9e15-0c2b7d4f8a31';

const PROFILE_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const notFound = (): CommandError => ({ key: 'editor.error.notFound', detail: null });

/** A small profile with the given sources stacked in one column. */
function stacked(name: string, sources: Source[]): Profile {
  const profile = withDefaults({ format: 1, name });
  let y = 0;
  for (const source of sources) {
    const block = newBlock(source, { x: 0, y }, profile.blocks);
    profile.blocks.push(block);
    y += block.rect.h;
  }
  return profile;
}

function builtin(id: string): Profile {
  const fps: Source = { frames: 'fps-displayed' };
  const sources: Source[] =
    id === 'builtin-minimal-fps'
      ? [fps]
      : id === 'builtin-gaming'
        ? [fps, { frames: 'low-1' }, { sensor: 'cpu/0/load/total' }, { sensor: 'gpu/pci-0000:01:00.0/load/core' }, { frames: 'frametime-displayed' }]
        : [fps, { frames: 'low-1' }, { frames: 'fg-multiplier' }, { sensor: 'cpu/0/load/total' }, { sensor: 'memory/0/load/used' }];
  return stacked(t(`overlay.template.${id}`), sources);
}

/** The user profiles in memory; `onChange` runs after every write (the catalog changed). */
export function mockProfileStore(onChange: () => void) {
  const user = new Map<string, Profile>([[MOCK_USER_PROFILE_ID, stacked('Stream (1440p)', [{ frames: 'fps-displayed' }, { frames: 'frametime-displayed' }])]]);
  const isBuiltin = (id: string) => (MOCK_BUILTIN_IDS as readonly string[]).includes(id);
  const profile = (id: string): Profile => {
    if (isBuiltin(id)) return builtin(id);
    const found = user.get(id);
    if (found === undefined) throw notFound();
    return structuredClone(found);
  };
  const create = (p: Profile): string => {
    const id = crypto.randomUUID();
    user.set(id, { ...p, name: uniqueName([...user.values()].map((u) => u.name), p.name) });
    onChange();
    return id;
  };
  const parse = (json: string): Profile => {
    try {
      return withDefaults(JSON.parse(json));
    } catch (e) {
      throw { key: 'editor.error.invalid', detail: String(e) } satisfies CommandError;
    }
  };
  return {
    entries: (): OverlayProfileEntry[] => [
      ...MOCK_BUILTIN_IDS.map((id) => ({ id, name: `overlay.template.${id}`, builtin: true })),
      ...[...user].map(([id, p]) => ({ id, name: p.name, builtin: false })),
    ],
    load: (id: string): EditableProfile => ({ id, builtin: isBuiltin(id), json: JSON.stringify(profile(id)) }),
    save(id: string | null, json: string): string {
      const p = parse(json);
      if (id === null) return create(p);
      if (isBuiltin(id)) throw { key: 'editor.error.readOnly', detail: null } satisfies CommandError;
      if (!PROFILE_ID.test(id)) throw { key: 'editor.error.invalidId', detail: null } satisfies CommandError;
      user.set(id, p);
      onChange();
      return id;
    },
    remove(id: string): void {
      if (!user.delete(id)) throw notFound();
      onChange();
    },
    duplicate: (id: string): string => create(profile(id)),
  };
}

/**
 * Fake `overlay-editor-data` while someone listens: every 100 ms the frames of a ~72 FPS game with
 * frame generation (×2) and a stutter now and then, plus the metrics they give.
 */
export function mockEditorData() {
  const listeners = new Set<(d: EditorData) => void>();
  let timer: ReturnType<typeof setInterval> | undefined;
  let t_s = 0;
  let n = 0;
  const tick = () => {
    const frameTimes = [];
    while (frameTimes.length < 14) {
      n++;
      const ms = n % 90 === 0 ? 48 : 6.9 + Math.sin(n / 7) * 0.8;
      t_s += ms / 1000;
      frameTimes.push({ t_s, displayed_ms: ms, app_ms: ms * 2 });
    }
    const data: EditorData = {
      metrics: {
        state: 'running',
        fps_displayed: 144 + Math.sin(n / 50) * 3,
        fps_rendered: 72,
        fps_presented: 144,
        rendered_source: 'FG',
        fg_suspected: false,
        frametime_displayed_ms: 6.9,
        frametime_app_ms: 13.9,
        fg_multiplier: 2,
        stutter_count: Math.floor(n / 90),
        stutter_percent: 1.1,
        latency_pc_ms: 28.4,
        latency_display_ms: 9.6,
        bound: 'gpu',
        lows: [{ window_s: 10, definition: 'integral', one_percent: 61.2, point_one_percent: 38.5 }],
      },
      frameTimes,
    };
    listeners.forEach((cb) => cb(data));
  };
  return {
    subscribe(cb: (d: EditorData) => void) {
      listeners.add(cb);
      timer ??= setInterval(tick, 100);
      return () => {
        listeners.delete(cb);
        if (listeners.size === 0 && timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
  };
}

const lows = (one: number, pointOne: number) => ({ onePercent: one, pointOnePercent: pointOne });

export const MOCK_BENCHMARKS: BenchmarkEntry[] = [
  {
    id: 'cyberpunk2077-20261005-213000',
    record: {
      format: 1,
      game: 'cyberpunk2077.exe',
      startedAt: '2026-10-05T21:30:00',
      endReason: 'user',
      summary: {
        durationS: 62.4,
        framesTotal: 8986,
        framesDisplayed: 8986,
        framesGenerated: 4493,
        fpsDisplayed: 144,
        fpsRendered: 72,
        renderedSource: 'FG',
        lowsIntegral: lows(61.2, 38.5),
        lowsPercentile: lows(64.8, 41.0),
        frametimeMinMs: 5.9,
        frametimeMaxMs: 48.1,
        stutterCount: 11,
        stutterPercent: 1.1,
        fgMultiplier: 2,
        latencyPcMs: 28.4,
        latencyDisplayMs: 9.6,
      },
    },
  },
];
