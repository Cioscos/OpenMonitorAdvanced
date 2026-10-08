// Minimal Node typings for the scripts (no @types/node: the Worker has no extra dependencies).
declare module "node:fs" {
  export function existsSync(path: string): boolean;
  export function readdirSync(path: string): string[];
  export function readFileSync(path: string, encoding: "utf8"): string;
  export function writeFileSync(path: URL | string, data: string): void;
}
declare module "node:path" {
  export function join(...parts: string[]): string;
}
declare const process: { argv: string[]; env: Record<string, string | undefined> };
interface ImportMeta {
  url: string;
}
