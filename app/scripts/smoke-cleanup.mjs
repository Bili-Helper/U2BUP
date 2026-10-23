// Real HTTP + filesystem test. All candidate files live in a fresh test directory.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdir,writeFile,readFile,stat,access} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const app=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const root=path.join(app,'.local','cleanup-smoke',new Date().toISOString().replaceAll(/[:.]/g,'-'));
const library=path.join(root,'library'),data=path.join(root,'data');
await mkdir(library,{recursive:true});
await writeFile(path.join(library,'delete.flv'),'synthetic video');
await writeFile(path.join(library,'keep.mp4'),'excluded video');
await writeFile(path.join(library,'聊天.xml'),'<i>保留弹幕</i>');
const before=await stat(path.join(library,'聊天.xml'));
const server=process.env.U2BUP_TEST_SERVER??path.join(app,'target','release','u2bup-server.exe');
const child=spawn(server,['--library',library,'--data',data,'--output',path.join(root,'output'),'--port','0'],{windowsHide:true,stdio:'ignore'});
let launchError;child.on('error',e=>launchError=e);
async function until(check){const deadline=Date.now()+30000;while(Date.now()<deadline){if(launchError)throw launchError;const v=await check();if(v)return v;await new Promise(r=>setTimeout(r,100));}throw Error('Timeout');}
try {
 const c=await until(async()=>{try{return JSON.parse(await readFile(path.join(data,'connection.json'),'utf8'));}catch{return null}});
 const url=new URL(c.url),token=new URLSearchParams(url.hash.slice(1)).get('token');
 async function api(route,body,ok=true){const r=await fetch(url.origin+'/api'+route,{method:body===undefined?'GET':'POST',headers:{Authorization:'Bearer '+token,...(body===undefined?{}:{'Content-Type':'application/json'})},body:body===undefined?undefined:JSON.stringify(body)});const v=await r.json();assert.equal(r.ok,ok,JSON.stringify(v));return v;}
 assert.equal((await fetch(url.origin+'/api/workflows/cleanup')).status,401);
 const {library:lib}=await api('/libraries',{name:'cleanup test',kind:'folder',path:library});
 async function scan(){await api('/libraries/'+lib.id+'/scan',{});await until(async()=>{const v=await api('/libraries/'+lib.id+'/status');return v.scanStatus!=='scanning'?v:null;});}
 await scan();const snapshot=await api('/snapshot?libraryId='+lib.id);assert.equal(snapshot.library.assets.length,2);
 const keep=snapshot.library.assets.find(a=>a.name==='keep.mp4'),remove=snapshot.library.assets.find(a=>a.name==='delete.flv');
 const body={asset_ids:[remove.id,keep.id],excluded_ids:[keep.id],platform:'test',note:'人工核对完整在线副本',confirmed_uploaded:false};
 await api('/workflows/cleanup',body,false);body.confirmed_uploaded=true;
 const plan=await api('/workflows/cleanup',body);assert.equal(plan.items[0].status,'planned');assert.equal(plan.items[1].status,'excluded');await access(path.join(library,'delete.flv'));
 await api('/workflows/cleanup/'+plan.id+'/apply',{confirm_permanent_delete:false},false);
 const result=await api('/workflows/cleanup/'+plan.id+'/apply',{confirm_permanent_delete:true});assert.equal(result.items[0].status,'deleted');
 await assert.rejects(access(path.join(library,'delete.flv')));await access(path.join(library,'keep.mp4'));
 assert.equal(await readFile(path.join(library,'聊天.xml'),'utf8'),'<i>保留弹幕</i>');assert.equal((await stat(path.join(library,'聊天.xml'))).mtimeMs,before.mtimeMs);
 const after=await api('/snapshot?libraryId='+lib.id);assert.ok(after.library.assets.find(a=>a.id===remove.id).warnings.some(w=>w.includes('local_deleted')));
 assert.ok((await api('/tasks')).tasks.some(t=>t.kind==='cleanup'&&t.status==='completed'));
 await scan();assert.equal((await api('/snapshot?libraryId='+lib.id)).library.assets.length,2);
 await writeFile(path.join(library,'delete.flv'),'new content must survive');
 await api('/workflows/cleanup/'+plan.id+'/apply',{confirm_permanent_delete:true});assert.equal(await readFile(path.join(library,'delete.flv'),'utf8'),'new content must survive');
 assert.equal((await api('/workflows/cleanup')).plans.length,1);
 const report={passed:true,checks:['authentication','confirmation','exclusion','preview has no deletion','permanent confirmation','video removed','sidecar content and mtime','cleanup state','task center','rescan metadata','no replay on replacement','durable history'],root};
 await writeFile(path.join(root,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
} finally {child.kill();}
