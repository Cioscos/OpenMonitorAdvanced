import { untrack } from 'svelte';
import type { Backend } from '../backend';
import type { SettingsStore } from '../settings.svelte';
import { applyTheme } from './apply';

/** The applied theme: `version` moves at every application, so canvas charts read the palette again. */
class ThemeStore {
  version = $state(0);
  /** The saved theme could not be loaded and Synthwave is in use. */
  fallback = $state(false);
  #keys: string[] = [];
  #seq = 0;

  /** Reads the active theme from the core and applies it to the document root. */
  async load(backend: Pick<Backend, 'themeResolved'>): Promise<void> {
    const seq = ++this.#seq;
    const resolved = await backend.themeResolved();
    // A later load won the race: its theme is the current one.
    if (seq !== this.#seq) return;
    this.#keys = applyTheme(resolved.vars, document.documentElement, this.#keys);
    this.fallback = resolved.fallback;
    this.version++;
  }

  /**
   * Reloads the theme when `appearance.theme` changes; the first id seen is the one applied before
   * mounting. Returns the function that stops following.
   */
  follow(backend: Pick<Backend, 'themeResolved'>, settings: SettingsStore): () => void {
    let last: string | undefined;
    return $effect.root(() => {
      $effect(() => {
        const id = settings.state?.settings.appearance.theme;
        if (id === undefined) return;
        untrack(() => {
          if (last !== undefined && id !== last) this.load(backend).catch((error) => console.error('theme unavailable', error));
          last = id;
        });
      });
    });
  }
}

export const theme = new ThemeStore();
