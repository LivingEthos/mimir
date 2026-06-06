#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import crypto from "node:crypto";

const cwd = process.cwd();
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const rawArgs = process.argv.slice(2);

const platformPackages = [
  {
    key: "darwin-arm64",
    rustTarget: "aarch64-apple-darwin",
    packageName: "@mimir/cli-darwin-arm64",
    packageDir: "packages/cli-darwin-arm64",
    os: "darwin",
    cpu: "arm64",
    binary: "mimir",
    files: ["bin/mimir", "mimir", "package.json"],
    homebrewPlaceholder: "PLACEHOLDER_SHA256_AARCH64_APPLE",
  },
  {
    key: "darwin-x64",
    rustTarget: "x86_64-apple-darwin",
    packageName: "@mimir/cli-darwin-x64",
    packageDir: "packages/cli-darwin-x64",
    os: "darwin",
    cpu: "x64",
    binary: "mimir",
    files: ["bin/mimir", "mimir", "package.json"],
    homebrewPlaceholder: "PLACEHOLDER_SHA256_X86_64_APPLE",
  },
  {
    key: "linux-arm64",
    rustTarget: "aarch64-unknown-linux-gnu",
    packageName: "@mimir/cli-linux-arm64",
    packageDir: "packages/cli-linux-arm64",
    os: "linux",
    cpu: "arm64",
    binary: "mimir",
    files: ["bin/mimir", "mimir", "package.json"],
    homebrewPlaceholder: "PLACEHOLDER_SHA256_AARCH64_LINUX",
  },
  {
    key: "linux-x64",
    rustTarget: "x86_64-unknown-linux-gnu",
    packageName: "@mimir/cli-linux-x64",
    packageDir: "packages/cli-linux-x64",
    os: "linux",
    cpu: "x64",
    binary: "mimir",
    files: ["bin/mimir", "mimir", "package.json"],
    homebrewPlaceholder: "PLACEHOLDER_SHA256_X86_64_LINUX",
  },
  {
    key: "win32-x64",
    rustTarget: "x86_64-pc-windows-msvc",
    packageName: "@mimir/cli-win32-x64",
    packageDir: "packages/cli-win32-x64",
    os: "win32",
    cpu: "x64",
    binary: "mimir.exe",
    files: ["bin/mimir.exe", "mimir.exe", "package.json"],
  },
];

