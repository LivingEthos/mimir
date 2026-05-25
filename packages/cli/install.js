#!/usr/bin/env node
// Platform-specific binary installer for @mimir/cli

const os = require('os');
const fs = require('fs');
const path = require('path');

const platform = os.platform();
const arch = os.arch();

const platformMap = {
  darwin: 'darwin',
  linux: 'linux',
  win32: 'win32',
};

const archMap = {
  arm64: 'arm64',
  x64: 'x64',
};

const mappedPlatform = platformMap[platform];
const mappedArch = archMap[arch];

function fail(message) {
  console.error(`Mimir CLI install failed: ${message}`);
  console.error(
    'The @mimir/cli package requires a matching @mimir/cli-<platform>-<arch> optional dependency.',
  );
  process.exit(1);
}

if (!mappedPlatform || !mappedArch) {
  fail(`unsupported platform ${platform}-${arch}`);
}

const platformKey = `${mappedPlatform}-${mappedArch}`;
const binaryName = platform === 'win32' ? 'mimir.exe' : 'mimir';
const targetName =
  platform === 'win32' ? `mimir-${platformKey}.exe` : `mimir-${platformKey}`;

// Try to find the platform-specific package
const optionalDep = `@mimir/cli-${platformKey}`;
const depCandidates = [
  path.join(__dirname, 'node_modules', optionalDep),
  path.join(path.dirname(__dirname), `cli-${platformKey}`),
  path.join(path.dirname(path.dirname(__dirname)), optionalDep),
];
let depPath;

try {
  depPath = path.dirname(
    require.resolve(`${optionalDep}/package.json`, { paths: [__dirname] }),
  );
} catch (_) {
  depPath = depCandidates.find((candidate) => fs.existsSync(candidate));
}

if (!depPath) {
  fail(`platform package ${optionalDep} was not found`);
}

const sourceCandidates = [
  path.join(depPath, binaryName),
  path.join(depPath, 'bin', binaryName),
];
const sourcePath = sourceCandidates.find((candidate) => fs.existsSync(candidate));

if (!sourcePath) {
  fail(`platform package ${optionalDep} does not contain ${binaryName}`);
}

const targetPath = path.join(__dirname, 'bin', targetName);
fs.mkdirSync(path.dirname(targetPath), { recursive: true });
fs.copyFileSync(sourcePath, targetPath);
fs.chmodSync(targetPath, 0o755);
console.log(`Mimir CLI installed: ${targetPath}`);
process.exit(0);
