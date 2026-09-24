const round = (n: number) => Math.round(n * 10) / 10;

/**
 * SVG path for a sparkline. NaN values break the line; series shorter than
 * `capacity` are right-aligned so new samples always enter from the right.
 */
export function sparklinePath(
  values: number[],
  width: number,
  height: number,
  min = 0,
  max?: number,
  capacity = values.length,
): string {
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return '';
  const hi = max ?? Math.max(min, ...finite);
  const span = hi - min || 1;
  const step = capacity > 1 ? width / (capacity - 1) : 0;
  const offset = capacity - values.length;
  let path = '';
  let drawing = false;
  values.forEach((v, i) => {
    if (!Number.isFinite(v)) {
      drawing = false;
      return;
    }
    const clamped = Math.min(Math.max(v, min), hi);
    const x = (offset + i) * step;
    const y = height - ((clamped - min) / span) * height;
    path += `${drawing ? 'L' : 'M'}${round(x)} ${round(y)}`;
    drawing = true;
  });
  return path;
}
