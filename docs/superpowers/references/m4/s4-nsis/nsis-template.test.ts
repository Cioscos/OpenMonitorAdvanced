// Drift guard for the custom NSIS template (spec §10).
import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const appDir = resolve(__dirname, '../..');
const nsisDir = resolve(appDir, 'src-tauri/nsis');
const read = (p: string) => readFileSync(p, 'utf8').replace(/\r\n/g, '\n');

// The reference copy is named after the tauri-cli version it was taken from.
const upstreamFiles = readdirSync(nsisDir).filter((f) => /^upstream-.+\.nsi$/.test(f));
const baseVersion = upstreamFiles[0]?.match(/^upstream-(.+)\.nsi$/)?.[1];

describe('custom NSIS template', () => {
  it('has exactly one upstream reference copy', () => {
    expect(upstreamFiles).toHaveLength(1);
  });

  it('matches the @tauri-apps/cli version pinned and installed', () => {
    const pkg = JSON.parse(read(resolve(appDir, 'package.json')));
    const installed = createRequire(resolve(appDir, 'package.json'))('@tauri-apps/cli/package.json')
      .version as string;
    const pinned = pkg.devDependencies['@tauri-apps/cli'] as string;
    const hint = `template derived from ${baseVersion}: run scripts/nsis-template-drift.ps1 -To ${pinned}`;
    expect(pinned, hint).toBe(baseVersion);
    expect(installed, hint).toBe(baseVersion);
  });

  it('differs from upstream only by lines marked OMA', () => {
    // Added lines end with "; OMA ..."; template sections are hidden with "-".
    const ours = read(resolve(nsisDir, 'installer.nsi'))
      .split('\n')
      .filter((l) => !/ ; OMA( .*)?$/.test(l) || / ; OMA hidden$/.test(l))
      .map((l) => l.replace(/^Section -(\w+) ; OMA hidden$/, 'Section $1'))
      .join('\n');
    expect(ours).toBe(read(resolve(nsisDir, upstreamFiles[0])));
  });
});
