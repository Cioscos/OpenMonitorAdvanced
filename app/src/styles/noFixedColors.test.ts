import { expect, test } from 'vitest';

const files = import.meta.glob('../**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

// Literal colours in <style> blocks that are allowed to stay: "file::reason" keyed by a substring of the match line.
const EXCEPTIONS: { file: string; pattern: RegExp; reason: string }[] = [
  { file: 'performance/Gauge.svelte', pattern: /#[0-9a-f]{3,8}/i, reason: 'the dial is a physical object with a fixed dark face; its labels sit on that face in every theme' },
  { file: 'editor/Canvas.svelte', pattern: /#140F1E/i, reason: 'simulated game background behind the overlay, not a theme colour' },
  { file: 'advanced/ChartStrip.svelte', pattern: /#607D8B|rgba\(0,\s*0,\s*0,\s*\.07\)/, reason: 'comment quoting uPlot.min.css defaults, not a style value' },
  { file: 'advanced/HistoryChart.svelte', pattern: /#607D8B|rgba\(0,\s*0,\s*0,\s*\.07\)/, reason: 'comment quoting uPlot.min.css defaults, not a style value' },
];

const COLOR = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|(?<![-\w])(black|white)(?![-\w])/;

function styles(src: string): string[] {
  return [...src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]);
}

test('component styles use theme variables, not fixed colours', () => {
  const found: string[] = [];
  for (const [path, src] of Object.entries(files)) {
    const rel = path.replace(/^\.\.\//, '');
    for (const block of styles(src)) {
      block.split('\n').forEach((line) => {
        if (!COLOR.test(line)) return;
        if (EXCEPTIONS.some((e) => rel.endsWith(e.file) && e.pattern.test(line))) return;
        found.push(`${rel}: ${line.trim()}`);
      });
    }
  }
  expect(found).toEqual([]);
});
