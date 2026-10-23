// playwright-cli -s=u2bup-cleanup run-code --filename=app/scripts/cleanup-ui-smoke.mjs
// Synthetic API only; no real files, credentials or remote mutations.
async page => {
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const assert=(v,m)=>{if(!v)throw Error(m)};
  const assets=['delete.flv','保留.mp4'].map((name,i)=>({id:String(i),name,relative_path:name,title:name,room_id:'demo',room_name:'演示',bytes:100,modified_ms:1,extension:name.split('.').pop(),role:'source',started_at:null,time_source:'unknown',metadata:null,sidecars:['聊天.xml'],warnings:[]}));
  const scan={running:false,completed:0,total:0,message:''};
  const snapshot={library:{root:'synthetic',scanned_at:null,assets,rooms:[],sidecar_count:1,errors:[]},scan,jobs:[],plans:[],settings:{}};
  const plans=[];let applies=0;
  await page.unrouteAll({behavior:'ignoreErrors'});
  await page.route('**/api/**',async route=>{
    const req=route.request(),path='/'+req.url().split('/').slice(3).join('/').split('?')[0],body=req.method()==='POST'?req.postDataJSON():null;
    let response={};
    if(path==='/api/libraries')response={libraries:[]};
    else if(path==='/api/snapshot')response=snapshot;
    else if(path==='/api/status')response={scan,jobs:[]};
    else if(path==='/api/tasks')response={tasks:[],scan};
    else if(path==='/api/youtube'||path==='/api/youtube/status')response={connected:false,videos:[],artifacts:[],uploads:[],batches:[]};
    else if(path==='/api/workflows')response={drafts:[],runs:[],metadata:[],playlists:[],modules:{modules:[],disabledBuiltinIds:[]},identityLedger:{items:[]}};
    else if(path==='/api/workflows/cleanup'&&body){
      assert(body.confirmed_uploaded,'manual confirmation sent');assert(body.excluded_ids.includes('1'),'excluded video sent');
      response={...body,id:'plan-1',created_at:'2026-10-23T12:00:00Z',items:body.asset_ids.map(id=>({asset_id:id,path:assets[Number(id)].name,bytes:100,status:body.excluded_ids.includes(id)?'excluded':'planned',message:''}))};plans.push(response);
    } else if(path==='/api/workflows/cleanup')response={supported:true,plans,assets:[]};
    else if(path==='/api/workflows/cleanup/plan-1/apply'){
      assert(body.confirm_permanent_delete,'permanent confirmation sent');applies++;plans[0].items.forEach(i=>{if(i.status==='planned')i.status='deleted'});assets[0].warnings=['本地视频已清理；弹幕和元数据保留'];response=plans[0];
    } else throw Error('Unexpected API '+path);
    await route.fulfill({json:response});
  });
  await page.setViewportSize({width:1440,height:1000});
  await page.goto('http://127.0.0.1:5198/#/pipeline/workflows');
  await page.reload();
  await page.getByRole('tab',{name:'本地视频清理',exact:true}).click();
  await page.getByRole('heading',{name:'上传后清理本地视频',exact:true}).waitFor();
  const preview=page.getByRole('button',{name:'生成冻结清理预览',exact:true});
  assert(await preview.isDisabled(),'preview starts disabled');
  await page.getByRole('button',{name:'选择筛选结果',exact:true}).click();
  await page.getByLabel('保留 保留.mp4',{exact:true}).check();
  await page.getByLabel('目标平台 / 账号',{exact:true}).fill('外部平台');
  await page.getByLabel('核对说明 / 在线地址',{exact:true}).fill('已核对全部内容');
  await page.getByRole('checkbox',{name:'我已核对所选视频的完整内容已上传并可在线访问（保留项除外）'}).check();
  await preview.click();
  const apply=page.getByRole('button',{name:'永久删除 1 个视频',exact:true});
  await apply.waitFor();assert(await apply.isDisabled(),'apply requires explicit approval');
  await page.getByRole('checkbox',{name:'永久删除此冻结清单中的待执行视频，不移入回收站，无法撤销'}).check();
  await apply.click();await page.getByText('已清理',{exact:true}).waitFor();assert(applies===1,'exactly one apply');
  assert(await page.getByRole('button',{name:'永久删除 0 个视频',exact:true}).isDisabled(),'completed plan cannot replay');
  const download=page.waitForEvent('download');await page.getByRole('button',{name:'导出 CSV 清单',exact:true}).click();assert((await download).suggestedFilename().endsWith('.csv'),'CSV download');
  await page.evaluate(()=>window.scrollTo(0,0));
  await page.screenshot({path:'app/.local/cleanup-desktop.png',fullPage:true});
  await page.reload();await page.getByRole('tab',{name:'本地视频清理',exact:true}).click();
  await page.getByRole('button',{name:/2026-10-23T12:00:00Z/}).click();await page.getByText('已清理',{exact:true}).waitFor();
  await page.getByLabel('确认方式',{exact:true}).selectOption('platform_verified');assert(await page.getByRole('heading',{name:/清理清单/}).count()===0,'changing method invalidates plan');
  await page.setViewportSize({width:390,height:844});await page.waitForFunction(()=>document.querySelector('.sidebar').getBoundingClientRect().right<=1);await page.evaluate(()=>scrollTo(0,0));await page.screenshot({path:'app/.local/cleanup-mobile.png',fullPage:true});
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'mobile horizontal overflow');
  assert(!errors.length,errors.join('\n'));return {passed:true,applies,viewports:['1440x1000','390x844'],checks:['confirmation','exclusion','freeze','single apply','CSV','reload','method invalidation','mobile']};
}
