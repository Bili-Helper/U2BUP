import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmod, copyFile, mkdir, readFile, readdir, realpath, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

// No downloads and no architecture guesses from archive names. Release inputs must
// have a reviewed SHA-256 manifest; --development explicitly skips this requirement.
const { values } = parseArgs({ options: {
  source: { type: 'string' }, target: { type: 'string' }, manifest: { type: 'string' },
  development: { type: 'boolean', default: false }, destination: { type: 'string' },
} });
if (!values.source || !values.target) throw new Error('Required: --source <FFmpeg folder> --target <Rust target triple> [--manifest <reviewed JSON> | --development]');
const supported = {
  'x86_64-pc-windows-msvc': ['win32', 'x64'],
  'aarch64-apple-darwin': ['darwin', 'arm64'],
  'x86_64-apple-darwin': ['darwin', 'x64'],
  'x86_64-unknown-linux-gnu': ['linux', 'x64'],
  'aarch64-unknown-linux-gnu': ['linux', 'arm64'],
};
const host = supported[values.target];
if (!host || host[0] !== process.platform || host[1] !== process.arch) throw new Error('Media tools must be staged and executed on their matching OS/architecture.');
if (!values.manifest && !values.development) throw new Error('Release staging requires --manifest with reviewed binary SHA-256 hashes and license filenames.');
const expected = values.manifest ? JSON.parse(await readFile(values.manifest, 'utf8')) : null;
if (expected && expected.target !== values.target) throw new Error('Manifest target does not match --target.');
const source = await realpath(values.source);
const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const destination = path.resolve(values.destination ?? path.join(app, 'desktop', 'resources'));
const suffix = process.platform === 'win32' ? '.exe' : '';
const components = [];
for (const name of ['ffmpeg', 'ffprobe']) {
  const filename = `${name}${suffix}`;
  let binary = path.join(source, filename);
  try { await stat(binary); } catch { binary = path.join(source, 'bin', filename); }
  const bytes = await readFile(binary);
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  if (expected && (!/^[a-f0-9]{64}$/i.test(expected.sha256?.[name] ?? '') || sha256 !== expected.sha256[name].toLowerCase())) {
    throw new Error(`Reviewed hash missing or mismatched for ${name}`);
  }
  const version = execFileSync(binary, ['-version'], { encoding: 'utf8', windowsHide: true }).split(/\r?\n/)[0];
  components.push({ name: filename, binary, version, sha256, bytes: bytes.length });
}
const licenses = expected?.licenses ?? (await readdir(source)).filter(name => /^(LICENSE|COPYING)([.-]|$)/i.test(name));
if (!values.development && (!Array.isArray(licenses) || !licenses.length)) throw new Error('Release manifest must list the bundled FFmpeg license files.');
for (const filename of licenses) {
  if (path.basename(filename) !== filename) throw new Error('License filenames must be relative basenames.');
  await stat(path.join(source, filename));
}
// Validate all inputs before replacing any existing staged component.
await mkdir(destination, { recursive: true });
for (const { name, binary } of components) {
  await copyFile(binary, path.join(destination, name));
  if (process.platform !== 'win32') await chmod(path.join(destination, name), 0o755);
}
for (const filename of licenses) await copyFile(path.join(source, filename), path.join(destination, `FFmpeg-${filename}`));
await writeFile(path.join(destination, 'media-manifest.json'), JSON.stringify({
  target: values.target, stagedAt: new Date().toISOString(),
  mode: values.development ? 'development' : 'hash-verified',
  redistributionReady: false,
  note: 'Dynamic library portability, code signing and corresponding-source distribution require separate release verification.',
  components: components.map(({ binary, ...component }) => component), licenses,
}, null, 2));
console.log(`Staged ${values.target} media tools in ${destination}`);
