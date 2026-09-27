import uPlot from 'uplot';

/** Draw only series geometry; never blur the canvas containing axes, labels and grid. */
export function drawChartSeriesDecoration(
  plot: uPlot,
  seriesIdx: number,
  strokePath: Path2D | null,
  gapsClip: Path2D | null,
  color: string,
): void {
  const series = plot.series[seriesIdx];
  if (!series || series.show === false) return;
  const { ctx, bbox } = plot;
  const ratio = uPlot.pxRatio;
  ctx.save();
  try {
    ctx.beginPath();
    ctx.rect(bbox.left, bbox.top, bbox.width, bbox.height);
    ctx.clip();
    if (gapsClip) ctx.clip(gapsClip);
    ctx.shadowBlur = 0;
    ctx.setLineDash([]);
    if (strokePath) {
      // Match the SVG's narrow translucent halo without a full-canvas filter.
      ctx.strokeStyle = color;
      ctx.lineWidth = 6 * ratio;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.globalAlpha = 0.18;
      ctx.stroke(strokePath);
    }

    const last = plot.data[0].length - 1;
    const timestamp = plot.data[0][last];
    const value = plot.data[seriesIdx]?.[last];
    // A missing latest sample is a gap, not permission to mark an older sample.
    if (timestamp == null || value == null || !Number.isFinite(timestamp) || !Number.isFinite(value)) return;
    const x = plot.valToPos(timestamp, 'x', true);
    const y = plot.valToPos(value, series.scale!, true);
    if (!Number.isFinite(x) || !Number.isFinite(y) || x < bbox.left || x > bbox.left + bbox.width || y < bbox.top || y > bbox.top + bbox.height) return;
    ctx.globalAlpha = 1;
    ctx.fillStyle = '#fff';
    ctx.beginPath();
    ctx.arc(x, y, 3 * ratio, 0, Math.PI * 2);
    ctx.fill();
  } finally {
    ctx.restore();
  }
}
