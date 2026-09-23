import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createNode,createTemplate,normalizeYoutube,normalizeAsset,parseWorkflowGraph,runWorkflow,validateGraph} from '../src/workflow.ts';

function graph(...types){const nodes=types.map((type,index)=>createNode(type,index));return{version:1,id:'test',name:'测试管线',nodes,edges:nodes.slice(1).map((n,i)=>({id:`e${i}`,source:nodes[i].id,target:n.id,port:nodes[i].type==='filter'?'yes':'out'}))};}
function video(id,title,description='',extra={}){return normalizeYoutube({id,snippet:{title,description,tags:[],categoryId:'20',publishedAt:'2026-09-22T00:00:00Z'},status:{privacyStatus:'private'},contentDetails:{duration:'PT1H'},...extra});}
function run(g,records,context={}){const result=runWorkflow(g,records,context);assert.equal(result.valid,true,result.errors.join('; '));return result.records;}

test('normalization does not mistake publishedAt, categoryId or licensedContent for recording/content/copyright facts',()=>{
  const source=video('one','无结构标题','',{contentDetails:{duration:'PT1H2M3S',licensedContent:true}});
  assert.equal(source.recordedAt,'');assert.equal(source.durationSeconds,3723);assert.equal(source.copyrightStatus,'unknown');
  const [result]=run(graph('input','parse','classify','copyright','output'),[source]);
  assert.equal(result.after.platform,'unknown');assert.deepEqual(result.after.labels,[]);assert.equal(result.after.copyrightStatus,'unknown');
  assert.equal(result.issues.some(i=>i.code==='copyright_review'),false);
});

test('recorder title parses explicit identities and moves date/time/room into an idempotent managed description',()=>{
  const g=graph('input','parse','classify','metadata','tags','output');
  const source=video('recorder','【主播甲】 12345 20260901 123456 123 跳舞 P2','人工写的介绍\nhttps://live.bilibili.com/12345');
  source.tags=['收藏','dance'];
  const snapshot=JSON.stringify(source),[first]=run(g,[source]);
  assert.equal(JSON.stringify(source),snapshot,'preview must not mutate its inputs');
  assert.equal(first.after.title,'【主播甲】跳舞 P2');assert.equal(first.after.recordedAt,'2026-09-01 12:34:56');assert.equal(first.after.roomId,'12345');
  assert.deepEqual(first.after.tags,['收藏','Dance','Bilibili直播','主播甲']);
  assert.match(first.after.description,/^人工写的介绍\nhttps:\/\/live\.bilibili\.com\/12345\n\n/);
  assert.match(first.after.description,/原始标题：【主播甲】 12345 20260901 123456 123 跳舞 P2/);
  const [second]=run(g,[first.after]);assert.equal(second.after.title,first.after.title);assert.equal(second.after.description,first.after.description);assert.deepEqual(second.changes,[]);
  // A refreshed remote record recovers managed metadata even without an application-side metadata cache.
  const refreshed=video('recorder',first.after.title,first.after.description);refreshed.tags=first.after.tags;
  const [third]=run(g,[refreshed]);assert.equal(third.after.description,first.after.description);assert.equal(third.after.recordedAt,'2026-09-01 12:34:56');
});

test('ambiguous names and rN_M remain reviewable and are not fabricated as verified identities or parts',()=>{
  const g=graph('input','parse','metadata','output');g.nodes[1].config.removeTechnical=true;
  const title='主播乙 夜间场 2026 09 01 ASMR merged r1_2';
  const [result]=run(g,[video('ambiguous',title)]);
  assert.equal(result.after.title,title);assert.equal(result.after.part,'');assert.equal(result.after.recordedAt,'2026-09-01');assert.match(result.after.sessionTitle,/r1 2/);
  assert.equal(result.status,'review');assert.ok(result.evidence===undefined);assert.ok(result.after.evidence.some(e=>e.field==='creator'&&e.confidence==='low'));
  assert.ok(result.issues.some(i=>i.code==='unknown_technical_marker'));
});

test('invalid calendar dates and multiple real dates preserve uncertainty',()=>{
  const g=graph('input','parse','output');
  const [invalid,multiple]=run(g,[video('invalid','【甲】20260230 聊天'),video('multi','【乙】20260901 20260902 夜间直播')]);
  assert.equal(invalid.after.recordedAt,'');assert.match(invalid.after.sessionTitle,/20260230/);
  assert.equal(multiple.after.recordedAt,'2026-09-01');assert.equal(multiple.after.recordedEnd,'2026-09-02');assert.equal(multiple.status,'review');
});

