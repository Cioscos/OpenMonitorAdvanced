<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
</script>

<p class="notice" role="status">{t('service.pageNotice')}</p>

<style>
  .notice {
    margin: 0;
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
