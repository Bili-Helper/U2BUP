// Run using: playwright-cli -s=u2bup-workflow run-code --filename=app/scripts/workflow-ui-smoke.mjs
// Requires a development UI on port 5173. All API requests are intercepted with synthetic data.
async page => {
  const failures=[];
  page.on('pageerror',error=>failures.push(error.message));
  const assert=(condition,message)=>{if(!condition)throw Error(message);};
  const scan={running:false,completed:0,total:0,message:''};
  const assets=[{id:'local-demo',relative_path:'example.mp4',name:'example.mp4',room_id:'123456',room_name:'演示主播',title:'【演示主播】舞蹈练习 2026-09-21',display_title:null,bytes:50000,modified_ms:1,extension:'mp4',role:'source',started_at:'2026-09-21T12:00:00+08:00',time_source:'xml',metadata:{duration:600,width:1280,height:720,codec:'h264',aspect:'16:9',signature:'demo',streams:[]},sidecars:[],warnings:[]}];
  const videos=[{id:'video-demo',snippet:{title:'【演示主播】2026-09-21 舞蹈练习 P1',description:'原有说明\nhttps://live.bilibili.com/123456',tags:['保留标签'],categoryId:'22',channelId:'demo-channel'},status:{privacyStatus:'private',uploadStatus:'processed'},contentDetails:{duration:'PT10M',hasCustomThumbnail:false}}];
  const snapshot={library:{root:'synthetic',scanned_at:null,assets,rooms:[],sidecar_count:0,errors:[]},scan,jobs:[],plans:[],settings:{}};
  const state={drafts:[],runs:[],metadata:[],playlists:{channel_id:'demo-channel',items:[{id:'playlist-demo',snippet:{title:'演示主播 123456',description:'原有列表说明\nhttps://live.bilibili.com/123456'},video_ids:[]}]}};
  let previewRequests=0,applyRequests=0;
  await page.unrouteAll({behavior:'ignoreErrors'});
  await page.route('**/api/**',async route=>{
    const request=route.request(),path='/'+request.url().split('/').slice(3).join('/').split('?')[0],body=request.method()==='POST'?request.postDataJSON():null;
    let response={};
    if(path==='/api/snapshot')response=snapshot;
    else if(path==='/api/status')response={scan,jobs:[]};
    else if(path==='/api/tasks')response={tasks:[],scan};
    else if(path==='/api/youtube'||path==='/api/youtube/status')response={enabled:true,connected:true,configured:true,channel:{id:'demo-channel'},videos,artifacts:[],uploads:[],batches:[]};
    else if(path==='/api/workflows')response=state;
    else if(path==='/api/workflows/drafts'){state.drafts=[body];response=body;}
    else if(path==='/api/workflows/preview'){
      previewRequests++;
      assert(body.items.length===1,'frozen target count');
      assert(body.items[0].before.description.includes('原有说明'),'before description preserved');
      assert(body.items[0].after.description.includes('原有说明'),'existing prose preserved');
      assert(body.items[0].after.tags.includes('保留标签'),'existing tags preserved');
      response={...body,id:'run-demo',created_at:new Date().toISOString(),status:'pending',items:body.items.map(i=>({...i,status:'pending',message:'',actions:{}}))};state.runs=[response];
    } else if(path==='/api/workflows/runs/run-demo/apply'){applyRequests++;state.runs[0].status='completed';state.runs[0].items.forEach(i=>{i.status='completed';i.message='已应用';});response=state.runs[0];}
    else if(path==='/api/workflows/playlists/sync')response=state.playlists;
    else throw Error(`Unexpected API path: ${path}`);
    await route.fulfill({json:response});
  });
  await page.setViewportSize({width:1640,height:1080});
  await page.goto('http://127.0.0.1:5173/#/pipeline/workflows');
  await page.reload();
  await page.getByRole('heading',{name:'自定义管线.'}).waitFor();
  await page.getByLabel('适应画布').click();
  assert(await page.locator('.flow-node').count()===9,'standard template node count');
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await page.getByRole('status').filter({hasText:'管线已保存'}).waitFor();
  await page.screenshot({path:'app/.local/workflow-canvas-desktop.png',fullPage:true});
  await page.getByRole('tab',{name:/输入数据/}).click();
  await page.getByRole('button',{name:'选本页',exact:true}).click();
  await page.getByRole('button',{name:'预览 1 项',exact:true}).click();
  await page.locator('.flow-result').waitFor();
  const approvals=page.getByRole('checkbox',{name:'已逐项核对本次建议，允许加入应用计划'});
  for(let i=0;i<await approvals.count();i++)await approvals.nth(i).check();
  await page.getByRole('button',{name:'生成 1 项应用计划',exact:true}).click();
  await page.getByRole('heading',{name:'已冻结的应用计划'}).waitFor();
  await page.screenshot({path:'app/.local/workflow-preview-desktop.png',fullPage:true});
  await page.getByRole('button',{name:'确认应用这些变更',exact:true}).click();
  await page.getByRole('status').filter({hasText:'执行已返回'}).waitFor();
  assert(await page.getByRole('button',{name:'确认应用这些变更',exact:true}).isDisabled(),'completed apply button must disable');
  assert(previewRequests===1&&applyRequests===1,'exact single preview/apply');
  await page.reload();
  await page.getByRole('tab',{name:'保存与运行记录'}).click();
  await page.getByRole('button',{name:'查看结果',exact:true}).click();
  await page.getByRole('heading',{name:'已冻结的应用计划'}).waitFor();
  await page.getByRole('tab',{name:'模块画布'}).click();
  await page.getByRole('button',{name:'AI Agent 提案接口',exact:true}).click();
  assert(await page.locator('.flow-node').count()===10,'module added');
  await page.getByRole('button',{name:'删除模块',exact:true}).click();
  assert(await page.locator('.flow-node').count()===9,'module deleted');
  await page.getByRole('button',{name:'撤销',exact:true}).click();
  assert(await page.locator('.flow-node').count()===10,'undo restored module');
  await page.getByRole('button',{name:'重做',exact:true}).click();
  assert(await page.locator('.flow-node').count()===9,'redo deleted module');
  await page.setViewportSize({width:390,height:844});
  await page.screenshot({path:'app/.local/workflow-canvas-mobile.png',fullPage:true});
  const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>window.innerWidth+1);
  assert(!overflow,'mobile document must not overflow horizontally');
  assert(!failures.length,failures.join('\n'));
  return {passed:true,previewRequests,applyRequests,nodeCount:9,viewports:['1640x1080','390x844'],screenshots:['workflow-canvas-desktop.png','workflow-preview-desktop.png','workflow-canvas-mobile.png']};
}
