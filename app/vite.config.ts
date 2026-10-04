import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// With OMA_LICENSE_MANIFEST=<file>, records which node_modules packages have code in the
// bundle, as a JSON array of { name, dir } sorted by name and dir, for
// scripts/generate-licenses.ps1. Without the variable the plugin is not added at all, so the
// normal build output is unchanged.
function licenseManifest(file: string): Plugin {
  return {
    name: 'oma-license-manifest',
    apply: 'build',
    generateBundle(_options, bundle) {
      const packages = new Map<string, { name: string; dir: string }>();
      for (const output of Object.values(bundle)) {
        if (output.type !== 'chunk') continue;
        for (const [id, info] of Object.entries(output.modules)) {
          if (info.renderedLength === 0) continue;
          const path = id.replace(/^\0/, '').split('?')[0].replace(/\\/g, '/');
          const marker = '/node_modules/';
          const at = path.lastIndexOf(marker);
          if (at < 0) continue;
          const parts = path.slice(at + marker.length).split('/');
          const name = parts[0].startsWith('@') ? `${parts[0]}/${parts[1]}` : parts[0];
          const dir = path.slice(0, at + marker.length + name.length);
          packages.set(`${name}\n${dir}`, { name, dir });
        }
      }
      const list = [...packages.values()].sort((a, b) =>
        a.name === b.name ? (a.dir < b.dir ? -1 : 1) : a.name < b.name ? -1 : 1,
      );
      mkdirSync(dirname(file), { recursive: true });
      writeFileSync(file, `${JSON.stringify(list, null, 2)}\n`);
    },
  };
}

const manifest = process.env.OMA_LICENSE_MANIFEST;

export default defineConfig({
  plugins: [svelte(), ...(manifest ? [licenseManifest(manifest)] : [])],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test-setup.ts'],
  },
});
