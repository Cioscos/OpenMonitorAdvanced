import { cleanup, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { i18n } from '../lib/i18n/index.svelte';
import { settings } from '../lib/settings.svelte';
import { FakeBackend } from '../test/fake-backend';
import EditorApp from './EditorApp.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  settings.state = null;
});

test('a new profile is named in the language of the settings', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { language: 'it' } });
  // No profile file to open: the editor starts a new profile.
  render(EditorApp, { backend });
  expect(await screen.findByText('Nuovo profilo')).toBeTruthy();
});
