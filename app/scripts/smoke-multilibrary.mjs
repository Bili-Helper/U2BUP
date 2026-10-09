import assert from 'node:assert/strict';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdir, readFile, writeFile, utimes, stat, copyFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const exec = promisify(execFile);
const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.join(app, '.local', 'multilibrary-smoke', new Date().toISOString().replaceAll(/[:.]/g, '-'));
const legacy = path.join(root, 'legacy'), library = path.join(root, 'biliLiveTools');
const data = path.join(root, 'data'), output = path.join(root, 'output');
const streamer = path.join(library, "测试'主播"), merged = path.join(streamer, 'merged');
await mkdir(legacy, {recursive: true});
await mkdir(merged, {recursive: true});
const files = [];
for (const second of ['02', '04']) {
  const file = path.join(merged, `测试'主播_20260925_0953${second}_直播_标题_[9x16_90x160_h264_High].mp4`);
  await exec(process.env.U2BUP_TEST_FFMPEG ?? 'ffmpeg', [
    '-hide_banner', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=90x160:rate=25',
    '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '2',
    '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-c:a', 'aac', file,
  ], {windowsHide: true});
  await utimes(file, new Date('2026-09-25'), new Date('2026-09-25'));
  await writeFile(path.join(streamer, `2026-09-25 09-53-${second}-838 直播 标题.xml`),
    `<i><metadata><video_start_time>${Date.parse(`2026-09-25T13:53:${second}.838Z`)}</video_start_time><room_title>直播 &amp; 标题</room_title><user_name>测试'主播</user_name><room_id>123456</room_id></metadata></i>`);
  const raw = path.join(streamer, `2026-09-25 09-53-${second}-838 直播 标题.mp4`);
  await copyFile(file, raw);
  await utimes(raw, new Date('2026-09-25'), new Date('2026-09-25'));
  files.push(file, raw);
}
const hashes = await Promise.all(files.map(async file => createHash('sha256').update(await readFile(file)).digest('hex')));
const server = process.env.U2BUP_TEST_SERVER ?? path.join(app, 'target', 'release', process.platform === 'win32' ? 'u2bup-server.exe' : 'u2bup-server');
const child = spawn(server, [
  '--library', legacy, '--data', data, '--output', output, '--port', '0',
  '--ffmpeg', process.env.U2BUP_TEST_FFMPEG ?? 'ffmpeg', '--ffprobe', process.env.U2BUP_TEST_FFPROBE ?? 'ffprobe',
], {windowsHide: true, stdio: 'ignore'});
async function until(check) {
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('Multi-library verification timed out');
}
try {
  const connection = await until(async () => {try {return JSON.parse(await readFile(path.join(data, 'connection.json'), 'utf8'));} catch {return null;}});
  const url = new URL(connection.url), token = new URLSearchParams(url.hash.slice(1)).get('token');
  const headers = {Authorization: `Bearer ${token}`};
  async function api(route, body, method = body === undefined ? 'GET' : 'POST') {
    const response = await fetch(url.origin + '/api' + route, {
      method, headers: {...headers, ...(body === undefined ? {} : {'Content-Type': 'application/json'})},
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error);
    return result;
  }
  const {library: registered} = await api('/libraries', {name: 'biliLiveTools', kind: 'folder', path: library});
  async function scan() {
    await api(`/libraries/${registered.id}/scan`, {});
    const status = await until(async () => {
      const value = await api(`/libraries/${registered.id}/status`);
      return value.scanStatus !== 'scanning' ? value : null;
    });
    assert.equal(status.scanStatus, 'idle', status.scanError);
    assert.equal(status.assetCount, 4);
  }
  await scan();
  const snapshot = await api(`/snapshot?libraryId=${registered.id}`);
  assert.equal((await api('/snapshot')).library.assets.length, 0, 'new library must not overwrite legacy root');
  assert.equal(snapshot.library.assets.length, 4);
  assert.equal(snapshot.library.rooms.length, 1);
  assert.equal(snapshot.library.rooms[0].id, '123456');
  for (const asset of snapshot.library.assets) {
    assert.equal(asset.title, '直播 & 标题');
    assert.equal(asset.room_name, "测试'主播");
    assert.equal(asset.sidecars.length, 1);
    assert.ok(asset.metadata.duration > 1);
    assert.ok(asset.started_at.includes('21:53:'), 'XML milliseconds give the authoritative UTC instant');
  }
  const first = snapshot.library.assets[0];
  const preview = await fetch(`${url.origin}/api/assets/${first.id}/media`, {headers: {...headers, Range: 'bytes=0-127'}});
  assert.equal(preview.status, 206);
  assert.equal((await preview.arrayBuffer()).byteLength, 128);
  const thumb = await fetch(`${url.origin}/api/assets/${first.id}/thumbnail`, {headers});
  assert.equal(thumb.status, 200);
  assert.ok((await thumb.arrayBuffer()).byteLength > 100);
  const changes = await api('/titles/preview', {asset_ids: [first.id], template: '{主播} · {标题}', find: '', replace: '', regex: false});
  await api('/titles/apply', changes);
  await api(`/assets/${first.id}`, {pubTitle: '发布标题', pubTags: ['保留标签']}, 'PUT');
  await scan();
  const refreshed = await api(`/assets/${first.id}`);
  assert.equal(refreshed.displayTitle, changes[0].after);
  assert.equal(refreshed.pubTitle, '发布标题');
  assert.deepEqual(refreshed.pubTags, ['保留标签']);
  const request = {max_duration: 42900, max_bytes: 250000000000, max_gap: 1800, include_legacy: true};
  const legacyPlan = await api('/plans', {...request, asset_ids: snapshot.library.assets.filter(asset => asset.role === 'legacy').map(asset => asset.id)});
  assert.equal(legacyPlan.outputs.length, 2, 'already merged historical outputs stay independent');
  const plan = await api('/plans', {...request, asset_ids: snapshot.library.assets.filter(asset => asset.role === 'source').map(asset => asset.id)});
  assert.equal(plan.blocked.length, 0);
  assert.equal(plan.outputs.length, 1);
  assert.equal(plan.outputs[0].inputs.length, 2);
  const job = await api(`/plans/${plan.id}/execute`, {});
  const completed = await until(async () => {
    const value = (await api('/status')).jobs.find(item => item.id === job.id);
    return ['completed', 'failed', 'cancelled'].includes(value?.status) ? value : null;
  });
  assert.equal(completed.status, 'completed', completed.message);
  assert.equal(completed.completed_outputs.length, 1);
  assert.ok((await stat(completed.completed_outputs[0].path)).size > 100);
  const direct = await api('/youtube/direct', {asset_ids: [first.id, first.id]});
  assert.deepEqual([direct.added, direct.existing, direct.rejected.length], [1, 0, 0]);
  const registeredDirect = (await api('/youtube')).artifacts.find(item => item.output_id === `direct-${first.id}`);
  assert.equal(path.resolve(registeredDirect.path.replace(/^\\\\\?\\/, '')).toLowerCase(), path.resolve(library, first.relative_path).toLowerCase(), 'direct upload points at the original file');
  assert.deepEqual(registeredDirect.source_ids, [first.id]);
  assert.equal(registeredDirect.local_title, '发布标题', 'upload preparation uses the library publishing title');
  assert.equal((await api(`/youtube/direct/direct-${first.id}`, undefined, 'DELETE')).removed, true);
  await api(`/libraries/${registered.id}?deleteAssets=true`, undefined, 'DELETE');
  const staleJob = await api(`/plans/${plan.id}/execute`, {});
  const rejected = await until(async () => {
    const value = (await api('/status')).jobs.find(item => item.id === staleJob.id);
    return ['completed', 'failed'].includes(value?.status) ? value : null;
  });
  assert.equal(rejected.status, 'failed', 'removed library cannot fall back to an unrelated legacy path');
  assert.deepEqual(await Promise.all(files.map(async file => createHash('sha256').update(await readFile(file)).digest('hex'))), hashes);
  const report = {passed: true, root, checks: [
    'isolated library scan', 'merged filename and XML metadata', 'authoritative XML time',
    'scoped snapshot', 'MP4 range preview', 'FFmpeg thumbnail', 'batch display titles',
    'rescan preserves publishing edits', 'real FFmpeg merge from another library',
    'direct upload registration in place', 'removed library rejects saved plan', 'source SHA-256 unchanged',
  ]};
  await writeFile(path.join(root, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
} finally {
  child.kill();
}
