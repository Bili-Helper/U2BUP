import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { chmod, mkdtemp, rm, utimes } from 'node:fs/promises';
import { createServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const image = process.argv[2] ?? 'u2bup:headless-preview';
const name = `u2bup-smoke-${process.pid}-${Date.now()}`;
const library = await mkdtemp(path.join(os.tmpdir(), 'u2bup-container-'));
// mkdtemp defaults to 0700; the non-root container must be able to read fixtures.
await chmod(library, 0o755);
const docker = async args => (await exec('docker', args, { windowsHide: true, maxBuffer: 1024 * 1024 })).stdout.trim();
const listener = createServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
const base = `http://127.0.0.1:${port}`;
let created = false;
try {
  await docker(['run', '--rm', '--user', '0', '--mount', `type=bind,source=${library},target=/fixtures`, '--entrypoint', 'ffmpeg', image,
    '-hide_banner', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=10', '-t', '2', '-c:v', 'libx264', '/fixtures/test.mp4']);
  await utimes(path.join(library, 'test.mp4'), new Date('2023-01-01'), new Date('2023-01-01'));
  await docker(['run', '--detach', '--name', name, '--publish', `127.0.0.1:${port}:4173`,
    '--mount', `type=bind,source=${library},target=/media/library,readonly`,
    '--tmpfs', '/data:uid=10001,gid=10001', '--tmpfs', '/output:uid=10001,gid=10001', '--tmpfs', '/tmp',
    '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', image,
    '--headless', '--bind', '0.0.0.0', '--public-origin', base,
    '--library', '/media/library', '--data', '/data', '--output', '/output']);
  created = true;
  let connection;
  for (let i = 0; i < 60; i++) {
    try {
      connection = JSON.parse(await docker(['exec', name, 'cat', '/data/connection.json']));
      break;
    } catch { await new Promise(resolve => setTimeout(resolve, 500)); }
  }
  assert.ok(connection, 'server creates session in headless container');
  const url = new URL(connection.url);
  assert.equal(url.origin, base, 'launch URL uses the published host port');
  const token = new URLSearchParams(url.hash.slice(1)).get('token');
  const headers = { Authorization: `Bearer ${token}` };
  assert.equal((await fetch(`${base}/`)).status, 200);
  assert.equal((await fetch(`${base}/api/status`)).status, 401);
  assert.equal((await fetch(`${base}/api/status`, { headers: { ...headers, Origin: 'https://example.invalid' } })).status, 403);
  const yt = await (await fetch(`${base}/api/youtube`, { headers })).json();
  assert.equal(yt.enabled, false, 'headless explicitly disables YouTube');
  assert.equal((await fetch(`${base}/api/youtube/auth/start`, { method: 'POST', headers })).status, 503);
  assert.equal((await fetch(`${base}/api/scan`, { method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' }, body: '{}' })).status, 200);
  let status;
  for (let i = 0; i < 60; i++) {
    status = await (await fetch(`${base}/api/status`, { headers })).json();
    if (!status.scan.running) break;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  assert.equal(status.scan.running, false);
  const snapshot = await (await fetch(`${base}/api/snapshot`, { headers })).json();
  assert.equal(snapshot.library.assets.length, 1);
  assert.ok(snapshot.library.assets[0].metadata.duration > 0, 'bundled FFprobe probes real media');
  const mounts = JSON.parse(await docker(['inspect', '--format', '{{json .Mounts}}', name]));
  assert.equal(mounts.find(mount => mount.Destination === '/media/library').RW, false);
  await docker(['stop', '--time', '75', name]);
  assert.equal(await docker(['inspect', '--format', '{{.State.ExitCode}}', name]), '0', 'SIGTERM shuts down cleanly');
  console.log('PASS: headless startup, published port, session authentication, Origin rejection, YouTube disabled, read-only media scan, FFmpeg/FFprobe, SIGTERM.');
} finally {
  if (created) await docker(['rm', '--force', name]);
  await rm(library, { recursive: true, force: true });
}
