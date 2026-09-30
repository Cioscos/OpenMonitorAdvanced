import { catalogs, type Translate } from '../i18n/index.svelte';
import type { LogError } from '../types';

/** The translated reason of a log in `error` (`{detail}` is the system's message for `log.error.other`). */
export function logErrorText(error: LogError | null | undefined, t: Translate): string {
  if (!error) return '';
  return t(error.key, { detail: error.detail ?? '' });
}

/**
 * What "Open folder" failed with: `openLogFolder` rejects with a `log.error.*` key or with the
 * system's own text. A known key is translated; anything else is shown as it came.
 */
export function folderErrorText(reason: unknown, t: Translate): string {
  if (typeof reason === 'string' && reason.startsWith('log.error.') && reason in catalogs.en) return t(reason);
  if (reason instanceof Error) return reason.message;
  return String(reason);
}
