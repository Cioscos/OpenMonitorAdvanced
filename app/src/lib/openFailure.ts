import { t } from './i18n/index.svelte';

/** The backend's i18n keys for a target that could not be opened. */
export const OPEN_ERROR_KEYS = ['shell.error.missing', 'shell.error.timeout', 'log.error.folderMissing'];

/**
 * The text shown next to a button whose "open" failed. The backend rejects with one of
 * [`OPEN_ERROR_KEYS`] or with the system's own text; a key is translated, anything else is shown as it came.
 */
export function openFailureText(error: unknown): string {
  const reason = typeof error === 'string' && OPEN_ERROR_KEYS.includes(error) ? t(error) : String(error);
  return t('settings.openFailed', { reason });
}
