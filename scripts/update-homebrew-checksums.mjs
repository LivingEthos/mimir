#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const cwd = process.cwd();
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");

const targets = [
  { rustTarget: "aarch64-apple-darwin", placeholder: "PLACEHOLDER_SHA256_AARCH64_APPLE" },
  { rustTarget: "x86_64-apple-darwin", placeholder: "PLACEHOLDER_SHA256_X86_64_APPLE" },
  { rustTarget: "aarch64-unknown-linux-gnu", placeholder: "PLACEHOLDER_SHA256_AARCH64_LINUX" },
  { rustTarget: "x86_64-unknown-linux-gnu", placeholder: "PLACEHOLDER_SHA256_X86_64_LINUX" },
];

function fail(message) {
  console.error(`Mimir Homebrew checksum update failed: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const args = {
    artifactsDir: path.join(repoRoot, "target", "distrib"),
    check: false,
    allowMissing: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--artifacts-dir") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        fail("--artifacts-dir requires a value");
      }
      args.artifactsDir = path.resolve(cwd, value);
      index += 1;
    } else if (arg === "--check") {
      args.check = true;
    } else if (arg === "--allow-missing") {
      args.allowMissing = true;
    } else if (arg === "--help") {
      console.log(`Usage:
  node scripts/update-homebrew-checksums.mjs [--artifacts-dir target/distrib]
  node scripts/update-homebrew-checksums.mjs --check [--artifacts-dir target/distrib]

Reads cargo-dist macOS/Linux archives and updates HomebrewFormula/mimir.rb sha256 values.
Use --check in CI/release review to verify the formula already matches the archives.
Use --allow-missing only for local partial staging; never before tagging.`);
      process.exit(0);
    } else {
      fail(`unknown argument ${arg}`);
    }
  }

  return args;
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function archivePath(artifactsDir, rustTarget) {
  return path.join(artifactsDir, `mimir-cli-${rustTarget}.tar.xz`);
}

function formulaUrl(version, rustTarget) {
  return `https://github.com/MisterWonderful/mimir/releases/download/v${version}/mimir-cli-${rustTarget}.tar.xz`;
}

function formulaVersion(formula) {
  const explicitMatch = /\bversion "([^"]+)"/.exec(formula);
  if (explicitMatch) {
    return explicitMatch[1];
  }

  const versions = new Set(
    [...formula.matchAll(/github\.com\/MisterWonderful\/mimir\/releases\/download\/v([^/]+)\/mimir-cli-/g)].map(
      (match) => match[1],
    ),
  );
  if (versions.size === 0) {
    fail("Homebrew formula is missing Mimir release URLs");
  }
  if (versions.size > 1) {
    fail(`Homebrew formula has mixed release versions: ${[...versions].sort().join(", ")}`);
  }
  return [...versions][0];
}

function replaceChecksumAfterUrl(formula, url, sha256) {
  const urlNeedle = `url "${url}"`;
  const urlIndex = formula.indexOf(urlNeedle);
  if (urlIndex === -1) {
    fail(`Homebrew formula is missing URL: ${url}`);
  }

  const before = formula.slice(0, urlIndex);
  const after = formula.slice(urlIndex);
  const shaMatch = /sha256 "[^"]+"/.exec(after);
  if (!shaMatch) {
    fail(`Homebrew formula is missing sha256 after URL: ${url}`);
  }
  return before + after.replace(/sha256 "[^"]+"/, `sha256 "${sha256}"`);
}

const args = parseArgs(process.argv.slice(2));
const formulaPath = path.join(repoRoot, "HomebrewFormula", "mimir.rb");
if (!fs.existsSync(formulaPath)) {
  fail(`missing Homebrew formula: ${formulaPath}`);
}

let formula = fs.readFileSync(formulaPath, "utf8");
const originalFormula = formula;
const version = formulaVersion(formula);
let checked = 0;
let missing = 0;

for (const target of targets) {
  const archive = archivePath(args.artifactsDir, target.rustTarget);
  if (!fs.existsSync(archive)) {
    if (args.allowMissing) {
      missing += 1;
      console.warn(`Skipping missing archive for ${target.rustTarget}: ${archive}`);
      continue;
    }
    fail(`missing cargo-dist archive for ${target.rustTarget}: ${archive}`);
  }

  const sha = sha256File(archive);
  formula = replaceChecksumAfterUrl(formula, formulaUrl(version, target.rustTarget), sha);
  checked += 1;
}

if (args.check) {
  if (formula !== originalFormula) {
    fail("Homebrew formula checksums do not match cargo-dist archives");
  }
  console.log(`Verified ${checked} Homebrew checksum(s) against ${args.artifactsDir}`);
} else if (formula !== originalFormula) {
  fs.writeFileSync(formulaPath, formula);
  console.log(`Updated ${checked} Homebrew checksum(s) in ${path.relative(repoRoot, formulaPath)}`);
} else {
  console.log(`Homebrew checksums already match ${checked} archive(s)`);
}

if (missing > 0) {
  console.warn(`Skipped ${missing} missing archive(s); rerun without --allow-missing before tagging.`);
}
