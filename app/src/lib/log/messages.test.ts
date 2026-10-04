import { describe, expect, it } from 'vitest';
import { i18n, t } from '../i18n/index.svelte';
import { folderErrorText } from './messages';

describe('folderErrorText', () => {
  it.each(['log.error.folderMissing', 'shell.error.missing', 'shell.error.timeout'])('translates %s', (key) => {
    i18n.locale = 'en';
    expect(folderErrorText(key, t)).toBe(t(key));
    expect(folderErrorText(key, t)).not.toBe(key);
  });

  it('shows a system text as it came', () => {
    expect(folderErrorText('Access is denied.', t)).toBe('Access is denied.');
  });
});