test('source and content are independent multi-label categories and tags preserve existing values',()=>{
  const source=video('multi','【甲】 Twitch VTuber ASMR Cosplay 跳舞 游戏','https://twitch.tv/streamer_1');source.tags=['珍藏','ASMR','asmr'];
  const g=graph('input','parse','classify','tags','output');g.nodes[3].config.extraTags='合集,珍藏';
  const [result]=run(g,[source]);
  assert.equal(result.after.platform,'twitch');assert.equal(result.after.roomId,'streamer_1');
  for(const label of ['Twitch','VTuber','ASMR','Cosplay','Dance','游戏'])assert.ok(result.after.labels.includes(label));
  assert.equal(result.after.tags.filter(t=>t.toLowerCase()==='asmr').length,1);assert.ok(result.after.tags.includes('珍藏'));assert.ok(result.after.tags.includes('合集'));
});

test('playlist assignment uses platform + full room evidence, handles ambiguity and avoids repeated members',()=>{
  const g=graph('input','parse','playlist','output'),source=video('p','【主播甲】 12345 20260901 跳舞','https://live.bilibili.com/12345');
  const context={playlists:[{id:'similar',title:'主播甲',description:'bilibili 房间号：123456'},{id:'correct',title:'旧标题 12345',description:'平台：bilibili\n房间号：12345'}]};
  const [result]=run(g,[source],context);assert.equal(result.playlistIntents[0].playlistId,'correct');assert.equal(result.playlistIntents[0].identityKey,'bilibili:12345');assert.equal(result.playlistIntents[0].title,'主播甲');
  assert.match(result.playlistIntents[0].description,/房间号：12345/);
  context.playlists.push({id:'another',title:'bilibili 12345',description:''});
  assert.equal(run(g,[source],context)[0].playlistIntents[0].action,'review');
  g.nodes[2].config.playlistId='correct';context.playlists[1].video_ids=['p'];const existing=run(g,[source],context)[0].playlistIntents;assert.equal(existing[0].playlistId,'correct');assert.match(existing[0].reason,/不会重复加入/);assert.equal(existing[0].title,'主播甲');
});

test('name similarity never creates an implicit playlist binding and conflicting room evidence stops inference',()=>{
  const g=graph('input','parse','playlist','output');
  const [name]=run(g,[video('name','【甲】20260901 聊天')],{playlists:[{id:'same-name',title:'甲',description:''}]});assert.equal(name.playlistIntents[0].action,'review');
  const [conflict]=run(g,[video('conflict','【甲】 12345 20260901 聊天','https://live.bilibili.com/67890')],{identityBindings:[{roomId:'12345',platform:'bilibili',creator:'甲'}]});
  assert.ok(conflict.issues.some(i=>i.code==='room_conflict'));assert.equal(conflict.playlistIntents[0].action,'review');
});

test('a manually confirmed creator overrides the old bracket name and retains original identity evidence',()=>{
  const g=graph('input','parse','metadata','output'),source=video('manual-creator','【旧名称】 12345 20260901 ASMR','https://live.bilibili.com/12345');
  source.creator='新名称';source.evidence.push({field:'creator',value:'新名称',source:'人工校正',confidence:'high'});
  const [result]=run(g,[source],{identityBindings:[{platform:'bilibili',roomId:'12345',creator:'旧名称'}]});
  assert.equal(result.after.creator,'新名称');assert.equal(result.after.title,'【新名称】ASMR');
  assert.equal(result.issues.some(i=>i.code==='creator_conflict'),false);
  assert.ok(result.after.evidence.some(e=>e.field==='originalCreator'&&e.value==='旧名称'));
  assert.match(result.after.description,/原始标题：【旧名称】 12345 20260901 ASMR/);
  source.evidence[0].source='自动解析';
  const [unconfirmed]=run(g,[source]);assert.ok(unconfirmed.issues.some(i=>i.code==='creator_conflict'));
});

