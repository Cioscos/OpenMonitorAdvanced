import { mount } from 'svelte';
import App from './App.svelte';
import EditorApp from './editor/EditorApp.svelte';
import { createBackend } from './lib/backend';
import { detectLocale, i18n } from './lib/i18n/index.svelte';
import { theme } from './lib/theme/theme.svelte';
import './styles/theme.css';
import './styles/fonts.css';

// The language follows `settings.general.language` once the settings arrive (`SettingsStore.accept`).
i18n.locale = detectLocale(navigator.languages);
document.documentElement.lang = i18n.locale;
const editor = new URLSearchParams(location.search).get('window') === 'overlay-editor';
const backend = createBackend();
// The theme is applied before mounting, so Synthwave never flashes; without it `theme.css` stays.
try {
  await theme.load(backend);
} catch (error) {
  console.error('theme unavailable', error);
}
mount(editor ? EditorApp : App, { target: document.getElementById('app')!, props: { backend } });
