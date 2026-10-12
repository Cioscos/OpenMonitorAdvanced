import { expect, test } from 'vitest';

const files = import.meta.glob('../**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

// Literal colours in <style> blocks that are allowed to stay: "file::reason" keyed by a substring of the match line.
const EXCEPTIONS: { file: string; pattern: RegExp; reason: string }[] = [
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

// Second pass: markup outside <style> (inline style=/style: values and SVG presentation attributes).
// Editor scripts hold overlay profile defaults, which are data, and are not markup.
const MARKUP_EXCEPTIONS: { file: string; pattern: RegExp; reason: string }[] = [];

const LITERAL = String.raw`(#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|(?<![-\w])(black|white)(?![-\w]))`;
const MARKUP_COLOR = new RegExp(
  String.raw`\b(fill|stroke|stop-color|flood-color)\s*=\s*["']${LITERAL}|style\s*=\s*"[^"]*${LITERAL}|style:[\w-]+\s*=\s*["']${LITERAL}`,
);

test('component markup has no fixed colours in inline styles or SVG attributes', () => {
  const found: string[] = [];
  for (const [path, src] of Object.entries(files)) {
    const rel = path.replace(/^\.\.\//, '');
    const markup = src.replace(/<style[^>]*>[\s\S]*?<\/style>/g, '').replace(/<script[^>]*>[\s\S]*?<\/script>/g, '');
    markup.split('\n').forEach((line) => {
      if (!MARKUP_COLOR.test(line)) return;
      if (MARKUP_EXCEPTIONS.some((e) => rel.endsWith(e.file) && e.pattern.test(line))) return;
      found.push(`${rel}: ${line.trim()}`);
    });
  }
  expect(found).toEqual([]);
});
