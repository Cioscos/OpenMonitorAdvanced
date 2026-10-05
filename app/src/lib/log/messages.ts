import { catalogs, type Translate } from '../i18n/index.svelte';
import { OPEN_ERROR_KEYS } from '../openFailure';
import type { LogError } from '../types';

/** The translated reason of a log in `error` (`{detail}` is the system's message for `log.error.other`). */
export function logErrorText(error: LogError | null | undefined, t: Translate): string {
  if (!error) return '';
  return t(error.key, { detail: error.detail ?? '' });
}

/**
 * What "Open folder" (or a benchmark command) failed with: a `log.error.*` or `benchmark.error.*`
 * key, or the system's own text. A known key is translated; anything else is shown as it came.
 */
export function folderErrorText(reason: unknown, t: Translate): string {
  if (typeof reason === 'string' && /^(log|benchmark)\.error\./.test(reason) && reason in catalogs.en) return t(reason);
  if (typeof reason === 'string' && OPEN_ERROR_KEYS.includes(reason)) return t(reason);
  if (reason instanceof Error) return reason.message;
  return String(reason);
}
