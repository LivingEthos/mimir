#!/usr/bin/env node
import { readdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const tsc = process.platform === "win32" ? "tsc.cmd" : "tsc";
const sources = readdirSync(root)
  .filter((name) => name.endsWith(".ts") && name !== "index.d.ts")
  .sort();

const header = `/* eslint-disable */
/**
 * TypeScript declaration barrel for Mimir JSON Schemas.
 * Regenerate with npm run build.
 */

`;
const exports = sources
  .map((source) => `export * from "./${basename(source, ".ts")}";`)
  .join("\n");
writeFileSync(join(root, "index.d.ts"), `${header}${exports}\n`);

const commonArgs = [
  "--noEmit",
  "--lib",
  "es2020",
  "--skipLibCheck",
  ...sources,
];
const result = spawnSync(
  tsc,
  commonArgs,
  { cwd: root, stdio: "inherit" },
);

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}
