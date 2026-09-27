import uPlot from 'uplot';
import { formatTimeTick } from './chartData';

export type ChartCanvasPath = Readonly<{
  stroke: Path2D | null;
  gapsClip: Path2D | null;
  color: string;
}>;

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

  if (tickXs.length > 0) {
    ctx.save();
    try {
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      ctx.setLineDash([]);
      ctx.strokeStyle = theme.gridColor;
      ctx.fillStyle = theme.textColor;
      ctx.lineWidth = ratio;
      ctx.font = `${12 * ratio}px sans-serif`;
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
