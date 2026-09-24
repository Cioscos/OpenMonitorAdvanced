import { mount } from 'svelte';
import App from './App.svelte';
import { detectLocale, i18n } from './lib/i18n/index.svelte';
import './styles/theme.css';

i18n.locale = detectLocale(navigator.languages);
mount(App, { target: document.getElementById('app')! });
