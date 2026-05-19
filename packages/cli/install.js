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

const platformKey = `${platformMap[platform]}-${archMap[arch]}`;
const binaryName = platform === 'win32' ? 'mimir.exe' : 'mimir';

// Try to find the platform-specific package
const optionalDep = `@mimir/cli-${platformKey}`;
const depPath = path.join(__dirname, 'node_modules', optionalDep);

if (fs.existsSync(depPath)) {
  const binPath = path.join(depPath, binaryName);
  if (fs.existsSync(binPath)) {
    const targetPath = path.join(__dirname, 'bin', binaryName);
    fs.mkdirSync(path.dirname(targetPath), { recursive: true });
    fs.copyFileSync(binPath, targetPath);
    fs.chmodSync(targetPath, 0o755);
    console.log(`Mimir CLI installed: ${targetPath}`);
    process.exit(0);
  }
}

console.warn(`Platform binary not found for ${platformKey}. Building from source...`);
console.warn('Install Rust and run: cargo install mimir-cli');
process.exit(0);
