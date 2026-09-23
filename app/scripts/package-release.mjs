#!/usr/bin/env node
/**
 * Assemble preview (A) or stable (B) release artifact folders/archives.
 *
 * Preview: node scripts/package-release.mjs --mode preview --tag v0.7.0-preview.1 --target <triple> --bin-dir <dir>
 * Stable Windows portable: ... --mode stable-windows --tag v0.7.0 --bin-dir <dir>
 * Stable collect Tauri: ... --mode stable-bundle --tag v0.7.0 --bundle-dir <tauri bundle dir> --os macos|linux --target <triple>
 */
import { copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const { values } = parseArgs({
  options: {
    mode: { type: 'string' },
    tag: { type: 'string' },
    target: { type: 'string' },
    'bin-dir': { type: 'string' },
    'bundle-dir': { type: 'string' },
    os: { type: 'string' },
    out: { type: 'string' },
  },
});

const mode = values.mode;
const tag = values.tag;
if (!mode || !tag) throw new Error('Required: --mode <preview|stable-windows|stable-bundle> --tag <tag>');

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outRoot = path.resolve(values.out ?? path.join(appRoot, 'dist', 'release', tag));
await mkdir(outRoot, { recursive: true });

const isWindows = (values.target ?? '').includes('windows') || process.platform === 'win32';
const exe = (name) => (isWindows || (values.target ?? '').includes('windows') ? `${name}.exe` : name);

async function sha256File(filePath) {
  const hash = createHash('sha256');
  hash.update(await readFile(filePath));
  return hash.digest('hex');
}

async function writeChecksums(files, destination) {
  const lines = [];
  for (const file of files) {
    lines.push(`${await sha256File(file)}  ${path.basename(file)}`);
  }
  await writeFile(destination, `${lines.join('\n')}\n`, 'utf8');
}

function zipDirectory(sourceDir, zipPath) {
  try { rmSyncSafe(zipPath); } catch { /* ignore */ }
  if (process.platform === 'win32') {
    execFileSync('powershell.exe', [
      '-NoProfile', '-Command',
      `Compress-Archive -Path (Join-Path '${sourceDir.replace(/'/g, "''")}' '*') -DestinationPath '${zipPath.replace(/'/g, "''")}' -Force`,
    ], { stdio: 'inherit', windowsHide: true });
    return;
  }
  const zip = spawnSync('zip', ['-r', zipPath, '.'], { cwd: sourceDir, encoding: 'utf8' });
  if (zip.status === 0) return;
  execFileSync('tar', ['-a', '-cf', zipPath, '-C', sourceDir, '.'], { stdio: 'inherit' });
}

function rmSyncSafe(filePath) {
  spawnSync(process.platform === 'win32' ? 'cmd' : 'rm', process.platform === 'win32' ? ['/c', 'del', '/f', '/q', filePath] : ['-f', filePath], { stdio: 'ignore', windowsHide: true });
}

async function copyIfExists(src, dest) {
  try {
    await stat(src);
    await copyFile(src, dest);
    return true;
  } catch {
    return false;
  }
}

if (mode === 'preview') {
  if (!values.target || !values['bin-dir']) throw new Error('preview requires --target and --bin-dir');
  const binDir = path.resolve(values['bin-dir']);
  const folderName = `U2BUP-${tag}-${values.target}-unbundled`;
  const folder = path.join(outRoot, folderName);
  await rm(folder, { recursive: true, force: true });
  await mkdir(folder, { recursive: true });

  const desktopName = exe('u2bup-desktop');
  const serverName = exe('u2bup-server');
  if (!(await copyIfExists(path.join(binDir, desktopName), path.join(folder, desktopName)))) {
    throw new Error(`Missing ${desktopName} in ${binDir}`);
  }
  if (!(await copyIfExists(path.join(binDir, serverName), path.join(folder, serverName)))) {
    throw new Error(`Missing ${serverName} in ${binDir}`);
  }
  await writeFile(path.join(folder, 'UNBUNDLED.txt'), [
    `U2BUP ${tag} preview (package A)`,
    '',
    'This archive is unbundled: FFmpeg/FFprobe are NOT included.',
    'Install compatible ffmpeg and ffprobe on PATH, or use a stable release portable/installer package.',
    `Target: ${values.target}`,
    '',
  ].join('\n'), 'utf8');
  await copyIfExists(path.join(appRoot, 'README.md'), path.join(folder, 'README.md'));
  await copyIfExists(path.join(appRoot, 'THIRD-PARTY-NOTICES.md'), path.join(folder, 'THIRD-PARTY-NOTICES.md'));

  const zipPath = path.join(outRoot, `${folderName}.zip`);
  zipDirectory(folder, zipPath);
  await writeChecksums([zipPath], path.join(outRoot, `${folderName}.sha256`));
  console.log(`Preview artifact: ${zipPath}`);
  process.exit(0);
}

if (mode === 'stable-windows') {
  if (!values['bin-dir']) throw new Error('stable-windows requires --bin-dir');
  const binDir = path.resolve(values['bin-dir']);
  const folderName = `U2BUP-${tag}-windows-x64`;
  const folder = path.join(outRoot, folderName);
  await rm(folder, { recursive: true, force: true });
  await mkdir(folder, { recursive: true });

  if (!(await copyIfExists(path.join(binDir, 'u2bup-desktop.exe'), path.join(folder, 'U2BUP.exe')))) {
    throw new Error('Missing u2bup-desktop.exe');
  }
  if (!(await copyIfExists(path.join(binDir, 'u2bup-server.exe'), path.join(folder, 'u2bup-server.exe')))) {
    throw new Error('Missing u2bup-server.exe');
  }
  const resources = path.join(folder, 'resources');
  await mkdir(resources, { recursive: true });
  const mediaDir = path.join(appRoot, 'desktop', 'resources');
  for (const entry of await readdir(mediaDir)) {
    const full = path.join(mediaDir, entry);
    if ((await stat(full)).isFile()) await copyFile(full, path.join(resources, entry));
  }
  await copyIfExists(path.join(appRoot, 'README.md'), path.join(folder, 'README.md'));
  await copyIfExists(path.join(appRoot, 'THIRD-PARTY-NOTICES.md'), path.join(folder, 'THIRD-PARTY-NOTICES.md'));
  await copyIfExists(path.join(appRoot, 'scripts', 'Start-PortableWeb.ps1'), path.join(folder, 'Start-Web.ps1'));

  const zipPath = path.join(outRoot, `${folderName}.zip`);
  zipDirectory(folder, zipPath);
  await writeChecksums([zipPath], path.join(outRoot, `${folderName}.sha256`));
  console.log(`Stable Windows portable: ${zipPath}`);
  process.exit(0);
}

if (mode === 'stable-bundle') {
  if (!values['bundle-dir'] || !values.os || !values.target) {
    throw new Error('stable-bundle requires --bundle-dir, --os macos|linux, and --target');
  }
  const bundleDir = path.resolve(values['bundle-dir']);
  const collected = [];
  async function walk(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) await walk(full);
      else if (/\.(dmg|AppImage|deb)$/i.test(entry.name)) collected.push(full);
    }
  }
  await walk(bundleDir);
  if (!collected.length) throw new Error(`No dmg/AppImage/deb under ${bundleDir}`);
  for (const file of collected) {
    const destName = `U2BUP-${tag}-${values.target}-${path.basename(file)}`;
    const dest = path.join(outRoot, destName);
    await copyFile(file, dest);
    console.log(`Collected ${dest}`);
  }
  const checksumName = `U2BUP-${tag}-${values.target}.sha256`;
  await writeChecksums(
    (await readdir(outRoot))
      .filter((n) => /\.(dmg|AppImage|deb)$/i.test(n) && n.includes(values.target))
      .map((n) => path.join(outRoot, n)),
    path.join(outRoot, checksumName),
  );
  process.exit(0);
}

throw new Error(`Unknown mode: ${mode}`);
