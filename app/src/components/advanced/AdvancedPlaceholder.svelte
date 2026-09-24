<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
</script>

<section class="placeholder">
  <p>{t('advanced.comingSoon')}</p>
</section>

<style>
  .placeholder {
    display: grid;
    place-items: center;
    min-height: 320px;
    color: var(--text-muted);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
  }
</style>
