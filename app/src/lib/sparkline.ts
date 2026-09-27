const round = (n: number) => Math.round(n * 10) / 10;

type Point = { x: number; y: number };

/** Plot timestamped samples in a moving time window. Missing samples break the curve. */
export function sparklineGeometry(
  values: number[],
  timestampsMs: number[],
  rightEdgeMs: number,
  windowMs: number,
  width: number,
  height: number,
  min: number,
  max?: number,
): { path: string; endpoint: Point | null } {
  if (windowMs <= 0 || width <= 0 || height <= 0 || !Number.isFinite(rightEdgeMs)) {
    return { path: '', endpoint: null };
  }
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return { path: '', endpoint: null };
  const hi = Math.max(min, max ?? Math.max(min, ...finite));
  const span = hi - min || 1;
  const leftEdgeMs = rightEdgeMs - windowMs;
  const project = (value: number, time: number): Point => ({
    x: ((time - leftEdgeMs) / windowMs) * width,
    y: height - ((Math.min(Math.max(value, min), hi) - min) / span) * height,
  });

  const runs: Point[][] = [];
  let run: Point[] = [];
  const count = Math.min(values.length, timestampsMs.length);
  for (let i = 0; i < count; i++) {
    const time = timestampsMs[i];
    if (!Number.isFinite(values[i]) || !Number.isFinite(time)) {
      if (run.length) runs.push(run);
      run = [];
      continue;
    }
    const point = project(values[i], time);
    if (run.length && point.x < run[run.length - 1].x) {
      runs.push(run);
      run = [];
    }
    if (run.length && point.x === run[run.length - 1].x) run[run.length - 1] = point;
    else run.push(point);
  }
  if (run.length) runs.push(run);

  const clip = (a: Point, b: Point, x: number): Point => ({
    x,
    y: a.y + ((b.y - a.y) * (x - a.x)) / (b.x - a.x),
  });
  const visibleRuns: Point[][] = runs.map((points) => {
    const visible: Point[] = [];
    for (let i = 0; i < points.length; i++) {
      const point = points[i];
      if (point.x >= 0 && point.x <= width) visible.push(point);
      if (i === 0) continue;
      const prev = points[i - 1];
      if (prev.x < 0 && point.x > 0) visible.unshift(clip(prev, point, 0));
      if (prev.x < width && point.x > width) visible.push(clip(prev, point, width));
    }
    return visible;
  }).filter((points) => points.length > 0);

  const segmentPath = (points: Point[]): string => {
    let path = `M${round(points[0].x)} ${round(points[0].y)}`;
    if (points.length === 1) return path;
    const slopes = points.slice(1).map((p, i) => (p.y - points[i].y) / (p.x - points[i].x));
    const tangents = points.map((_, i) => {
      if (i === 0) return slopes[0];
      if (i === points.length - 1) return slopes[slopes.length - 1];
      const before = slopes[i - 1];
      const after = slopes[i];
      if (before * after <= 0) return 0;
      return Math.sign(before) * Math.min(Math.abs(before), Math.abs(after));
    });
    for (let i = 0; i < slopes.length; i++) {
      const a = points[i];
      const b = points[i + 1];
      const third = (b.x - a.x) / 3;
      const lo = Math.min(a.y, b.y);
      const hiY = Math.max(a.y, b.y);
      const y1 = Math.min(hiY, Math.max(lo, a.y + tangents[i] * third));
      const y2 = Math.min(hiY, Math.max(lo, b.y - tangents[i + 1] * third));
      path += `C${round(a.x + third)} ${round(y1)} ${round(b.x - third)} ${round(y2)} ${round(b.x)} ${round(b.y)}`;
    }
    return path;
  };
  const lastIndex = count - 1;
  const lastTime = timestampsMs[lastIndex];
  const endpoint = lastIndex >= 0 && Number.isFinite(values[lastIndex]) && Number.isFinite(lastTime)
    && lastTime >= leftEdgeMs && lastTime <= rightEdgeMs
    ? project(values[lastIndex], lastTime)
    : null;
  return { path: visibleRuns.map(segmentPath).join(''), endpoint: endpoint && { x: round(endpoint.x), y: round(endpoint.y) } };
}

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
