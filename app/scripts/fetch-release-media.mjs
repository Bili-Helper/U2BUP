#!/usr/bin/env node
/**
 * Download reviewed FFmpeg archives for a Rust target, verify SHA-256, stage into desktop/resources.
 * Usage: node scripts/fetch-release-media.mjs --target <triple>
 */
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { chmod, copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const { values } = parseArgs({
  options: {
    target: { type: 'string' },
    pin: { type: 'string' },
    workdir: { type: 'string' },
  },
});

if (!values.target) throw new Error('Required: --target <Rust target triple>');

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const pinPath = path.resolve(values.pin ?? path.join(appRoot, 'release', 'media', `${values.target}.json`));
const pin = JSON.parse(await readFile(pinPath, 'utf8'));
if (pin.target !== values.target) throw new Error(`Pin target ${pin.target} != --target ${values.target}`);
if (pin.status !== 'ready') {
  throw new Error(`Media pin for ${values.target} is "${pin.status}". Fill reviewed URLs/hashes in ${pinPath} before a stable release.`);
}

const workRoot = path.resolve(values.workdir ?? path.join(appRoot, '.local', 'release-media', values.target));
await rm(workRoot, { recursive: true, force: true });
await mkdir(workRoot, { recursive: true });
const extractRoot = path.join(workRoot, 'extract');
await mkdir(extractRoot, { recursive: true });

function sha256Buffer(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

async function download(url, destination) {
  const response = await fetch(url, { redirect: 'follow' });
  if (!response.ok) throw new Error(`Download failed ${response.status} ${url}`);
  await pipeline(response.body, createWriteStream(destination));
}

function findSevenZip() {
  const candidates = [
    process.env.SEVEN_ZIP,
    '7z',
    '7zz',
    path.join(process.env['ProgramFiles'] ?? 'C:\\Program Files', '7-Zip', '7z.exe'),
    path.join(process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', '7-Zip', '7z.exe'),
  ].filter(Boolean);
  for (const bin of candidates) {
    const probe = spawnSync(bin, ['--help'], { encoding: 'utf8', windowsHide: true });
    if (probe.error) continue;
    if (probe.status === 0 || probe.status === 1 || /7-Zip|7zz/i.test(`${probe.stdout}\n${probe.stderr}`)) return bin;
  }
  return null;
}

function extractArchive(archivePath, format, destination) {
  const fmt = format ?? inferFormat(archivePath);
  if (fmt === 'zip') {
    if (process.platform === 'win32') {
      execFileSync('powershell.exe', [
        '-NoProfile', '-Command',
        `Expand-Archive -LiteralPath '${archivePath.replace(/'/g, "''")}' -DestinationPath '${destination.replace(/'/g, "''")}' -Force`,
      ], { stdio: 'inherit', windowsHide: true });
      return;
    }
    execFileSync('unzip', ['-o', archivePath, '-d', destination], { stdio: 'inherit' });
    return;
  }
  if (fmt === 'tar.gz' || fmt === 'tgz') {
    execFileSync('tar', ['-xzf', archivePath, '-C', destination], { stdio: 'inherit' });
    return;
  }
  if (fmt === 'tar.xz') {
    const tarTry = spawnSync('tar', ['-xJf', archivePath, '-C', destination], { encoding: 'utf8' });
    if (tarTry.status === 0) return;
    const seven = findSevenZip();
    if (!seven) throw new Error(`Cannot extract tar.xz (tar failed: ${tarTry.stderr || tarTry.error}; install xz-utils or 7-Zip)`);
    execFileSync(seven, ['x', archivePath, `-o${destination}`, '-y'], { stdio: 'inherit', windowsHide: true });
    const tarFile = path.basename(archivePath).replace(/\.xz$/i, '');
    const tarPath = path.join(destination, tarFile);
    execFileSync(seven, ['x', tarPath, `-o${destination}`, '-y'], { stdio: 'inherit', windowsHide: true });
    return;
  }
  if (fmt === '7z') {
    const seven = findSevenZip();
    if (!seven) throw new Error('7-Zip CLI (7z) is required to extract .7z media archives');
    execFileSync(seven, ['x', archivePath, `-o${destination}`, '-y'], { stdio: 'inherit', windowsHide: true });
    return;
  }
  throw new Error(`Unsupported archive format: ${fmt}`);
}

function inferFormat(filePath) {
  const lower = filePath.toLowerCase();
  if (lower.endsWith('.tar.xz')) return 'tar.xz';
  if (lower.endsWith('.tar.gz') || lower.endsWith('.tgz')) return 'tar.gz';
  if (lower.endsWith('.zip')) return 'zip';
  if (lower.endsWith('.7z')) return '7z';
  throw new Error(`Cannot infer archive format for ${filePath}`);
}

for (const [index, archive] of (pin.archives ?? []).entries()) {
  if (!archive?.url || !/^[a-f0-9]{64}$/i.test(archive.sha256 ?? '')) {
    throw new Error(`Archive #${index} needs url and sha256`);
  }
  const ext = path.extname(new URL(archive.url).pathname) || `.${archive.format ?? 'bin'}`;
  const name = `archive-${index}${archive.format === 'tar.xz' ? '.tar.xz' : ext}`;
  const archivePath = path.join(workRoot, name);
  console.log(`Downloading ${archive.url}`);
  await download(archive.url, archivePath);
  const digest = sha256Buffer(await readFile(archivePath));
  if (digest !== archive.sha256.toLowerCase()) {
    throw new Error(`Archive hash mismatch for ${archive.url}: got ${digest}, expected ${archive.sha256.toLowerCase()}`);
  }
  extractArchive(archivePath, archive.format, extractRoot);
}

const stageSource = path.join(workRoot, 'stage-source');
await mkdir(stageSource, { recursive: true });
const suffix = values.target.includes('windows') ? '.exe' : '';

for (const name of ['ffmpeg', 'ffprobe']) {
  const meta = pin.binaries?.[name];
  if (!meta?.path || !/^[a-f0-9]{64}$/i.test(meta.sha256 ?? '')) {
    throw new Error(`Pin binaries.${name} needs path and sha256`);
  }
  const sourceBinary = path.join(extractRoot, meta.path);
  await stat(sourceBinary);
  const digest = sha256Buffer(await readFile(sourceBinary));
  if (digest !== meta.sha256.toLowerCase()) {
    throw new Error(`Binary hash mismatch for ${name}: got ${digest}, expected ${meta.sha256.toLowerCase()}`);
  }
  const destName = `${name}${suffix}`;
  await copyFile(sourceBinary, path.join(stageSource, destName));
  if (!suffix) await chmod(path.join(stageSource, destName), 0o755);
}

async function findLicense(root, basename) {
  const stack = [root];
  while (stack.length) {
    const dir = stack.pop();
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== '__MACOSX') stack.push(full);
      } else if (entry.name === basename) {
        return full;
      }
    }
  }
  return null;
}

