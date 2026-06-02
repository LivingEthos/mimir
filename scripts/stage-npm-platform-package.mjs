#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const platformMap = new Map([
  ["darwin-arm64", { package: "cli-darwin-arm64", binary: "mimir" }],
  ["darwin-x64", { package: "cli-darwin-x64", binary: "mimir" }],
  ["linux-arm64", { package: "cli-linux-arm64", binary: "mimir" }],
  ["linux-x64", { package: "cli-linux-x64", binary: "mimir" }],
  ["win32-x64", { package: "cli-win32-x64", binary: "mimir.exe" }],
]);

function fail(message) {
  console.error(`Mimir npm staging failed: ${message}`);
  process.exit(1);
}

function usage() {
  console.log(`Usage:
  node scripts/stage-npm-platform-package.mjs --platform darwin-arm64 --binary path/to/mimir
  node scripts/stage-npm-platform-package.mjs --platform win32-x64 --binary path/to/mimir.exe

If omitted, --platform defaults to the current Node platform and --binary defaults
to target/release/mimir for the current host.`);
}

function parseArgs(argv) {
  const args = {
    platform: null,
    binary: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help") {
      usage();
      process.exit(0);
    }
    if (arg === "--platform" || arg === "--binary") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        fail(`${arg} requires a value`);
      }
      if (arg === "--platform") {
        args.platform = value;
      } else {
        args.binary = value;
      }
      index += 1;
      continue;
    }
    fail(`unknown argument ${arg}`);
  }

  return args;
}

function assertSourceBinary(sourcePath, platform) {
  const stat = fs.statSync(sourcePath);
  if (!stat.isFile()) {
    fail(`source binary is not a regular file: ${sourcePath}`);
  }

  const file = fs.openSync(sourcePath, "r");
  const buffer = Buffer.alloc(8);
  const bytesRead = fs.readSync(file, buffer, 0, buffer.length, 0);
  fs.closeSync(file);
  const header = buffer.subarray(0, bytesRead);
  const magic4 = header.subarray(0, 4).toString("hex");
  const machOMagic = new Set([
    "feedface",
    "feedfacf",
    "cefaedfe",
    "cffaedfe",
    "cafebabe",
    "bebafeca",
    "cafebabf",
    "bfbafeca",
  ]);

  const matches =
    (platform.startsWith("darwin-") && machOMagic.has(magic4)) ||
    (platform.startsWith("linux-") &&
      header.length >= 4 &&
      header[0] === 0x7f &&
      header.subarray(1, 4).toString("ascii") === "ELF") ||
    (platform.startsWith("win32-") && header.length >= 2 && header.subarray(0, 2).toString("ascii") === "MZ");

  if (!matches) {
    fail(`source file does not look like an unpacked native binary for ${platform}: ${sourcePath}`);
  }
}

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const detectedPlatform = `${os.platform()}-${os.arch()}`;
const args = parseArgs(process.argv.slice(2));
const platform = args.platform || detectedPlatform;
const source = args.binary || path.join(repoRoot, "target", "release", os.platform() === "win32" ? "mimir.exe" : "mimir");
const entry = platformMap.get(platform);

if (!entry) {
  fail(`unsupported platform ${platform}; expected one of ${[...platformMap.keys()].join(", ")}`);
}
if (!fs.existsSync(source)) {
  fail(`source binary not found: ${source}`);
}
assertSourceBinary(source, platform);

const packageDir = path.join(repoRoot, "packages", entry.package);
const rootBinary = path.join(packageDir, entry.binary);
const binBinary = path.join(packageDir, "bin", entry.binary);
const wrapperBinary = path.join(
  repoRoot,
  "packages",
  "cli",
  "bin",
  platform === "win32-x64" ? "mimir-win32-x64.exe" : `mimir-${platform}`,
);

for (const target of [rootBinary, binBinary, wrapperBinary]) {
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(source, target);
  if (!target.endsWith(".exe")) {
    fs.chmodSync(target, 0o755);
  }
  console.log(`Staged ${target}`);
}

const verifier = path.join(repoRoot, "scripts", "verify-platform-package.mjs");
const verification = spawnSync(process.execPath, [verifier], {
  cwd: packageDir,
  stdio: "inherit",
});

if (verification.status !== 0) {
  fail(`staged package verification failed for ${entry.package}`);
}