function fail(message) {
  console.error(`Mimir platform package verification failed: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const flags = new Set();
  const values = new Map();
  const valueFlags = new Set(["--homebrew-artifacts-dir"]);
  const booleanFlags = new Set([
    "--all",
    "--require-homebrew-sha256",
    "--require-platform-binaries",
    "--help",
  ]);

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (valueFlags.has(arg)) {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        fail(`${arg} requires a value`);
      }
      values.set(arg, value);
      index += 1;
    } else if (booleanFlags.has(arg)) {
      flags.add(arg);
    } else {
      fail(`unknown argument ${arg}`);
    }
  }

  return { flags, values };
}

const parsedArgs = parseArgs(rawArgs);
const requireHomebrewSha256 = parsedArgs.flags.has("--require-homebrew-sha256");
const requirePlatformBinaries = parsedArgs.flags.has("--require-platform-binaries");
const homebrewArtifactsDir = parsedArgs.values.has("--homebrew-artifacts-dir")
  ? path.resolve(cwd, parsedArgs.values.get("--homebrew-artifacts-dir"))
  : null;

function usage() {
  console.log(`Usage:
  node scripts/verify-platform-package.mjs
  node scripts/verify-platform-package.mjs --all [--require-homebrew-sha256]
  node scripts/verify-platform-package.mjs --all --require-platform-binaries
  node scripts/verify-platform-package.mjs --all --require-homebrew-sha256 --homebrew-artifacts-dir target/distrib

Run without flags from a platform package directory to verify its staged native binary.
Run --all from anywhere in the repo to verify release metadata across Node packages and Homebrew.
Pass --require-platform-binaries before packaging to verify every platform package is staged.
Pass --homebrew-artifacts-dir to compare Homebrew checksums with cargo-dist archive files.`);
}

function readJson(file) {
  if (!fs.existsSync(file)) {
    fail(`missing JSON file: ${file}`);
  }
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`invalid JSON in ${file}: ${error.message}`);
  }
}

function assertEqual(label, actual, expected) {
  if (actual !== expected) {
    fail(`${label} expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertArrayEqual(label, actual, expected) {
  if (!Array.isArray(actual)) {
    fail(`${label} expected array ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
  if (actual.length !== expected.length || actual.some((value, index) => value !== expected[index])) {
    fail(`${label} expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function packageJsonPath(packageDir) {
  return path.join(packageDir, "package.json");
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function homebrewArchivePath(entry) {
  return path.join(homebrewArtifactsDir, `mimir-cli-${entry.rustTarget}.tar.xz`);
}

function verifyPlatformMetadata(packageDir, entry, rootVersion) {
  const pkg = readJson(packageJsonPath(packageDir));
  assertEqual(`${entry.packageName} name`, pkg.name, entry.packageName);
  assertEqual(`${entry.packageName} version`, pkg.version, rootVersion);
  assertEqual(`${entry.packageName} private`, pkg.private, true);
  assertEqual(`${entry.packageName} license`, pkg.license, "Apache-2.0");
  assertArrayEqual(`${entry.packageName} os`, pkg.os, [entry.os]);
  assertArrayEqual(`${entry.packageName} cpu`, pkg.cpu, [entry.cpu]);
  assertArrayEqual(`${entry.packageName} files`, pkg.files, entry.files);
  assertEqual(
    `${entry.packageName} prepack`,
    pkg.scripts?.prepack,
    "node ../../scripts/verify-platform-package.mjs",
  );
  if (pkg.publishConfig) {
    fail(`${entry.packageName} must not declare publishConfig; npm publication is disabled`);
  }
}

function verifyPlatformBinary(packageDir, entry) {
  const candidates = [path.join(packageDir, entry.binary), path.join(packageDir, "bin", entry.binary)];
  const binary = candidates.find((candidate) => fs.existsSync(candidate));
  if (!binary) {
    fail(`missing native binary for ${entry.packageName}; expected one of ${candidates.join(", ")}`);
  }

  const stat = fs.statSync(binary);
  if (!stat.isFile()) {
    fail(`${binary} is not a regular file`);
  }
  if (entry.os !== "win32" && (stat.mode & 0o111) === 0) {
    fail(`${binary} is not executable`);
  }

  return binary;
}

function verifyRootCliPackage(rootVersion) {
  const pkg = readJson(path.join(repoRoot, "packages", "cli", "package.json"));
  assertEqual("@mimir/cli name", pkg.name, "@mimir/cli");
  assertEqual("@mimir/cli version", pkg.version, rootVersion);
  assertEqual("@mimir/cli private", pkg.private, true);
  assertEqual("@mimir/cli bin.mimir", pkg.bin?.mimir, "bin/mimir");
  assertEqual(
    "@mimir/cli prepack",
    pkg.scripts?.prepack,
    "node --check ./bin/mimir && node --check ./install.js && node ../../scripts/verify-platform-package.mjs --all --require-platform-binaries",
  );
  assertEqual("@mimir/cli postinstall", pkg.scripts?.postinstall, "node ./install.js");
  if (pkg.publishConfig) {
    fail("@mimir/cli must not declare publishConfig; npm publication is disabled");
  }

  const expectedOptionalDeps = new Map(
    platformPackages.map((entry) => [entry.packageName, rootVersion]),
  );
  const actualOptionalDeps = pkg.optionalDependencies || {};
  for (const [name, version] of expectedOptionalDeps) {
    assertEqual(`@mimir/cli optional dependency ${name}`, actualOptionalDeps[name], version);
  }
  for (const name of Object.keys(actualOptionalDeps)) {
    if (!expectedOptionalDeps.has(name)) {
      fail(`@mimir/cli has unexpected optional dependency ${name}`);
    }
  }
}

function verifyHomebrewFormula(rootVersion) {
  const formulaPath = path.join(repoRoot, "HomebrewFormula", "mimir.rb");
  if (!fs.existsSync(formulaPath)) {
    fail(`missing Homebrew formula: ${formulaPath}`);
  }
  const formula = fs.readFileSync(formulaPath, "utf8");
  if (/\bversion\s+"/.test(formula)) {
    fail("Homebrew formula must not declare a redundant version; Homebrew infers it from release URLs");
  }

  let placeholders = 0;
  for (const entry of platformPackages.filter((candidate) => candidate.homebrewPlaceholder)) {
    const expectedUrl = `https://github.com/LivingEthos/mimir/releases/download/v${rootVersion}/mimir-cli-${entry.rustTarget}.tar.xz`;
    const urlNeedle = `url "${expectedUrl}"`;
    const urlIndex = formula.indexOf(urlNeedle);
    if (urlIndex === -1) {
      fail(`Homebrew formula missing URL for ${entry.rustTarget}: ${expectedUrl}`);
    }

    const shaMatch = /sha256 "([^"]+)"/.exec(formula.slice(urlIndex));
    if (!shaMatch) {
      fail(`Homebrew formula missing sha256 after URL for ${entry.rustTarget}`);
    }
    const sha = shaMatch[1];
    const isSha256 = /^[a-f0-9]{64}$/i.test(sha);
    const isPlaceholder = sha === entry.homebrewPlaceholder;
    if (requireHomebrewSha256 && !isSha256) {
      fail(`Homebrew checksum for ${entry.rustTarget} must be a real 64-character sha256`);
    }
    if (!isSha256 && !isPlaceholder) {
      fail(`Homebrew checksum for ${entry.rustTarget} is neither the expected placeholder nor a sha256`);
    }
    if (homebrewArtifactsDir) {
      const archivePath = homebrewArchivePath(entry);
      if (!fs.existsSync(archivePath)) {
        fail(`missing cargo-dist archive for ${entry.rustTarget}: ${archivePath}`);
      }
      const expectedSha = sha256File(archivePath);
      if (sha !== expectedSha) {
        fail(
          `Homebrew checksum for ${entry.rustTarget} does not match ${archivePath}: expected ${expectedSha}, got ${sha}`,
        );
      }
    }
    if (isPlaceholder) {
      placeholders += 1;
    }
  }

  return placeholders;
}

function rootVersion() {
  return readJson(path.join(repoRoot, "packages", "cli", "package.json")).version;
}

function verifyAllMetadata() {
  const version = rootVersion();
  verifyRootCliPackage(version);
  for (const entry of platformPackages) {
    const packageDir = path.join(repoRoot, entry.packageDir);
    verifyPlatformMetadata(packageDir, entry, version);
    if (requirePlatformBinaries) {
      verifyPlatformBinary(packageDir, entry);
    }
  }
  const homebrewPlaceholders = verifyHomebrewFormula(version);
  if (homebrewPlaceholders > 0 && !requireHomebrewSha256) {
    console.warn(
      `Homebrew formula still has ${homebrewPlaceholders} placeholder checksum(s); rerun with --require-homebrew-sha256 before tagging.`,
    );
  }
  console.log(
    `Verified release metadata for @mimir/cli ${version}, ${platformPackages.length} platform packages, and Homebrew URLs${homebrewArtifactsDir ? ", checksums" : ""}${requirePlatformBinaries ? ", platform binaries" : ""}.`,
  );
}

function verifyCurrentPlatformPackage() {
  const pkgPath = packageJsonPath(cwd);
  const pkg = readJson(pkgPath);
  const entry = platformPackages.find((candidate) => candidate.packageName === pkg.name);
  if (!entry) {
    fail(`unexpected package name ${pkg.name || "<missing>"}`);
  }
  verifyPlatformMetadata(cwd, entry, rootVersion());

  const binary = verifyPlatformBinary(cwd, entry);

  console.log(`Verified ${pkg.name}: ${path.relative(cwd, binary)}`);
}

if (parsedArgs.flags.has("--help")) {
  usage();
  process.exit(0);
}

if (
  !parsedArgs.flags.has("--all") &&
  (requireHomebrewSha256 || homebrewArtifactsDir || requirePlatformBinaries)
) {
  fail("release-wide verification flags require --all");
}

if (parsedArgs.flags.has("--all")) {
  verifyAllMetadata();
} else {
  verifyCurrentPlatformPackage();
}
