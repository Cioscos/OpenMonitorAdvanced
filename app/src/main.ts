import { mount } from 'svelte';
import App from './App.svelte';
import { detectLocale, i18n } from './lib/i18n/index.svelte';
import './styles/theme.css';

i18n.locale = detectLocale(navigator.languages);
document.documentElement.lang = i18n.locale;
mount(App, { target: document.getElementById('app')! });
