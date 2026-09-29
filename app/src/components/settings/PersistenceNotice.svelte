<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import type { Persistence } from '../../lib/types';

  // Where the settings stand on disk, when there is something to say: nothing for `ok` and
  // `pending` (a save on its way is normal).
  let { persistence }: { persistence: Persistence } = $props();

  const message = $derived.by(() => {
    switch (persistence.kind) {
      case 'recovered':
        return t('settings.persistence.recovered', { path: persistence.path });
      case 'readOnly':
        return t('settings.persistence.readOnly');
      case 'error':
        return t('settings.persistence.error', { reason: persistence.reason });
      default:
        return null;
    }
  });
</script>

{#if message !== null}
  <p class="persistence" class:error={persistence.kind === 'error'} role="status">{message}</p>
{/if}

<style>
  .persistence {
    margin: 0;
    padding: 10px 14px;
    font-size: 13px;
    line-height: 1.45;
    overflow-wrap: anywhere;
    user-select: text;
    border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--warn) 8%, var(--surface));
  }
  .error {
    border-color: color-mix(in srgb, var(--crit) 50%, transparent);
    background: color-mix(in srgb, var(--crit) 8%, var(--surface));
  }
</style>