const licenseBundleDir = path.join(appRoot, 'release', 'media', 'licenses');
for (const license of pin.licenses ?? []) {
  if (path.basename(license) !== license) throw new Error('License names must be basenames');
  let sourceLicense = await findLicense(extractRoot, license);
  if (!sourceLicense) {
    try {
      await stat(path.join(licenseBundleDir, license));
      sourceLicense = path.join(licenseBundleDir, license);
    } catch { /* missing */ }
  }
  if (!sourceLicense) throw new Error(`License file not found for ${license}`);
  await copyFile(sourceLicense, path.join(stageSource, license));
}

const stageManifest = {
  target: values.target,
  sha256: {
    ffmpeg: pin.binaries.ffmpeg.sha256.toLowerCase(),
    ffprobe: pin.binaries.ffprobe.sha256.toLowerCase(),
  },
  licenses: pin.licenses,
};

const manifestPath = path.join(workRoot, 'stage-manifest.json');
await writeFile(manifestPath, JSON.stringify(stageManifest, null, 2));

const stageScript = path.join(appRoot, 'scripts', 'stage-media.mjs');
execFileSync(process.execPath, [
  stageScript,
  '--source', stageSource,
  '--target', values.target,
  '--manifest', manifestPath,
], { stdio: 'inherit', cwd: appRoot });

console.log(`Staged release media for ${values.target}`);