test('unsupported platform identities require explicit playlist bindings and per-record bindings work without source inference',()=>{
  const g=graph('input','parse','playlist','output'),source=video('other','【甲】 聊天');source.platform='other';source.roomId='12345';
  const context={playlists:[{id:'twitch-room',title:'Twitch 12345',description:''}]};
  const [unsupported]=run(g,[source],context);assert.equal(unsupported.playlistIntents[0].action,'review');
  source.playlistId='explicit';assert.equal(run(g,[source],context)[0].playlistIntents[0].playlistId,'explicit');
  source.platform='unknown';source.roomId='';assert.equal(run(g,[source],context)[0].playlistIntents[0].playlistId,'explicit');
});

test('managed description recovery preserves a subsequent explicit platform correction',()=>{
  const source=video('manual-platform','【甲】聊天','[U2BUP 元信息]\n平台：twitch\n原始标题：【甲】聊天\n[/U2BUP 元信息]');
  source.platform='other';source.evidence.push({field:'platform',value:'other',source:'人工校正',confidence:'high'});
  const [result]=run(graph('input','parse','output'),[source]);
  assert.equal(result.after.platform,'other');assert.ok(result.issues.some(i=>i.code==='platform_conflict'));
});

test('local identity binding unifies pre-upload metadata without inventing platform from a numeric room',()=>{
  const source=normalizeAsset({id:'asset-a',title:'晚间 ASMR',room_name:'甲',room_id:'456',started_at:'2026-09-01T20:00:00',metadata:{duration:120}});
  assert.equal(source.platform,'unknown');assert.equal(source.privacy,'private');assert.equal(source.categoryId,'22');
  const [result]=run(graph('input','parse','classify','output'),[source],{identityBindings:[{platform:'bilibili',roomId:'456',creator:'甲'}]});
  assert.equal(result.after.platform,'bilibili');assert.ok(result.after.labels.includes('ASMR'));assert.equal(result.after.localAssetId,'asset-a');
});

test('yes/no routing joins exactly the branch reached, independent of node array order',()=>{
  const g=createTemplate('fill-empty'),empty=video('empty','【甲】20260901 ASMR'),manual=video('manual','【乙】20260902 ASMR','保留手写描述');
  const results=run(g,[empty,manual]);
  assert.match(results[0].after.description,/U2BUP 元信息/);assert.ok(results[0].after.tags.includes('ASMR'));
  assert.equal(results[1].after.description,'保留手写描述');assert.deepEqual(results[1].after.tags,[]);
  assert.equal(results[1].trace.find(t=>t.nodeId==='metadata-4').status,'skipped');
  g.nodes.reverse();assert.deepEqual(run(g,[empty,manual]).map(r=>r.after),results.map(r=>r.after));
});

test('parallel conflicting field writes block a record while equal writes merge deterministically',()=>{
  const g=graph('input','tags','tags','join','output');
  g.edges=[{id:'a',source:'input-0',target:'tags-1'},{id:'b',source:'input-0',target:'tags-2'},{id:'c',source:'tags-1',target:'join-3'},{id:'d',source:'tags-2',target:'join-3'},{id:'e',source:'join-3',target:'output-4'}];
  g.nodes[1].config.extraTags='一';g.nodes[2].config.extraTags='二';
  const source=video('parallel','测试'),[conflict]=run(g,[source]);assert.equal(conflict.status,'blocked');assert.ok(conflict.issues.some(i=>i.code==='branch_conflict'));
  assert.deepEqual(conflict.after.tags,[]);
  g.nodes[2].config.extraTags='一';const [same]=run(g,[source]);assert.equal(same.status,'ready');assert.deepEqual(same.after.tags,['一']);
});

test('a later write on one branch supersedes a shared ancestor without a false join conflict',()=>{
  const g=graph('input','tags','tags','classify','join','output');
  g.edges=[{id:'a',source:'input-0',target:'tags-1'},{id:'b',source:'tags-1',target:'tags-2'},{id:'c',source:'tags-1',target:'classify-3'},{id:'d',source:'tags-2',target:'join-4'},{id:'e',source:'classify-3',target:'join-4'},{id:'f',source:'join-4',target:'output-5'}];
  g.nodes[1].config.extraTags='共享';g.nodes[2].config.extraTags='追加';
  const [result]=run(g,[video('shared','ASMR')]);assert.equal(result.status,'ready');assert.deepEqual(result.after.tags,['共享','追加']);assert.deepEqual(result.after.labels,['ASMR']);
});

