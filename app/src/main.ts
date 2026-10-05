import { mount } from 'svelte';
import App from './App.svelte';
import EditorApp from './editor/EditorApp.svelte';
import { detectLocale, i18n } from './lib/i18n/index.svelte';
import './styles/theme.css';

// The language follows `settings.general.language` once the settings arrive (`SettingsStore.accept`).
i18n.locale = detectLocale(navigator.languages);
document.documentElement.lang = i18n.locale;
const editor = new URLSearchParams(location.search).get('window') === 'overlay-editor';
mount(editor ? EditorApp : App, { target: document.getElementById('app')! });
