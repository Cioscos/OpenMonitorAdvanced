<script lang="ts">
  import { formatDuration } from '../../lib/format';
  import type { HealthState } from '../../lib/health';
  import { t } from '../../lib/i18n/index.svelte';

  let { health, nowMs }: { health: HealthState; nowMs: number } = $props();
</script>

<section
  class="banner"
  class:ok={health.level === 'ok'}
  class:warn={health.level === 'warn'}
  class:crit={health.level === 'crit'}
  role="status"
>
  <div class="dot" aria-hidden="true">{health.level === 'neutral' ? '•' : health.level === 'ok' ? '✓' : '!'}</div>
  <div>
    <div class="title">{t(health.messageKey, health.params)}</div>
    <div class="sub">{t('health.since', { duration: formatDuration(nowMs - health.sinceMs, t) })}</div>
  </div>
</section>

<style>
  .banner {
    --state: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: linear-gradient(135deg, color-mix(in srgb, var(--state) 14%, var(--surface)), var(--surface));
  }
  .ok { --state: var(--ok); }
  .warn {
    --state: var(--warn);
  }
  .crit {
    --state: var(--crit);
  }
  .dot {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    font-size: 20px;
    color: var(--state);
    background: color-mix(in srgb, var(--state) 18%, transparent);
  }
  .title {
    font-size: 20px;
    font-weight: 600;
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