test('a join preserves original-title provenance recovered from a refreshed remote description',()=>{
  const [first]=run(graph('input','parse','metadata','output'),[video('provenance','【甲】20260901 ASMR')]);
  const refreshed=video('provenance',first.after.title,first.after.description);
  const g=graph('input','parse','tags','classify','join','metadata','output');
  g.edges=[{id:'a',source:'input-0',target:'parse-1'},{id:'b',source:'parse-1',target:'tags-2'},{id:'c',source:'parse-1',target:'classify-3'},{id:'d',source:'tags-2',target:'join-4'},{id:'e',source:'classify-3',target:'join-4'},{id:'f',source:'join-4',target:'metadata-5'},{id:'g',source:'metadata-5',target:'output-6'}];
  const [result]=run(g,[refreshed]);
  assert.equal(result.after.originalTitle,'【甲】20260901 ASMR');assert.equal(result.after.description,first.after.description);
});

test('copyright claims require explicit state and region restrictions remain a separate review',()=>{
  const g=graph('input','copyright','output');
  const [region]=run(g,[video('region','标题','',{contentDetails:{licensedContent:true,regionRestriction:{blocked:['RU','BY']}}})]);
  assert.equal(region.after.copyrightStatus,'unknown');assert.equal(region.status,'review');assert.ok(region.issues.some(i=>i.code==='region_restriction'));assert.equal(region.issues.some(i=>i.code==='copyright_review'),false);
  const [claim]=run(g,[video('claim','标题')],{copyrightReviews:{claim:{status:'claim',note:'人工 Studio 核实'}}});
  assert.equal(claim.after.copyrightStatus,'claim');assert.equal(claim.status,'review');assert.ok(claim.issues.some(i=>i.code==='copyright_review'));
});

test('same title with 3 second and 3 hour durations is a candidate and repeated input IDs are only deduped',()=>{
  const short=video('short','完全相同标题','',{contentDetails:{duration:'PT3S'}}),long=video('long','完全相同标题','',{contentDetails:{duration:'PT3H'}});
  const result=runWorkflow(graph('input','quality','output'),[short,long,short]);
  assert.equal(result.records.length,2);assert.ok(result.records.every(r=>r.issues.some(i=>i.code==='duplicate_candidate')));
  assert.ok(result.records[0].issues.some(i=>i.code==='short_video'));assert.equal(result.records[0].after.privacy,'private');assert.deepEqual(result.records[0].changes,[]);
});

test('thumbnail candidates require a real local link and AI remains an unevaluated structured request',()=>{
  const g=graph('input','thumbnail','ai','output');g.nodes[1].config.seconds=5;
  const local=normalizeAsset({id:'local',title:'录像',metadata:{duration:20}}),[remote,linked]=run(g,[video('remote','录像'),local]);
  assert.equal(remote.after.thumbnailCandidate,undefined);assert.ok(remote.issues.some(i=>i.code==='thumbnail_source_missing'));
  assert.deepEqual(linked.after.thumbnailCandidate,{assetId:'local',seconds:5,status:'review'});assert.equal(linked.after.aiProposal.status,'awaiting_agent');assert.equal(linked.after.title,'录像');assert.equal(linked.status,'review');
  g.nodes[1].config.seconds=20;assert.ok(run(g,[local])[0].issues.some(i=>i.code==='thumbnail_out_of_range'));
});

test('graph import rejects cycles, unreachable nodes, invalid configuration and missing parser dependencies',()=>{
  for(const key of ['standard','fill-empty','library','copyright'])assert.equal(validateGraph(createTemplate(key)).valid,true);
  const cycle=graph('input','parse','metadata','output');cycle.edges.push({id:'cycle',source:'metadata-2',target:'parse-1'});assert.ok(validateGraph(cycle).errors.some(e=>e.includes('循环')));
  const orphan=graph('input','output');orphan.nodes.push(createNode('tags',9));assert.ok(validateGraph(orphan).errors.some(e=>e.includes('未连接')));
  assert.equal(validateGraph(graph('input','metadata','output')).valid,false);
  const invalid=graph('input','tags','output');invalid.nodes[1].config={extraTags:42};assert.throws(()=>parseWorkflowGraph(invalid),/应为有效文本/);
  const unknown=graph('input','output');unknown.nodes[0].config.injected='bad';assert.throws(()=>parseWorkflowGraph(JSON.stringify(unknown)),/未知配置/);
  assert.throws(()=>parseWorkflowGraph('{'),/JSON/);assert.deepEqual(parseWorkflowGraph(JSON.stringify(graph('input','output'))),graph('input','output'));
});
