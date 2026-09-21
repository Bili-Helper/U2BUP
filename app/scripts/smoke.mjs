import assert from 'node:assert/strict';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdir,readFile,writeFile,utimes,stat} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

const exec=promisify(execFile),app=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const testRoot=path.join(app,'.local','smoke',new Date().toISOString().replaceAll(/[:.]/g,'-'));
const library=path.join(testRoot,'library'),data=path.join(testRoot,'state'),output=path.join(testRoot,'output');
const room=path.join(library,"123-测试'主播");await mkdir(room,{recursive:true});
const files=[];
for(const [i,shape] of [[0,'320x180'],[1,'320x180'],[2,'180x320'],[3,'320x180']]){
  const file=path.join(room,`录制-123-20230620-0000${String(i*4).padStart(2,'0')}-100-测试_PART00${i}.flv`);
  await exec('ffmpeg',['-hide_banner','-v','error','-f','lavfi','-i',`testsrc2=size=${shape}:rate=25`,'-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','4','-c:v','libx264','-preset','ultrafast','-pix_fmt','yuv420p','-c:a','aac','-f','flv',file],{windowsHide:true});
  await utimes(file,new Date('2023-06-20'),new Date('2023-06-20'));files.push(file);
}
await writeFile(path.join(room,'broken.flv'),'not a video');await utimes(path.join(room,'broken.flv'),new Date('2023-06-20'),new Date('2023-06-20'));
const hashes=await Promise.all(files.map(async f=>createHash('sha256').update(await readFile(f)).digest('hex')));
const exe=process.env.U2BUP_TEST_SERVER||path.join(app,'target','debug',process.platform==='win32'?'u2bup-server.exe':'u2bup-server');
const child=spawn(exe,['--library',library,'--data',data,'--output',output,'--port','0'],{windowsHide:true,stdio:['ignore','pipe','pipe']});
let diagnostic='';child.stdout.on('data',v=>diagnostic+=v);child.stderr.on('data',v=>diagnostic+=v);
const deadline=(ms=60000)=>Date.now()+ms;
async function until(f,timeout=60000){const end=deadline(timeout);while(Date.now()<end){const result=await f();if(result)return result;await new Promise(r=>setTimeout(r,150));}throw new Error(`Timed out\n${diagnostic}`);}
try{
  const conn=await until(async()=>{try{return JSON.parse(await readFile(path.join(data,'connection.json'),'utf8'));}catch{return null;}});
  const url=new URL(conn.url),token=new URLSearchParams(url.hash.slice(1)).get('token'),base=url.origin;
  async function api(p,body){const r=await fetch(base+'/api'+p,{method:body===undefined?'GET':'POST',headers:{Authorization:`Bearer ${token}`,...(body===undefined?{}:{'Content-Type':'application/json'})},body:body===undefined?undefined:JSON.stringify(body)});const v=await r.json();if(!r.ok)throw new Error(v.error);return v;}
  assert.equal((await fetch(base+'/api/snapshot')).status,401,'unauthenticated API rejected');
  assert.equal((await fetch(base+'/api/scan',{method:'POST',headers:{Authorization:`Bearer ${token}`,Origin:'https://example.invalid','Content-Type':'application/json'},body:'{}'})).status,403,'cross-origin mutation rejected');
  await api('/scan',{});await until(async()=>!(await api('/status')).scan.running);
  const snap=await api('/snapshot');assert.equal(snap.library.assets.length,5);assert.equal(snap.library.rooms.length,1);
  const good=snap.library.assets.filter(a=>a.metadata?.duration);assert.equal(good.length,4);
  const request={asset_ids:snap.library.assets.map(a=>a.id),max_duration:42900,max_bytes:250000000000,max_gap:1800,include_legacy:false};
  const plan=await api('/plans',request);assert.equal(plan.outputs.length,3,'wide-wide, tall, wide groups');assert.equal(plan.outputs[0].inputs.length,2);assert.equal(plan.blocked.length,1,'broken file blocked');
  const changes=await api('/titles/preview',{asset_ids:[good[0].id],template:'{主播} · {日期} · {标题}',find:'测试',replace:'示例',regex:false});
  await api('/titles/apply',changes);assert.equal((await api('/snapshot')).library.assets.find(a=>a.id===good[0].id).display_title,changes[0].after);
  await assert.rejects(()=>api('/titles/apply',changes),/标题已变化/);
  await assert.rejects(()=>api('/titles/preview',{asset_ids:[good[0].id],template:'{标题}',find:'[',replace:'',regex:true}));
  const cancelJob=await api(`/plans/${plan.id}/execute`,{});
  await api(`/jobs/${cancelJob.id}/cancel`,{});
  const cancelled=await until(async()=>{const j=(await api('/status')).jobs.find(j=>j.id===cancelJob.id);return ['cancelled','completed','failed'].includes(j?.status)?j:null;});
  assert.equal(cancelled.status,'cancelled',cancelled.message);
  const job=await api(`/plans/${plan.id}/execute`,{});
  const completed=await until(async()=>{const j=(await api('/status')).jobs.find(j=>j.id===job.id);return ['completed','failed','cancelled'].includes(j?.status)?j:null;});
  assert.equal(completed.status,'completed',completed.message);assert.equal(completed.completed_outputs.length,3);
  for(const artifact of completed.completed_outputs)assert.ok((await stat(artifact.path)).size>0);
  const retry=await api(`/plans/${plan.id}/execute`,{});
  const retried=await until(async()=>{const j=(await api('/status')).jobs.find(j=>j.id===retry.id);return ['completed','failed'].includes(j?.status)?j:null;});
  assert.equal(retried.status,'completed',retried.message);
  assert.deepEqual(retried.completed_outputs.map(a=>a.path),completed.completed_outputs.map(a=>a.path),'validated artifacts reused');
  await api('/scan',{});await until(async()=>!(await api('/status')).scan.running);assert.equal((await api('/snapshot')).library.assets.length,5);
  const hashesAfter=await Promise.all(files.map(async f=>createHash('sha256').update(await readFile(f)).digest('hex')));assert.deepEqual(hashesAfter,hashes,'source videos untouched');
  const longFile=path.join(room,'录制-123-20230621-000000-100-自动切割.flv');
  await exec('ffmpeg',['-hide_banner','-v','error','-f','lavfi','-i','testsrc2=size=160x90:rate=10','-f','lavfi','-i','sine=frequency=660:sample_rate=48000','-t','125','-c:v','libx264','-preset','ultrafast','-c:a','aac','-f','flv',longFile],{windowsHide:true});
  await utimes(longFile,new Date('2023-06-21'),new Date('2023-06-21'));
  const longHash=createHash('sha256').update(await readFile(longFile)).digest('hex');
  await api('/scan',{});await until(async()=>!(await api('/status')).scan.running);
  const longAsset=(await api('/snapshot')).library.assets.find(a=>a.name.includes('自动切割'));
  const cutPlan=await api('/plans',{...request,asset_ids:[longAsset.id],max_duration:60});
  assert.equal(cutPlan.outputs.length,3);assert.equal(cutPlan.blocked.length,0);assert.equal(cutPlan.outputs[0].cut_start,0);
  for(let i=1;i<cutPlan.outputs.length;i++)assert.ok(Math.abs(cutPlan.outputs[i].cut_start-cutPlan.outputs[i-1].cut_start-cutPlan.outputs[i-1].duration)<0.001,'cut ranges continuous');
  const cutJob=await api(`/plans/${cutPlan.id}/execute`,{});
  const cutDone=await until(async()=>{const j=(await api('/status')).jobs.find(j=>j.id===cutJob.id);return ['completed','failed'].includes(j?.status)?j:null;},120000);
  assert.equal(cutDone.status,'completed',cutDone.message);assert.equal(cutDone.completed_outputs.length,3);
  for(const a of cutDone.completed_outputs){assert.ok(a.duration<=60);const {stdout}=await exec('ffprobe',['-v','error','-show_streams','-of','json',a.path],{windowsHide:true});assert.equal(JSON.parse(stdout).streams[0].codec_name,'h264');}
  assert.equal(createHash('sha256').update(await readFile(longFile)).digest('hex'),longHash);
  const yt=await api('/youtube');assert.equal(yt.connected,false);assert.ok(yt.artifacts.length>=6);assert.equal(yt.uploads.length,0);
  await assert.rejects(()=>api('/youtube/uploads',{artifact_ids:[cutDone.completed_outputs[0].output_id],metadata:{title:'测试',privacy:'private',made_for_kids:false}}),/连接频道/);
  assert.equal((await fetch(base+'/oauth/youtube/callback?state=invalid&code=x')).status,400);
  await utimes(files[0],new Date(),new Date());
  const stale=await api(`/plans/${plan.id}/execute`,{});
  const rejected=await until(async()=>{const j=(await api('/status')).jobs.find(j=>j.id===stale.id);return j?.status==='failed'?j:null;});assert.match(rejected.message,/源文件已变化/);
  const report={passed:true,testRoot,checks:['authentication','origin rejection','scan','unknown duration blocking','aspect order','Chinese and apostrophe paths','title preview/apply','stale title conflict','invalid regex','cancel then retry','real FFmpeg merge and validation','idempotent output reuse','rescan idempotence','source SHA-256 unchanged','stale input rejection','automatic exact split','continuous split ranges','split source SHA-256 unchanged','YouTube setup status','unauthorized upload blocked','invalid OAuth callback rejected'],artifacts:[...completed.completed_outputs,...cutDone.completed_outputs]};
  await writeFile(path.join(testRoot,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{child.kill();}
