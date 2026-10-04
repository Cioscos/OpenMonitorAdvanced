import { beforeEach, describe, expect, it } from 'vitest';
import { i18n, t } from './i18n/index.svelte';
import { openFailureText } from './openFailure';

describe('openFailureText', () => {
  beforeEach(() => (i18n.locale = 'en'));

  it.each(['shell.error.missing', 'shell.error.timeout', 'log.error.folderMissing'])('translates %s', (key) => {
    expect(openFailureText(key)).toBe(t('settings.openFailed', { reason: t(key) }));
    expect(openFailureText(key)).not.toContain(key);
  });

  it('passes a system text through unchanged', () => {
    expect(openFailureText('Access is denied. (os error 5)')).toBe(
      t('settings.openFailed', { reason: 'Access is denied. (os error 5)' }),
    );
    expect(openFailureText(new Error('boom'))).toBe(t('settings.openFailed', { reason: 'Error: boom' }));
  });
});
