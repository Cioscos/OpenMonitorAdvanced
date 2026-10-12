<script lang="ts">
  // A miniature of the app drawn with a theme's variables (spec M10 §4.1): top bar, a tile with a
  // bar, a chart line and the three status colours. The variables are set on this element only, so
  // the rest of the page keeps the active theme.
  let { vars }: { vars: Record<string, string> } = $props();
</script>

<div
  class="screen"
  aria-hidden="true"
  style:--bg={vars['--bg']}
  style:--surface={vars['--surface']}
  style:--surface-2={vars['--surface-2']}
  style:--border={vars['--border']}
  style:--text={vars['--text']}
  style:--text-muted={vars['--text-muted']}
  style:--accent={vars['--accent']}
  style:--accent-2={vars['--accent-2']}
  style:--ok={vars['--ok']}
  style:--warn={vars['--warn']}
  style:--crit={vars['--crit']}
  style:--chart-grid={vars['--chart-grid']}
  style:--radius={vars['--radius']}
  style:--font={vars['--font']}
>
  <div class="bar">
    <span class="logo"></span>
    <span class="tab on"></span>
    <span class="tab"></span>
  </div>
  <div class="body">
    <div class="tile">
      <span class="value">Aa</span>
      <span class="label"></span>
      <span class="track"><span class="fill"></span></span>
    </div>
    <div class="tile chart">
      <svg viewBox="0 0 100 40" preserveAspectRatio="none">
        <path class="grid" d="M0 13.5H100M0 26.5H100" />
        <polyline points="0,32 14,27 26,30 40,17 54,21 68,9 82,14 100,6" />
      </svg>
      <span class="status"><i class="ok"></i><i class="warn"></i><i class="crit"></i></span>
    </div>
  </div>
</div>

<style>
  .screen {
    display: grid;
    grid-template-rows: 13% 1fr;
    aspect-ratio: 16 / 10;
    overflow: hidden;
    font-family: var(--font);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 7px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .logo {
    width: 7px;
    height: 7px;
    background: var(--accent);
    border-radius: 2px;
  }
  .tab {
    width: 22px;
    height: 3px;
    background: var(--text-muted);
    border-radius: 2px;
    opacity: 0.6;
  }
  .tab.on {
    background: var(--accent);
    opacity: 1;
  }
  .body {
    display: grid;
    grid-template-columns: 1fr 1.35fr;
    gap: 7px;
    padding: 8px;
    min-height: 0;
  }
  .tile {
    overflow: hidden;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 4px;
    min-width: 0;
    padding: 7px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: calc(var(--radius) / 2);
  }
  .value {
    font-size: 20px;
    font-weight: 700;
    line-height: 1;
    color: var(--text);
  }
  .label {
    width: 70%;
    height: 3px;
    background: var(--text-muted);
    border-radius: 2px;
  }
  .track {
    height: 5px;
    overflow: hidden;
    background: var(--surface-2);
    border-radius: 3px;
  }
  .fill {
    display: block;
    width: 64%;
    height: 100%;
    background: var(--accent);
  }
  .chart {
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    padding: 5px 5px 6px;
  }
  svg {
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .grid {
    fill: none;
    stroke: var(--chart-grid);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  polyline {
    fill: none;
    stroke: var(--accent-2);
    stroke-width: 1.6;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
  .status {
    display: flex;
    gap: 4px;
  }
  .status i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .ok {
    background: var(--ok);
  }
  .warn {
    background: var(--warn);
  }
  .crit {
    background: var(--crit);
  }
</style>
