import uPlot from 'uplot';
import { formatTimeTick } from './chartData';

export type ChartCanvasPath = Readonly<{
  stroke: Path2D | null;
  gapsClip: Path2D | null;
  color: string;
}>;

/**
 * The last real value of a series held flat from its sample (canvas px) to the right end of the
 * overscan. A visual projection only: it never enters uPlot data, legend, KPIs, stats or logs.
 */
export type ChartHeldSegment = Readonly<{
  x: number;
  y: number;
  color: string;
}>;

/** uPlot 1.6.32's default axis font family, in which it draws the Y axis labels. */
const AXIS_FONT_FAMILY = 'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, "Noto Sans", sans-serif, "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji"';

/**
 * Font of the X labels at a device pixel ratio: uPlot's 12 px axis font, rounded to whole
 * device pixels as uPlot scales it, so X and Y labels match. At ratio 1 it measures them in CSS px.
 */
export function timeTickFont(ratio: number): string {
  return `${Math.round(12 * ratio)}px ${AXIS_FONT_FAMILY}`;
}

/** Draw a snapshot in uPlot's canvas pixel coordinate system. */
export function drawChartCanvas(
  ctx: CanvasRenderingContext2D,
  plot: uPlot,
  paths: ReadonlyArray<ChartCanvasPath>,
  ticks: readonly number[],
  tickIncrementSeconds: number,
  locale: string,
  overscanPx: number,
  theme: { gridColor: string; textColor: string },
  held: ReadonlyArray<ChartHeldSegment> = [],
): void {
  const { left, top, width, height } = plot.bbox;
  const start = left - Math.max(0, overscanPx);
  const right = left + width + Math.max(0, overscanPx);
  const bottom = top + height;
  const ratio = uPlot.pxRatio;
  const tickXs = ticks
    .map((tick) => ({ tick, x: plot.valToPos(tick, 'x', true) }))
    .filter(({ x }) => Number.isFinite(x) && x >= start && x <= right);

  if (tickXs.length > 0) {
    ctx.save();
    try {
      ctx.beginPath();
      ctx.rect(start, top, right - start, height);
      ctx.clip();
      ctx.beginPath();
      for (const { x } of tickXs) {
        ctx.moveTo(x, top);
        ctx.lineTo(x, bottom);
      }
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      ctx.setLineDash([]);
      ctx.strokeStyle = theme.gridColor;
      ctx.lineWidth = ratio;
      ctx.stroke();
    } finally {
      ctx.restore();
    }
  }

  for (const { stroke, gapsClip, color } of paths) {
    if (!stroke) continue;
    ctx.save();
    try {
      ctx.beginPath();
      ctx.rect(start, top, right - start, height);
      ctx.clip();
      if (gapsClip) ctx.clip(gapsClip);
      ctx.shadowBlur = 0;
      ctx.setLineDash([]);
      ctx.strokeStyle = color;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.lineWidth = 6 * ratio;
      ctx.globalAlpha = 0.18;
      ctx.stroke(stroke);
      ctx.lineWidth = 1.5 * ratio;
      ctx.globalAlpha = 1;
      ctx.stroke(stroke);
    } finally {
      ctx.restore();
    }
  }

  // Drawn into the translated canvas, the held segments scroll with their curves, so frames
  // between samples need no per-series layer writes.
  for (const { x, y, color } of held) {
    ctx.save();
    try {
      ctx.beginPath();
      ctx.rect(start, top, right - start, height);
      ctx.clip();
      ctx.beginPath();
      ctx.moveTo(Math.max(start, x), y);
      ctx.lineTo(right, y);
      ctx.shadowBlur = 0;
      ctx.setLineDash([]);
      ctx.strokeStyle = color;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.lineWidth = 6 * ratio;
      ctx.globalAlpha = 0.18;
      ctx.stroke();
      ctx.lineWidth = 1.5 * ratio;
      ctx.globalAlpha = 1;
      ctx.stroke();
    } finally {
      ctx.restore();
    }
  }

  if (tickXs.length > 0) {
    ctx.save();
    try {
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      ctx.setLineDash([]);
      ctx.strokeStyle = theme.gridColor;
      ctx.fillStyle = theme.textColor;
      ctx.lineWidth = ratio;
      ctx.font = timeTickFont(ratio);
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      for (const { tick, x } of tickXs) {
        ctx.beginPath();
        ctx.moveTo(x, bottom);
        ctx.lineTo(x, bottom + 4 * ratio);
        ctx.stroke();
        ctx.fillText(formatTimeTick(tick, locale, tickIncrementSeconds), x, bottom + 6 * ratio);
      }
    } finally {
      ctx.restore();
    }
  }
}
