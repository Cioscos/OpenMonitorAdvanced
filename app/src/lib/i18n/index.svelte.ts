import en from './en.json';
import it from './it.json';

export type Locale = 'en' | 'it';
export type Params = Record<string, string | number>;
export type Translate = (key: string, params?: Params) => string;

export const catalogs: Record<Locale, Record<string, string>> = { en, it };
const SUPPORTED: readonly Locale[] = ['en', 'it'];

class I18nState {
  locale = $state<Locale>('en');
}

/** Current UI language; reading it inside templates makes them reactive. */
export const i18n = new I18nState();

/** First supported language in the user's preference list, else English. */
export function detectLocale(languages: readonly string[]): Locale {
  for (const language of languages) {
    const base = language.toLowerCase().split('-')[0];
    const match = SUPPORTED.find((l) => l === base);
    if (match) return match;
  }
  return 'en';
}

/** The language the UI shows: the chosen one, or the browser's when the setting is `system`. */
export function resolveLocale(language: 'system' | Locale, languages: readonly string[]): Locale {
  return language === 'system' ? detectLocale(languages) : language;
}

export function translate(locale: Locale, key: string, params: Params = {}): string {
  const template = catalogs[locale][key] ?? catalogs.en[key] ?? key;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

export const t: Translate = (key, params) => translate(i18n.locale, key, params);
