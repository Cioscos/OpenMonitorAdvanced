import type { Backend } from '../lib/backend';
import { theme } from '../lib/theme/theme.svelte';

/** Applies `vars` as the active theme through the theme store, as a theme change does. */
export async function setTheme(vars: Record<string, string>): Promise<void> {
  await theme.load({ themeResolved: async () => ({ id: 'test', vars, fallback: false }) } as unknown as Backend);
}
