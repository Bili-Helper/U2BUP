import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from 'node:net';

const app=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const root=path.join(app,'.local','headless-smoke',new Date().toISOString().replaceAll(/[:.]/g,'-'));
await mkdir(path.join(root,'library'),{recursive:true});
const socket=createServer();
await new Promise(resolve=>socket.listen(0,'127.0.0.1',resolve));
const port=socket.address().port;
await new Promise(resolve=>socket.close(resolve));
const base=`http://127.0.0.1:${port}`;
const exe=process.env.U2BUP_TEST_SERVER??path.join(app,'target','release',process.platform==='win32'?'u2bup-server.exe':'u2bup-server');
const child=spawn(exe,['--headless','--bind','127.0.0.1','--public-origin',base,'--port',String(port),'--library',path.join(root,'library'),'--data',path.join(root,'data'),'--output',path.join(root,'output')],{windowsHide:true,stdio:'ignore'});
let spawnError;child.on('error',error=>spawnError=error);
try{
  let connection;
  for(let i=0;i<100;i++){
    if(spawnError)throw spawnError;
    try{connection=JSON.parse(await readFile(path.join(root,'data','connection.json'),'utf8'));break;}catch{await new Promise(resolve=>setTimeout(resolve,100));}
  }
  assert.ok(connection,'headless startup');
  const url=new URL(connection.url);assert.equal(url.origin,base);
  const headers={Authorization:`Bearer ${new URLSearchParams(url.hash.slice(1)).get('token')}`};
  assert.equal((await fetch(base+'/api/tasks')).status,401);
  assert.equal((await fetch(base+'/api/tasks',{headers:{...headers,Origin:'https://example.invalid'}})).status,403);
  const session=await fetch(base+'/api/session',{method:'POST',headers});
  assert.equal(session.status,200);assert.match(session.headers.get('set-cookie'),/HttpOnly; SameSite=Strict/);
  const snapshot=await(await fetch(base+'/api/snapshot',{headers})).json();
  assert.equal(snapshot.settings.mode,'Headless · 单用户');
  for(const endpoint of ['/api/youtube','/api/youtube/status']){
    const response=await fetch(base+endpoint,{headers});assert.equal(response.status,200);
    const value=await response.json();assert.equal(value.enabled,false);assert.equal(value.connected,false);assert.ok(value.reason);
  }
  for(const endpoint of ['/api/youtube/config','/api/youtube/connect','/api/youtube/sync','/api/youtube/uploads']){
    assert.equal((await fetch(base+endpoint,{method:'POST',headers:{...headers,'Content-Type':'application/json'},body:'{}'})).status,503);
  }
  assert.equal((await fetch(base+'/oauth/youtube/callback?state=invalid&code=x')).status,503);
  assert.deepEqual((await(await fetch(base+'/api/tasks',{headers})).json()).tasks,[]);
  const report={passed:true,version:snapshot.settings.version,checks:['headless startup','public origin','session authentication','cross-origin rejection','headless mode label','safe YouTube status without keyring','OAuth and YouTube writes blocked','task center available'],root};
  await writeFile(path.join(root,'report.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
}finally{child.kill();}
