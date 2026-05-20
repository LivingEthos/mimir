#!/usr/bin/env node
import { readdirSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const tsc = process.platform === "win32" ? "tsc.cmd" : "tsc";
const sources = readdirSync(root)
  .filter((name) => name.endsWith(".ts") && name !== "index.d.ts")
  .sort();

rmSync(join(root, "index.d.ts"), { force: true });

const commonArgs = [
  "--declaration",
  "--emitDeclarationOnly",
  "--lib",
  "es2020",
  "--skipLibCheck",
  "--outFile",
  "index.d.ts",
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
