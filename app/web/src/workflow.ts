import type { Asset } from './types.ts';
import type { ConfigField, HostIntent } from './modules/types.ts';
import {
  getModuleDefinition,
  isModuleEnabled,
  listEnabledCatalog,
  registerBuiltin,
  resolveModuleType,
  runModuleHandler,
} from './modules/registry.ts';
import { hostIntentFromPlaylist, playlistIntentFromHost } from './modules/types.ts';

export type { ConfigField, HostIntent };
export {
  applyRegistryState,
  exportRegistryState,
  installLocalPackage,
  uninstallLocalModule,
  setBuiltinEnabled,
  setLocalModuleEnabled,
  listAllModules,
  listEnabledCatalog,
  parseModulePackage,
  exampleTagDictionaryPackage,
} from './modules/registry.ts';
export { isKnownIntentKind, KNOWN_INTENT_KINDS } from './modules/types.ts';

/** Pure, local planning engine. No node performs network or filesystem writes. */
/** Short aliases (parse) and full ids (com.u2bup.builtin.parse) are both accepted. */
export type NodeType = string;
export interface WorkflowNode { id:string; type:NodeType; position:{x:number;y:number}; config:Record<string,unknown>; enabled:boolean; label?:string }
export interface WorkflowEdge { id:string; source:string; target:string; port?:'out'|'yes'|'no' }
export interface WorkflowGraph { version:1; id:string; name:string; nodes:WorkflowNode[]; edges:WorkflowEdge[]; moduleApi?:1 }
export interface NodeDefinition { type:NodeType; label:string; description:string; group:string; color:string; fields:ConfigField[]; defaults:Record<string,unknown> }
export interface Evidence { field:string; value:string; source:string; confidence:'high'|'medium'|'low' }
export interface WorkflowIssue { code:string; message:string; severity:'info'|'review'|'error'; nodeId?:string }
export interface NormalizedRecord {
  id:string; kind:'youtube'|'local'; title:string; description:string; tags:string[]; privacy:string; categoryId:string;
  creator:string; roomId:string; platform:string; recordedAt:string; sessionTitle:string; part:string; sourceUrl:string;
  labels:string[]; evidence:Evidence[]; issues:WorkflowIssue[]; originalTitle:string; localAssetId?:string; playlistId?:string;
  recordedEnd:string; technicalFields:string[]; durationSeconds:number|null; defaultLanguage:string; defaultAudioLanguage:string;
  copyrightStatus:'unknown'|'clear'|'claim'|'strike'; thumbnailCandidate?:{assetId:string;seconds:number;status:'review'};
  aiProposal?:{status:'awaiting_agent';task:string;input:Record<string,unknown>;outputSchema:Record<string,unknown>}; raw:Record<string,any>;
}
export interface IdentityBinding { platform:string; roomId:string; creator:string; playlistId?:string; aliases?:string[] }
export interface PlaylistInfo { id:string; title?:string; description?:string; snippet?:{title?:string;description?:string}; video_ids?:string[] }
export interface PlaylistIntent { action:'add'|'create'|'review'; playlistId?:string; identityKey:string; title:string; description:string; reason:string }
export interface WorkflowContext {
  identityBindings?:IdentityBinding[];
  identityLedger?:IdentityBinding[];
  playlists?:PlaylistInfo[];
  records?:NormalizedRecord[];
  copyrightReviews?:Record<string,{status:'unknown'|'clear'|'claim'|'strike';note?:string}>;
}
export interface NodeTrace { nodeId:string; status:'done'|'skipped'|'review'|'blocked'; message:string }
export interface RecordChange { field:string; before:unknown; after:unknown }
export interface RecordPreview { id:string; before:NormalizedRecord; after:NormalizedRecord; changes:RecordChange[]; trace:NodeTrace[]; playlistIntents:PlaylistIntent[]; intents:HostIntent[]; issues:WorkflowIssue[]; status:'ready'|'review'|'unchanged'|'skipped'|'blocked' }
export interface WorkflowResult { valid:boolean; errors:string[]; records:RecordPreview[]; summary:{total:number;ready:number;review:number;blocked:number;changed:number;skipped:number} }

const option = (value:string,label:string) => ({value,label});

const BUILTIN_SPECS: Array<NodeDefinition & { id:string; capabilities:string[] }> = [
  {id:'com.u2bup.builtin.input',type:'input',label:'素材输入',description:'将所选频道视频和本地素材转为统一记录；按真实 ID 去重。',group:'流转',color:'#7b8cff',defaults:{kind:'all'},capabilities:['read.record'],fields:[{key:'kind',label:'接收素材',type:'select',options:[option('all','全部所选素材'),option('youtube','仅已上传视频'),option('local','仅本地素材')]}]},
  {id:'com.u2bup.builtin.parse',type:'parse',label:'历史标题解析',description:'提取主播、平台、房间、录像日期、场次和明确分片，保留证据及不确定项。',group:'理解',color:'#8f86ff',defaults:{removeTechnical:false},capabilities:['read.record','write.fields:creator,roomId,platform,recordedAt,sessionTitle'],fields:[{key:'removeTechnical',label:'从主题移出明确技术后缀',type:'checkbox',help:'仅处理 flv / merged / 合并 / 8 位哈希；rN_M 含义未验证，始终保留。'}]},
  {id:'com.u2bup.builtin.identity',type:'identity',label:'来源身份台账',description:'按房间号匹配已确认身份与别名；冲突进入复核，不因裸数字断定平台。',group:'理解',color:'#7aa2ff',defaults:{requireConfirmed:true},capabilities:['read.record','write.fields:creator,platform,roomId','emit.intent:identity.upsert'],fields:[{key:'requireConfirmed',label:'仅应用台账中的已确认身份',type:'checkbox',help:'关闭后仍要求房间号一致，但允许把台账创建者写入候选。'}]},
  {id:'com.u2bup.builtin.classify',type:'classify',label:'来源与内容标签',description:'Bilibili直播、Twitch 与 Dance / ASMR / VTuber / 游戏 / Cosplay 多标签识别。',group:'理解',color:'#bc82ec',defaults:{customKeywords:'',customLabel:''},capabilities:['read.record','write.fields:labels'],fields:[{key:'customKeywords',label:'自定义关键词（逗号分隔）',type:'text'},{key:'customLabel',label:'命中后附加的内容标签',type:'text'}]},
  {id:'com.u2bup.builtin.filter',type:'filter',label:'条件分流',description:'符合条件走 yes，其他记录走 no；可连接汇合模块。',group:'流转',color:'#e4b45a',defaults:{field:'description',operator:'empty',value:''},capabilities:['read.record'],fields:[{key:'field',label:'筛选字段',type:'select',options:['kind','title','description','tags','labels','privacy','platform','creator','roomId','recordedAt','categoryId','durationSeconds','copyrightStatus','defaultLanguage','defaultAudioLanguage'].map(x=>option(x,({kind:'素材类型',title:'标题',description:'描述',tags:'YouTube 标签',labels:'内容标签',privacy:'可见性',platform:'平台',creator:'主播',roomId:'房间号',recordedAt:'录像日期',categoryId:'YouTube 分类',durationSeconds:'时长（秒）',copyrightStatus:'版权复核状态',defaultLanguage:'标题语言',defaultAudioLanguage:'音频语言'} as Record<string,string>)[x]))},{key:'operator',label:'条件',type:'select',options:[option('empty','为空'),option('notEmpty','非空'),option('contains','包含'),option('equals','等于'),option('notEquals','不等于'),option('lte','小于等于'),option('gte','大于等于')]},{key:'value',label:'比较值',type:'text'}]},
  {id:'com.u2bup.builtin.metadata',type:'metadata',label:'标题与描述模板',description:'以【主播】主题 分片统一命名，日期、场次、房间移入可重复更新的描述区块。',group:'修改',color:'#65b7ca',defaults:{titleTemplate:'【{主播}】{主题} {分片}',writeTitle:true,writeDescription:true,onlyEmptyDescription:false,allowUnconfirmed:false,footer:''},capabilities:['read.record','write.fields:title,description'],fields:[{key:'titleTemplate',label:'标题模板',type:'text',help:'支持 {主播}、{主题}、{分片}、{日期}、{平台}、{房间号}。'},{key:'writeTitle',label:'生成标题',type:'checkbox'},{key:'writeDescription',label:'补充结构化描述',type:'checkbox'},{key:'onlyEmptyDescription',label:'只填空描述',type:'checkbox'},{key:'allowUnconfirmed',label:'允许候选主播名进入改名预览',type:'checkbox',help:'仍需复核；默认只使用已绑定或明确括号名称。'},{key:'footer',label:'统一描述尾注',type:'textarea'}]},
  {id:'com.u2bup.builtin.tags',type:'tags',label:'标签合并去重',description:'将来源、内容分类及自定义标签合并到现有标签，保留原有标签。',group:'修改',color:'#63b9a0',defaults:{includeLabels:true,includeCreator:true,extraTags:''},capabilities:['read.record','write.fields:tags'],fields:[{key:'includeLabels',label:'加入来源与内容标签',type:'checkbox'},{key:'includeCreator',label:'加入已识别主播名',type:'checkbox'},{key:'extraTags',label:'补充标签（逗号或换行分隔）',type:'textarea'}]},
  {id:'com.u2bup.builtin.playlist',type:'playlist',label:'主播播放列表',description:'按平台 + 房间身份匹配列表；主播名作列表名，房间与来源写入列表说明。',group:'修改',color:'#d89b72',defaults:{playlistId:'',allowCreate:false,footer:''},capabilities:['read.record','emit.intent:playlist.add'],fields:[{key:'playlistId',label:'明确绑定的播放列表 ID',type:'text',help:'留空时仅匹配有平台与完整房间证据的列表；名称相似不自动绑定。'},{key:'allowCreate',label:'找不到时提出新建列表建议',type:'checkbox'},{key:'footer',label:'列表说明尾注',type:'textarea'}]},
  {id:'com.u2bup.builtin.quality',type:'quality',label:'质量与补档对账',description:'极短视频、同标题与同场次仅进入候选核对；不判断重复上传，不删除。',group:'复核',color:'#ddbf67',defaults:{shortSeconds:10,checkDuplicates:true,checkMissing:true},capabilities:['read.record'],fields:[{key:'shortSeconds',label:'极短视频阈值（秒）',type:'number'},{key:'checkDuplicates',label:'检查同标题 / 同来源日期候选',type:'checkbox'},{key:'checkMissing',label:'标出空描述、标签、语言',type:'checkbox'}]},
  {id:'com.u2bup.builtin.copyright',type:'copyright',label:'版权与地区复核',description:'人工 claim / strike 标记进入复核；地区限制单独显示，licensedContent 不视为版权警告。',group:'复核',color:'#df868a',defaults:{reviewRegions:true,holdUnknown:false},capabilities:['read.record','write.fields:copyrightStatus'],fields:[{key:'reviewRegions',label:'有地区限制时转人工复核',type:'checkbox'},{key:'holdUnknown',label:'版权状态未知时也暂停应用',type:'checkbox',help:'普通视频接口不提供完整 Content ID 申诉或版权警告；未导入人工状态时保持未知。'}]},
  {id:'com.u2bup.builtin.thumbnail',type:'thumbnail',label:'当前帧封面候选',description:'为已关联本地素材记录选帧时间；实际截图与应用由素材播放器和确认任务处理。',group:'修改',color:'#b383d7',defaults:{seconds:0},capabilities:['read.record','emit.intent:thumbnail.setFromLocalFrame'],fields:[{key:'seconds',label:'选定帧时间（秒）',type:'number',help:'必须有本地源素材；此模块仅保存候选，不推测远端视频画面。'}]},
  {id:'com.u2bup.builtin.ai',type:'ai',label:'AI Agent 提案接口',description:'输出有来源的结构化输入与提案 schema；未接入 Agent 时不伪造理解或爆款标题。',group:'理解',color:'#ab8ef3',defaults:{task:'根据视频信息提出准确、自然的标题和内容标签，引用来源证据，不捏造视频情节。'},capabilities:['read.record','emit.intent:agent.task'],fields:[{key:'task',label:'Agent 任务',type:'textarea'}]},
  {id:'com.u2bup.builtin.join',type:'join',label:'分支汇合',description:'合并实际到达的分支；同一字段的并行冲突会阻止该条应用。',group:'流转',color:'#8194a5',defaults:{},capabilities:['read.record'],fields:[]},
  {id:'com.u2bup.builtin.output',type:'output',label:'变更预览',description:'逐条展示前后值、节点轨迹及复核项；执行由独立确认任务负责。',group:'流转',color:'#70bfa4',defaults:{},capabilities:['read.record'],fields:[]},
];

function catalogFromRegistry(): NodeDefinition[] {
  return listEnabledCatalog().map((m) => ({
    type: m.type,
    label: m.label,
    description: m.description,
    group: m.group,
    color: m.color,
    fields: m.fields,
    defaults: m.defaults,
  }));
}

/** Enabled modules for the canvas palette (registry-driven). */
export function getNodeCatalog(): NodeDefinition[] {
  return catalogFromRegistry();
}

/** @deprecated Use getNodeCatalog(); kept as a live view for existing imports. */
export const NODE_CATALOG: NodeDefinition[] = new Proxy([] as NodeDefinition[], {
  get(_target, prop, receiver) {
    const list = catalogFromRegistry();
    if (prop === 'length') return list.length;
    if (prop === Symbol.iterator) return list[Symbol.iterator].bind(list);
    if (typeof prop === 'string' && /^\d+$/.test(prop)) return list[Number(prop)];
    const value = Reflect.get(list, prop, receiver);
    return typeof value === 'function' ? value.bind(list) : value;
  },
});

export const WORKFLOW_TEMPLATES = [
  {key:'standard',name:'历史视频标准化',description:'解析 → 身份台账 → 来源分类 → 标题描述 → 标签 → 播放列表 → 复核'},
  {key:'fill-empty',name:'空描述与标签补全',description:'仅为空描述的记录生成描述与标签，其他记录保持原状'},
  {key:'library',name:'素材入库与补档核对',description:'统一理解本地与云端素材，核对场次候选并预留封面'},
  {key:'copyright',name:'版权与地区例外',description:'检查版权人工标记、地区限制与极短素材'},
];

export function createNode(type:NodeType,index=0):WorkflowNode {
  const definition=getModuleDefinition(type)??BUILTIN_SPECS.find(s=>s.type===type||s.id===type);
  if(!definition) throw new Error(`未知模块：${type}`);
  const shortType=('type' in definition?definition.type:getModuleDefinition(type)?.type)||type;
  const defaults='defaults' in definition?definition.defaults:{};
  return {id:`${shortType}-${index}`,type:shortType,enabled:true,position:{x:80+(index%4)*290,y:90+Math.floor(index/4)*170},config:clone(defaults)};
}

export function createTemplate(key='standard'):WorkflowGraph {
  const template=WORKFLOW_TEMPLATES.find(t=>t.key===key);
  if(!template) throw new Error(`未知管线模板：${key}`);
  const types:NodeType[]=key==='standard'?['input','parse','identity','classify','metadata','tags','playlist','quality','copyright','output']:key==='fill-empty'?['input','parse','classify','filter','metadata','tags','join','output']:key==='library'?['input','parse','identity','classify','quality','thumbnail','output']:['input','copyright','quality','output'];
  const nodes=types.map((type,index)=>createNode(type,index));
  const edges:WorkflowEdge[]=nodes.slice(1).map((node,index)=>({id:`edge-${index}`,source:nodes[index].id,target:node.id,port:nodes[index].type==='filter'?'yes':'out'}));
  if(key==='fill-empty') {
    nodes[4].config.writeTitle=false;
    edges.push({id:'edge-no',source:nodes[3].id,target:nodes[6].id,port:'no'});
    nodes[4].position={x:1240,y:90}; nodes[5].position={x:1530,y:90}; nodes[6].position={x:1820,y:190}; nodes[7].position={x:2110,y:190};
  }
  return {version:1,moduleApi:1,id:`workflow-${key}`,name:template.name,nodes,edges};
}

const clone=<T>(value:T):T=>JSON.parse(JSON.stringify(value));
const str=(value:unknown)=>typeof value==='string'?value:typeof value==='number'?String(value):'';
const strings=(value:unknown):string[]=>Array.isArray(value)?value.filter((x):x is string=>typeof x==='string'):[];
const unique=(values:string[])=>[...new Map(values.map(x=>[x.trim().toLocaleLowerCase(),x.trim()])).values()].filter(Boolean);
const equal=(a:unknown,b:unknown)=>JSON.stringify(a)===JSON.stringify(b);
const asObject=(value:unknown):Record<string,any>=>value&&typeof value==='object'&&!Array.isArray(value)?value as Record<string,any>:{};
const initial=(id:string,kind:'youtube'|'local',raw:Record<string,any>):NormalizedRecord=>({id,kind,title:'',description:'',tags:[],privacy:'private',categoryId:'22',creator:'',roomId:'',platform:'unknown',recordedAt:'',recordedEnd:'',sessionTitle:'',part:'',sourceUrl:'',labels:[],evidence:[],issues:[],originalTitle:'',technicalFields:[],durationSeconds:null,defaultLanguage:'',defaultAudioLanguage:'',copyrightStatus:'unknown',raw:clone(raw)});

export function normalizeYoutube(input:unknown):NormalizedRecord {
  const raw=asObject(input),snippet=asObject(raw.snippet),status=asObject(raw.status),content=asObject(raw.contentDetails);
  const record=initial(str(raw.id),'youtube',raw);
  Object.assign(record,{title:str(snippet.title??raw.title),description:str(snippet.description??raw.description),tags:strings(snippet.tags??raw.tags),privacy:str(status.privacyStatus??raw.privacy)||'private',categoryId:str(snippet.categoryId??raw.categoryId)||'22',defaultLanguage:str(snippet.defaultLanguage),defaultAudioLanguage:str(snippet.defaultAudioLanguage),durationSeconds:duration(content.duration??raw.durationSeconds)});
  record.originalTitle=record.title;
  applyStoredMetadata(record,asObject(raw.workflow_metadata??raw.pipeline_metadata));
  return record;
}

export function normalizeAsset(input:Asset|unknown):NormalizedRecord {
  const raw=asObject(input),record=initial(str(raw.id),'local',raw),meta=asObject(raw.metadata);
  Object.assign(record,{title:str(raw.display_title||raw.title||raw.name),creator:str(raw.room_name),roomId:str(raw.room_id),recordedAt:str(raw.started_at),localAssetId:str(raw.id),platform:str(raw.platform)||'unknown',durationSeconds:duration(meta.duration)});
  record.originalTitle=record.title;
  for(const field of ['creator','roomId','recordedAt'] as const) if(record[field]) record.evidence.push({field,value:record[field],source:`素材库.${({creator:'room_name',roomId:'room_id',recordedAt:'started_at'} as const)[field]}`,confidence:'high'});
  for(const message of strings(raw.warnings)) record.issues.push({code:'asset_warning',severity:'review',message});
  applyStoredMetadata(record,asObject(raw.workflow_metadata??raw.pipeline_metadata));
  return record;
}

function applyStoredMetadata(record:NormalizedRecord,meta:Record<string,any>) {
  for(const field of ['creator','roomId','recordedAt','recordedEnd','sessionTitle','part','sourceUrl','originalTitle','localAssetId','playlistId'] as const) if(typeof meta[field]==='string') record[field]=meta[field];
  if(['bilibili','twitch','unknown'].includes(meta.platform)) record.platform=meta.platform;
  if(Array.isArray(meta.labels)) record.labels=strings(meta.labels);
  if(['unknown','clear','claim','strike'].includes(meta.copyrightStatus)) record.copyrightStatus=meta.copyrightStatus;
}

function duration(value:unknown):number|null {
  if(typeof value==='number'&&Number.isFinite(value)&&value>=0) return value;
  const match=str(value).match(/^PT(?:(\d+(?:\.\d+)?)H)?(?:(\d+(?:\.\d+)?)M)?(?:(\d+(?:\.\d+)?)S)?$/);
  return match?[Number(match[1]||0)*3600,Number(match[2]||0)*60,Number(match[3]||0)].reduce((a,b)=>a+b,0):null;
}

export function parseWorkflowGraph(value:unknown):WorkflowGraph {
  let graph:unknown=value;
  if(typeof value==='string') { try { graph=JSON.parse(value); } catch { throw new Error('管线文件不是有效 JSON'); } }
  const result=validateGraph(graph);
  if(!result.valid) throw new Error(result.errors.join('；'));
  return clone(graph as WorkflowGraph);
}

export function validateGraph(input:unknown):{valid:boolean;errors:string[];order:string[]} {
  const errors:string[]=[],graph=asObject(input);
  if(graph.version!==1) errors.push('仅支持 version: 1 的管线');
  if(typeof graph.id!=='string'||!graph.id.trim()||typeof graph.name!=='string'||!graph.name.trim()) errors.push('管线必须有 ID 和名称');
  if(!Array.isArray(graph.nodes)||!Array.isArray(graph.edges)) return {valid:false,errors:[...errors,'nodes 和 edges 必须为数组'],order:[]};
  if(graph.nodes.length>200||graph.edges.length>600) return {valid:false,errors:[...errors,'管线超过 200 个模块或 600 条连线'],order:[]};
  ensureBuiltinsRegistered();
  const nodes=graph.nodes as WorkflowNode[],edges=graph.edges as WorkflowEdge[],ids=new Set<string>(),edgeIds=new Set<string>();
  for(const node of nodes) {
    if(!node||typeof node.id!=='string'||!node.id||ids.has(node.id)) { errors.push('模块 ID 缺失或重复'); continue; }
    ids.add(node.id);
    const definition=lookupDefinition(node.type);
    if(!definition) errors.push(`未知模块类型：${node.type}`);
    else if(!isModuleEnabled(node.type)) errors.push(`模块已禁用：${node.type}`);
    if(typeof node.enabled!=='boolean') errors.push(`${node.id} 缺少 enabled 布尔值`);
    if(!node.position||!Number.isFinite(node.position.x)||!Number.isFinite(node.position.y)) errors.push(`${node.id} 的位置无效`);
    if(!node.config||typeof node.config!=='object'||Array.isArray(node.config)) errors.push(`${node.id} 的配置无效`);
    else if(definition) for(const [key,value] of Object.entries(node.config)) {
      const field=definition.fields.find(f=>f.key===key);
      if(!field) {errors.push(`${node.id} 包含未知配置 ${key}`);continue;}
      if(field.type==='checkbox'&&typeof value!=='boolean') errors.push(`${node.id}.${key} 应为布尔值`);
      if(field.type==='number'&&(typeof value!=='number'||!Number.isFinite(value)||value<0)) errors.push(`${node.id}.${key} 应为非负数字`);
      if(['text','textarea','select'].includes(field.type)&&(typeof value!=='string'||value.length>10000)) errors.push(`${node.id}.${key} 应为有效文本`);
      if(field.type==='select'&&!field.options?.some(o=>o.value===value)) errors.push(`${node.id}.${key} 选项无效`);
    }
  }
  const byId=new Map(nodes.filter(Boolean).map(n=>[n.id,n]));
  const incoming=new Map(nodes.filter(Boolean).map(n=>[n.id,[] as WorkflowEdge[]])),outgoing=new Map(nodes.filter(Boolean).map(n=>[n.id,[] as WorkflowEdge[]]));
  for(const edge of edges) {
    if(!edge||typeof edge.id!=='string'||!edge.id||edgeIds.has(edge.id)) {errors.push('连线 ID 缺失或重复');continue;}
    edgeIds.add(edge.id);
    if(!ids.has(edge.source)||!ids.has(edge.target)) {errors.push(`${edge.id} 指向不存在的模块`);continue;}
    if(edge.port!==undefined&&!['out','yes','no'].includes(edge.port)) errors.push(`${edge.id} 的输出端口无效`);
    const source=byId.get(edge.source)!;
    if(canonicalType(source.type)==='filter'&&!['yes','no'].includes(edge.port||'')) errors.push(`${edge.id} 必须选择条件分流的 yes 或 no 端口`);
    if(canonicalType(source.type)!=='filter'&&edge.port&&edge.port!=='out') errors.push(`${edge.id} 的源模块不支持条件端口`);
    incoming.get(edge.target)!.push(edge); outgoing.get(edge.source)!.push(edge);
  }
  const inputs=nodes.filter(n=>canonicalType(n?.type)==='input'),outputs=nodes.filter(n=>canonicalType(n?.type)==='output');
  if(inputs.length!==1||outputs.length!==1) errors.push('管线需要且只能有一个素材输入和一个变更预览');
  if(inputs.some(n=>incoming.get(n.id)?.length)||outputs.some(n=>outgoing.get(n.id)?.length)) errors.push('输入模块不能有入线，预览模块不能有出线');
  for(const node of nodes.filter(Boolean)) if((incoming.get(node.id)?.length||0)>1&&!['join','output'].includes(canonicalType(node.type))) errors.push(`${node.id} 有多个入口，请先用分支汇合模块合并`);
  const degree=new Map([...incoming].map(([id,list])=>[id,list.length])),queue=[...degree].filter(([,n])=>n===0).map(([id])=>id).sort(),order:string[]=[];
  while(queue.length) {const id=queue.shift()!;order.push(id);for(const edge of outgoing.get(id)||[]){degree.set(edge.target,degree.get(edge.target)!-1);if(degree.get(edge.target)===0){queue.push(edge.target);queue.sort();}}}
  if(order.length!==ids.size) errors.push('管线存在循环连线');
  if(inputs.length===1&&outputs.length===1) {
    const reachable=new Set<string>();const visit=(id:string,map:Map<string,WorkflowEdge[]>,reverse=false)=>{if(reachable.has(id))return;reachable.add(id);for(const e of map.get(id)||[])visit(reverse?e.source:e.target,map,reverse);};
    visit(inputs[0].id,outgoing);for(const id of ids) if(!reachable.has(id)) errors.push(`${id} 未连接到素材输入`);
    reachable.clear();visit(outputs[0].id,incoming,true);for(const id of ids)if(!reachable.has(id))errors.push(`${id} 无法到达变更预览`);
    const ancestors=new Map<string,Set<string>>();for(const id of order){const set=new Set<string>();for(const edge of incoming.get(id)||[]){set.add(edge.source);for(const a of ancestors.get(edge.source)||[])set.add(a);}ancestors.set(id,set);}
    const parsed=new Map<string,boolean>();for(const id of order){const node=byId.get(id)!;const ins=incoming.get(id)||[];const before=ins.length>0&&ins.every(e=>parsed.get(e.source));parsed.set(id,(canonicalType(node.type)==='parse'&&node.enabled)||before);if(node.enabled&&['metadata','playlist'].includes(canonicalType(node.type))&&!before)errors.push(`${id} 需要先经过已启用的历史标题解析模块`);}
  }
  return {valid:errors.length===0,errors:unique(errors),order};
}

function lookupDefinition(type:string):NodeDefinition|undefined {
  const fromRegistry=getModuleDefinition(type);
  if(fromRegistry) return {type:fromRegistry.type,label:fromRegistry.label,description:fromRegistry.description,group:fromRegistry.group,color:fromRegistry.color,fields:fromRegistry.fields,defaults:fromRegistry.defaults};
  const builtin=BUILTIN_SPECS.find(s=>s.type===type||s.id===type);
  return builtin?{type:builtin.type,label:builtin.label,description:builtin.description,group:builtin.group,color:builtin.color,fields:builtin.fields,defaults:builtin.defaults}:undefined;
}

function canonicalType(type:string):string {
  const resolved=resolveModuleType(type);
  if(resolved?.alias) return resolved.alias;
  const builtin=BUILTIN_SPECS.find(s=>s.id===type||s.type===type);
  return builtin?.type??type;
}

function addIssue(record:NormalizedRecord,code:string,message:string,severity:WorkflowIssue['severity']='review',nodeId?:string) {
  if(!record.issues.some(i=>i.code===code&&i.message===message)) record.issues.push({code,message,severity,...(nodeId?{nodeId}:{})});
}
function evidence(record:NormalizedRecord,field:string,value:string,source:string,confidence:Evidence['confidence']) {
  if(!record.evidence.some(e=>e.field===field&&e.value===value&&e.source===source)) record.evidence.push({field,value,source,confidence});
}
const escapeRegExp=(value:string)=>value.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
const datePattern=/(?:19|20)\d{2}[ _./-]?(?:0[1-9]|1[0-2])[ _./-]?(?:0[1-9]|[12]\d|3[01])/g;
const validDate=(value:string)=>{const digits=value.replace(/\D/g,'');const result=`${digits.slice(0,4)}-${digits.slice(4,6)}-${digits.slice(6,8)}`;const date=new Date(`${result}T00:00:00Z`);return !Number.isNaN(date.getTime())&&date.toISOString().slice(0,10)===result?result:'';};

function parseRecord(record:NormalizedRecord,config:Record<string,unknown>,context:WorkflowContext,nodeId:string) {
  const title=record.title,description=record.description,text=`${title}\n${description}`;
  const manuallyConfirmedCreator=record.creator&&record.evidence.some(e=>e.field==='creator'&&e.value===record.creator&&e.confidence==='high'&&e.source.startsWith('人工校正'))?record.creator:'';
  const managed=description.match(/\[U2BUP 元信息\]\n([\s\S]*?)\n\[\/U2BUP 元信息\]/)?.[1];
  if(managed) for(const [key,field] of Object.entries({主播:'creator',平台:'platform',房间号:'roomId',录像时间:'recordedAt',录像结束:'recordedEnd',场次主题:'sessionTitle',分片:'part',来源:'sourceUrl',原始标题:'originalTitle'})) {
    const value=managed.match(new RegExp(`^${key}：(.+)$`,'m'))?.[1];
    if(value&&(!str((record as any)[field])||field==='originalTitle'||field==='platform'&&record.platform==='unknown')) (record as any)[field]=value;
  }
  const bilibili=text.match(/https?:\/\/live\.bilibili\.com\/(\d+)(?:[/?#\s]|$)/i),twitch=text.match(/https?:\/\/(?:www\.)?twitch\.tv\/([a-zA-Z0-9_]+)(?:[/?#\s]|$)/i);
  const platformCandidates=unique([...(bilibili||/\bBilibili\b|哔哩哔哩|B站/i.test(text)?['bilibili']:[]),...(twitch||/\bTwitch\b/i.test(text)?['twitch']:[])]);
  if(platformCandidates.length===1&&(record.platform==='unknown'||record.platform===platformCandidates[0])) {record.platform=platformCandidates[0];evidence(record,'platform',record.platform,'标题 / 描述中的明确平台名称或 URL','high');}
  else if(platformCandidates.length>1||platformCandidates.length===1&&record.platform!=='unknown'&&record.platform!==platformCandidates[0]) addIssue(record,'platform_conflict','检测到多个来源平台，保留原身份并转人工复核','review',nodeId);
  if(bilibili&&!record.sourceUrl)record.sourceUrl=bilibili[0].trim().replace(/[?#].*$/,'');
  if(twitch&&!record.sourceUrl)record.sourceUrl=twitch[0].trim().replace(/[?#].*$/,'');
  const roomPrefix=title.match(/^(?:【[^】]+】\s*)?(?:录制[ _]*)?(\d{2,12})[ _]+(?=(?:19|20)\d{2}[ _./-]?(?:0[1-9]|1[0-2]))/);
  const explicitRoom=text.match(/(?:房间号|直播间(?:ID|号)?|room(?:\s*id)?)[：:\s_]*(\d{2,12})/i);
  const descriptionRoom=description.match(/^\s*(\d{2,12})(?:\s|$)/);
  const candidates=unique([bilibili?.[1]||'',twitch?.[1]?.toLowerCase()||'',roomPrefix?.[1]||'',explicitRoom?.[1]||'',descriptionRoom?.[1]||'']);
  for(const value of candidates)evidence(record,'roomId',value,bilibili?.[1]===value||twitch?.[1]?.toLowerCase()===value?'原直播间 URL':'录像标题 / 描述房间号候选',bilibili?.[1]===value||twitch?.[1]?.toLowerCase()===value?'high':'medium');
  if(candidates.length===1&&!record.roomId)record.roomId=candidates[0];
  if(candidates.length>1||record.roomId&&candidates.some(x=>x!==record.roomId))addIssue(record,'room_conflict','存在多个房间号候选，不能自动关联主播或播放列表','review',nodeId);
  const bindings=(context.identityBindings||[]).filter(b=>record.roomId&&b.roomId===record.roomId&&(record.platform==='unknown'||record.platform===b.platform));
  const identities=unique(bindings.map(b=>`${b.platform}:${b.roomId}:${b.creator}`));
  if(identities.length===1&&!record.issues.some(i=>['room_conflict','platform_conflict'].includes(i.code))) {
    const binding=bindings[0];record.platform=binding.platform;record.creator=manuallyConfirmedCreator||binding.creator;
    if(record.creator===binding.creator)evidence(record,'creator',record.creator,'已确认的来源身份台账','high');
    evidence(record,'platform',record.platform,'已确认的平台 + 房间绑定','high');
  } else if(identities.length>1)addIssue(record,'identity_conflict','同一房间命中多个来源身份，需要确认平台与主播','review',nodeId);
  const bracket=title.match(/^\s*【([^】]{1,80})】/);
  if(bracket&&!record.creator) {record.creator=bracket[1].trim();evidence(record,'creator',record.creator,'标题开头的【主播】候选','medium');}
  else if(bracket&&record.creator!==bracket[1].trim()) {
    if(manuallyConfirmedCreator)evidence(record,'originalCreator',bracket[1].trim(),'原标题【主播】；已由人工校正覆盖','medium');
    else if(!bindings.some(b=>b.aliases?.includes(bracket[1].trim())))addIssue(record,'creator_conflict','标题主播名称与身份台账不一致，请确认是否为别名','review',nodeId);
  }
  const dates=[...title.matchAll(datePattern)].filter(m=>!/[0-9]/.test(title[(m.index||0)-1]||'')&&!/[0-9]/.test(title[(m.index||0)+m[0].length]||'')).map(m=>({raw:m[0],value:validDate(m[0]),index:m.index||0})).filter(d=>d.value);
  if(dates.length) {
    if(!record.recordedAt)record.recordedAt=dates[0].value;
    for(const date of dates)evidence(record,'recordedAt',date.value,'标题中的有效日历日期（不等于上传时间）','medium');
    if(dates.length>1) {record.recordedEnd=dates[dates.length-1].value;addIssue(record,'multiple_dates','标题含多个日期，已保留起止候选；场次范围需核对','review',nodeId);}
    const rest=title.slice(dates[0].index+dates[0].raw.length),time=rest.match(/^[ _]+([01]\d|2[0-3])([0-5]\d)([0-5]\d)(?:[ _]+\d{3})?(?=[ _]|$)/);
    if(time&&record.recordedAt.length===10) {record.recordedAt+=` ${time[1]}:${time[2]}:${time[3]}`;evidence(record,'recordedAt',record.recordedAt,'标题紧随日期的 HHMMSS；时区未知','medium');}
    if(!record.creator) {
      const prefix=title.slice(0,dates[0].index).replace(/^录制[ _]*/,'').replace(/[ _]+$/,'').trim();
      if(prefix&&!/^\d+$/.test(prefix)&&prefix.length<=60&&!/[【】]/.test(prefix)) {record.creator=prefix;evidence(record,'creator',prefix,'日期之前的文本，可能混有主题','low');addIssue(record,'creator_candidate','日期前缀仅是主播名称候选，需与房间身份台账核对','review',nodeId);}
    }
  }
  const part=title.match(/(?:^|[\s_【[(])((?:[Pp](?:art)?[ _-]?\d+)|(?:第\s*\d+\s*[段集部分]))(?=$|[\s_】\])])/);
  if(part&&!record.part){record.part=part[1].replace(/^part[ _-]?/i,'P');evidence(record,'part',record.part,'明确 P / Part / 第N段 标记','high');}
  const technical=title.match(/(?:\bflv\b|\bmerged\b|合并|(?:^|[ _])r\d+[ _]\d+(?=$|[ _])|(?:^|[ _])[a-f0-9]{8}$|(?:^|[ _])(?:16[ _]9|9[ _]16)(?=$|[ _]))/gi)||[];
  record.technicalFields=unique([...record.technicalFields,...technical.map(t=>t.trim())]);
  if(technical.some(t=>/r\d+[ _]\d+/i.test(t)))addIssue(record,'unknown_technical_marker','rN_M 含义未验证，保留原文，不当作分片号','info',nodeId);
  if(!record.sessionTitle) {
    let topic=title;
    if(bracket)topic=topic.slice(bracket[0].length);
    else if(record.creator&&topic.startsWith(record.creator))topic=topic.slice(record.creator.length);
    topic=topic.replace(/^\s*录制[ _]*/,'');
    if(record.roomId)topic=topic.replace(new RegExp(`(^|[ _])${escapeRegExp(record.roomId)}(?=[ _]|$)`),' ');
    for(const date of dates) topic=topic.replace(date.raw,' ');
    if(dates.length)topic=topic.replace(/^[ _]+(?:[01]\d|2[0-3])[0-5]\d[0-5]\d(?:[ _]+\d{3})?(?=[ _]|$)/,' ');
    if(part)topic=topic.replace(part[1],' ');
    if(config.removeTechnical)topic=topic.replace(/\bflv\b|\bmerged\b|合并/gi,' ').replace(/(?:^|[ _])[a-f0-9]{8}$/i,' ');
    record.sessionTitle=topic.replace(/_/g,' ').replace(/\s+/g,' ').trim();
    if(record.sessionTitle)evidence(record,'sessionTitle',record.sessionTitle,'保留未解释文本后的场次主题','medium');
  }
  if(!record.creator)addIssue(record,'creator_unknown','主播未知，需绑定来源身份后再统一命名','info',nodeId);
  if(record.platform==='unknown')addIssue(record,'platform_unknown','没有可靠平台证据，未按录像命名推断 Bilibili 或 Twitch','info',nodeId);
}

function classify(record:NormalizedRecord,config:Record<string,unknown>) {
  const text=`${record.title}\n${record.description}\n${record.tags.join(' ')}`;
  const rules:[string,RegExp][]=[['Dance',/\bdance\b|跳舞|舞蹈|宅舞|热舞/i],['ASMR',/\basmr\b|助眠|耳语|掏耳/i],['VTuber',/\bvtuber\b|虚拟主播|虚拟偶像|\bvup\b/i],['游戏',/\bgaming\b|\bgameplay\b|游戏|原神|英雄联盟|绝区零|崩坏|星穹铁道|\bminecraft\b|\bvalorant\b/i],['Cosplay',/\bcosplay\b|\bcoser\b|角色扮演/i]];
  for(const [label,pattern] of rules)if(pattern.test(text)){record.labels=unique([...record.labels,label]);evidence(record,'labels',label,`标题 / 描述 / 标签关键词：${text.match(pattern)?.[0]}`,'medium');}
  if(record.platform==='bilibili')record.labels=unique([...record.labels,'Bilibili直播']);
  if(record.platform==='twitch')record.labels=unique([...record.labels,'Twitch']);
  const keywords=str(config.customKeywords).split(/[,，\n]/).map(x=>x.trim()).filter(Boolean),customLabel=str(config.customLabel).trim();
  if(customLabel&&keywords.some(keyword=>text.toLocaleLowerCase().includes(keyword.toLocaleLowerCase()))){record.labels=unique([...record.labels,customLabel]);evidence(record,'labels',customLabel,'用户配置的关键词规则','medium');}
  // categoryId 20 is not evidence that all of these recordings contain games.
}

function applyIdentity(record:NormalizedRecord,config:Record<string,unknown>,context:WorkflowContext,nodeId:string):HostIntent[] {
  const ledger=[...(context.identityLedger||[]),...(context.identityBindings||[])];
  if(!record.roomId){addIssue(record,'identity_room_missing','没有房间号，无法匹配来源身份台账','info',nodeId);return[];}
  const matches=ledger.filter(b=>b.roomId===record.roomId&&(record.platform==='unknown'||!b.platform||b.platform===record.platform||b.platform==='unknown'));
  const identities=unique(matches.map(b=>`${b.platform||'unknown'}:${b.roomId}:${b.creator}`));
  if(!matches.length){addIssue(record,'identity_unmatched','台账中没有匹配此房间号的来源身份','info',nodeId);return[];}
  if(identities.length>1){addIssue(record,'identity_conflict','同一房间命中多个来源身份，需要确认平台与主播','review',nodeId);return[];}
  const binding=matches[0]!;
  const requireConfirmed=config.requireConfirmed!==false;
  if(requireConfirmed&&record.platform==='unknown'&&!binding.platform){addIssue(record,'identity_platform_unknown','台账条目缺少平台，未自动写入','review',nodeId);return[];}
  if(binding.platform&&binding.platform!=='unknown'){record.platform=binding.platform;evidence(record,'platform',binding.platform,'来源身份台账','high');}
  if(binding.creator){
    const manuallyConfirmed=record.evidence.some(e=>e.field==='creator'&&e.value===record.creator&&e.confidence==='high'&&e.source.startsWith('人工校正'));
    if(!manuallyConfirmed){record.creator=binding.creator;evidence(record,'creator',binding.creator,'来源身份台账','high');}
  }
  if(binding.playlistId&&!record.playlistId)record.playlistId=binding.playlistId;
  for(const alias of binding.aliases||[])evidence(record,'creatorAlias',alias,'来源身份台账别名','medium');
  return[{kind:'identity.upsert',platform:record.platform,roomId:record.roomId,creator:record.creator,aliases:binding.aliases,playlistId:binding.playlistId,reason:'台账匹配后的来源身份'}];
}

function metadata(record:NormalizedRecord,config:Record<string,unknown>,nodeId:string) {
  const low=record.evidence.some(e=>e.field==='creator'&&e.value===record.creator&&e.confidence==='low')&&!record.evidence.some(e=>e.field==='creator'&&e.value===record.creator&&e.confidence!=='low');
  const conflict=record.issues.some(i=>['creator_conflict','identity_conflict','room_conflict','platform_conflict'].includes(i.code));
  if(config.writeTitle) {
    if(!record.creator||!record.sessionTitle||low&&!config.allowUnconfirmed||conflict)addIssue(record,'title_needs_identity','标题未改：需要无冲突的主播和场次主题；候选名称可在模块设置中允许预览','review',nodeId);
    else {
      const vars:Record<string,string>={'主播':record.creator,'主题':record.sessionTitle,'分片':record.part,'日期':record.recordedAt,'平台':record.platform==='unknown'?'':record.platform,'房间号':record.roomId};
      const template=str(config.titleTemplate),unknown=[...template.matchAll(/\{([^}]+)\}/g)].filter(m=>!(m[1] in vars));
      const title=template.replace(/\{([^}]+)\}/g,(_,key)=>vars[key]??'').replace(/\s+/g,' ').trim();
      if(unknown.length)addIssue(record,'template_unknown','标题模板含不支持的变量，未生成新标题','review',nodeId);
      else if(!title||[...title].length>100||/[<>]/.test(title))addIssue(record,'title_limit','新标题为空、超过 100 字符或含 < >；请调整模板','review',nodeId);
      else record.title=title;
    }
  }
  if(config.writeDescription&&(!config.onlyEmptyDescription||!record.description.trim())) {
    const lines=[['主播',record.creator],['平台',record.platform==='unknown'?'':record.platform],['录像时间',record.recordedAt],['录像结束',record.recordedEnd],['场次主题',record.sessionTitle],['房间号',record.roomId],['分片',record.part],['来源',record.sourceUrl],['技术标记',record.technicalFields.join(' / ')],['原始标题',record.originalTitle]].filter(([,v])=>v).map(([k,v])=>`${k}：${v}`);
    const footer=str(config.footer).trim();if(footer)lines.push(footer);
    const block=`[U2BUP 元信息]\n${lines.join('\n')}\n[/U2BUP 元信息]`;
    const existing=record.description.replace(/\n*\[U2BUP 元信息\]\n[\s\S]*?\n\[\/U2BUP 元信息\]\n*/g,'\n').trim();
    const next=[existing,block].filter(Boolean).join('\n\n');
    if([...next].length>5000)addIssue(record,'description_limit','描述超过 5000 字符，保留原描述并等待调整','review',nodeId);else record.description=next;
  }
}

function playlist(record:NormalizedRecord,config:Record<string,unknown>,context:WorkflowContext,nodeId:string):PlaylistIntent[] {
  const known=record.platform!=='unknown'&&Boolean(record.roomId),identityKey=known?`${record.platform}:${record.roomId}`:'',binding=(context.identityBindings||[]).find(b=>b.roomId===record.roomId&&b.platform===record.platform);
  const explicit=str(config.playlistId)||record.playlistId||binding?.playlistId||'';
  const title=record.creator,description=[record.platform==='unknown'?'':`平台：${record.platform}`,record.roomId?`房间号：${record.roomId}`:'',record.sourceUrl?`来源：${record.sourceUrl}`:'',str(config.footer)].filter(Boolean).join('\n');
  const review=(reason:string):PlaylistIntent[]=>{addIssue(record,'playlist_review',reason,'review',nodeId);return[{action:'review',identityKey,title,description,reason}];};
  if(explicit) {
    const target=(context.playlists||[]).find(p=>p.id===explicit);
    if(!record.creator)return review('已选择播放列表，但仍需确认主播名才能生成列表改名建议');
    return[{action:'add',playlistId:explicit,identityKey,title,description,reason:target?.video_ids?.includes(record.id)?'视频已在此列表；仅整理列表名称与说明，执行时不会重复加入':'使用明确绑定的播放列表；列表名称与说明变更仍需单独确认'}];
  }
  if(!known||!record.creator)return review('缺少平台、房间或主播身份；不按名称相似度自动归类');
  if(!['bilibili','twitch'].includes(record.platform))return review('此平台暂未提供房间身份匹配规则，请明确绑定播放列表');
  if(record.issues.some(i=>['room_conflict','identity_conflict','platform_conflict','creator_conflict'].includes(i.code)))return review('来源身份冲突，需要先确认房间与主播绑定');
  const roomPattern=new RegExp(`(^|[^\\p{L}\\p{N}_])${escapeRegExp(record.roomId)}(?=$|[^\\p{L}\\p{N}_])`,'iu');
  const matched=(context.playlists||[]).filter(p=>{const text=`${p.title||p.snippet?.title||''}\n${p.description||p.snippet?.description||''}`;return roomPattern.test(text)&&(record.platform==='bilibili'?/bilibili|哔哩哔哩|B站/i:/twitch/i).test(text);});
  if(matched.length>1)return review('多个播放列表同时包含相同平台和房间，需明确选择');
  if(matched.length===1) {
    return[{action:'add',playlistId:matched[0].id,identityKey,title,description,reason:matched[0].video_ids?.includes(record.id)?'视频已在此列表；仅整理列表名称与说明，执行时不会重复加入':'已有列表具有明确平台与完整房间号证据'}];
  }
  if(!config.allowCreate)return review('没有匹配列表；可明确绑定列表或启用新建建议');
  return[{action:'create',identityKey,title,description,reason:'建议按平台 + 房间创建主播播放列表；默认私密，执行前确认'}];
}

function matches(record:NormalizedRecord,config:Record<string,unknown>):boolean {
  const value=(record as any)[str(config.field)],needle=str(config.value),empty=value==null||value===''||Array.isArray(value)&&!value.length;
  switch(config.operator){case 'empty':return empty;case 'notEmpty':return !empty;case 'contains':return Array.isArray(value)?value.some(x=>str(x).toLocaleLowerCase().includes(needle.toLocaleLowerCase())):str(value).toLocaleLowerCase().includes(needle.toLocaleLowerCase());case 'equals':return Array.isArray(value)?value.includes(needle):str(value)===needle;case 'notEquals':return Array.isArray(value)?!value.includes(needle):str(value)!==needle;case 'lte':return value!==null&&str(value)!==''&&needle.trim()!==''&&Number.isFinite(Number(value))&&Number.isFinite(Number(needle))&&Number(value)<=Number(needle);case 'gte':return value!==null&&str(value)!==''&&needle.trim()!==''&&Number.isFinite(Number(value))&&Number.isFinite(Number(needle))&&Number(value)>=Number(needle);default:return false;}
}

function quality(record:NormalizedRecord,config:Record<string,unknown>,records:NormalizedRecord[],nodeId:string) {
  if(record.durationSeconds!==null&&record.durationSeconds<=Number(config.shortSeconds))addIssue(record,'short_video',`时长仅 ${record.durationSeconds} 秒，可能为中断片段，请核对源素材`,'review',nodeId);
  if(config.checkMissing) for(const [field,label] of [['description','描述'],['tags','标签'],['defaultLanguage','标题语言'],['defaultAudioLanguage','音频语言']] as const) if(!record[field].length)addIssue(record,`missing_${field}`,`${label}未填写；语言应由人工确认或可靠内容识别补全`,'info',nodeId);
  if(config.checkDuplicates) {
    const titleKey=(title:string)=>title.normalize('NFKC').replace(/[ _]+/g,' ').trim().toLocaleLowerCase();
    const candidates=records.filter(r=>!(r.id===record.id&&r.kind===record.kind)&&(titleKey(r.originalTitle||r.title)===titleKey(record.originalTitle||record.title)||record.platform!=='unknown'&&record.roomId&&record.recordedAt&&r.platform===record.platform&&r.roomId===record.roomId&&r.recordedAt.slice(0,10)===record.recordedAt.slice(0,10)));
    if(candidates.length)addIssue(record,'duplicate_candidate',`存在 ${candidates.length} 条同标题 / 同来源日期候选（${candidates.slice(0,6).map(r=>`${r.kind}:${r.id} · ${r.durationSeconds??'未知'}秒`).join('；')}）；可能为分片、画幅或补档，不能认定重复`,'review',nodeId);
  }
}

const changedFields=['title','description','tags','privacy','categoryId','creator','roomId','platform','recordedAt','recordedEnd','sessionTitle','part','sourceUrl','originalTitle','labels','technicalFields','defaultLanguage','defaultAudioLanguage','copyrightStatus','thumbnailCandidate','aiProposal'] as const;
interface Write {nodeId:string;value:unknown}
interface State {record:NormalizedRecord;writes:Map<string,Write[]>;intents:PlaylistIntent[];hostIntents:HostIntent[];route?:'yes'|'no';blocked:boolean}
function forkState(state:State):State {return{record:clone(state.record),writes:new Map([...state.writes].map(([k,v])=>[k,clone(v)])),intents:clone(state.intents),hostIntents:clone(state.hostIntents),blocked:state.blocked};}

export function runWorkflow(graph:WorkflowGraph,records:NormalizedRecord[],context:WorkflowContext={}):WorkflowResult {
  ensureBuiltinsRegistered();
  const validation=validateGraph(graph),summary={total:0,ready:0,review:0,blocked:0,changed:0,skipped:0};
  if(!validation.valid)return{valid:false,errors:validation.errors,records:[],summary};
  const byId=new Map(graph.nodes.map(n=>[n.id,n])),incoming=new Map(graph.nodes.map(n=>[n.id,graph.edges.filter(e=>e.target===n.id)]));
  const ancestors=new Map<string,Set<string>>();for(const id of validation.order){const set=new Set<string>();for(const edge of incoming.get(id)||[]){set.add(edge.source);for(const a of ancestors.get(edge.source)||[])set.add(a);}ancestors.set(id,set);}
  const uniqueRecords=[...new Map(records.map(r=>[`${r.kind}:${r.id}`,r])).values()];
  const comparisonRecords=(context.records||uniqueRecords).map(r=>{const next=clone(r);parseRecord(next,{removeTechnical:false},context,'comparison');return next;});
  const previews:RecordPreview[]=uniqueRecords.map(original=>{
    const before=clone(original),states=new Map<string,State|null>(),trace:NodeTrace[]=[];
    if(!before.id){addIssue(before,'record_id_missing','素材 ID 缺失，无法生成可应用变更','error');}
    for(const id of validation.order) {
      const node=byId.get(id)!,definition=lookupDefinition(node.type)!,config={...definition.defaults,...node.config};
      let state:State;
      const nodeKind=canonicalType(node.type);
      if(nodeKind==='input')state={record:clone(before),writes:new Map(),intents:[],hostIntents:[],blocked:!before.id};
      else {
        const upstream=(incoming.get(id)||[]).map(e=>{const s=states.get(e.source);return s&&(!s.route||s.route===e.port)?s:null;}).filter((s):s is State=>Boolean(s));
        if(!upstream.length){states.set(id,null);trace.push({nodeId:id,status:'skipped',message:'当前记录未到达此分支'});continue;}
        state=forkState(upstream[0]);
        if(upstream.length>1) {
          state.record=clone(before);state.writes=new Map();state.intents=[];state.hostIntents=[];state.blocked=upstream.some(s=>s.blocked);
          for(const source of upstream) {
            for(const item of source.record.evidence)evidence(state.record,item.field,item.value,item.source,item.confidence);
            for(const item of source.record.issues)addIssue(state.record,item.code,item.message,item.severity,item.nodeId);
            for(const [field,writes] of source.writes)state.writes.set(field,[...(state.writes.get(field)||[]),...writes].filter((w,i,all)=>all.findIndex(x=>x.nodeId===w.nodeId&&equal(x.value,w.value))===i));
            for(const intent of source.intents)if(!state.intents.some(i=>equal(i,intent)))state.intents.push(clone(intent));
            for(const intent of source.hostIntents)if(!state.hostIntents.some(i=>equal(i,intent)))state.hostIntents.push(clone(intent));
          }
          for(const [field,writes] of state.writes) {
            const last=writes.filter(w=>!writes.some(other=>other.nodeId!==w.nodeId&&ancestors.get(other.nodeId)?.has(w.nodeId)));
            if(last.some(w=>!equal(w.value,last[0].value))){state.blocked=true;addIssue(state.record,'branch_conflict',`并行分支对 ${field} 提出不同修改；请调整顺序或移除冲突`,'error',id);}
            else (state.record as any)[field]=clone(last[0].value);
          }
          const identities=unique(state.intents.map(i=>i.identityKey));for(const identity of identities){const intents=state.intents.filter(i=>i.identityKey===identity);if(intents.length>1&&!intents.every(i=>equal(i,intents[0]))){state.blocked=true;addIssue(state.record,'playlist_branch_conflict','并行分支提出不同的播放列表意图，请先合并为一个明确目标','error',id);}}
        }
      }
      if(!node.enabled){states.set(id,state);trace.push({nodeId:id,status:'skipped',message:'模块已停用，记录直接通过'});continue;}
      if(state.blocked){states.set(id,state);trace.push({nodeId:id,status:'blocked',message:'上游冲突或输入错误，暂停后续变更'});continue;}
      if(!isModuleEnabled(node.type)){state.blocked=true;addIssue(state.record,'module_disabled',`模块已禁用：${node.type}`,'error',id);states.set(id,state);trace.push({nodeId:id,status:'blocked',message:'模块未启用'});continue;}
      const start=clone(state.record),issuesBefore=state.record.issues.length;
      let message='已完成本地预览';
      switch(nodeKind) {
        case 'input': if(config.kind!=='all'&&config.kind!==state.record.kind){states.set(id,null);trace.push({nodeId:id,status:'skipped',message:'素材类型不匹配'});continue;}message='已接收统一素材记录';break;
        case 'parse':parseRecord(state.record,config,context,id);message='已解析字段并保留来源证据';break;
        case 'identity':{
          const produced=applyIdentity(state.record,config,context,id);
          state.hostIntents.push(...produced);
          message=produced.length?'已匹配来源身份台账':'未写入新的身份绑定';
          break;
        }
        case 'classify':classify(state.record,config);message=state.record.labels.length?state.record.labels.join(' · '):'没有明确关键词，未强行分类';break;
        case 'filter':state.route=matches(state.record,config)?'yes':'no';message=`分流 → ${state.route}`;break;
        case 'metadata':metadata(state.record,config,id);message='标题与描述模板预览';break;
        case 'tags':{
          const extra=str(config.extraTags).split(/[,，\n]/);const tags=unique([...state.record.tags,...(config.includeLabels?state.record.labels:[]),...(config.includeCreator&&state.record.creator?[state.record.creator]:[]),...extra]);
          const length=tags.reduce((sum,tag)=>sum+tag.length+(tag.includes(' ')?2:0),Math.max(0,tags.length-1));
          if(length>500)addIssue(state.record,'tags_limit','合并标签超过 500 字符预算，保留原标签，请减少补充标签','review',id);else state.record.tags=tags;
          message=`标签合并后 ${state.record.tags.length} 个`;break;
        }
        case 'playlist':{
          const produced=playlist(state.record,config,context,id);
          state.intents.push(...produced);
          state.hostIntents.push(...produced.map(hostIntentFromPlaylist));
          message=produced.at(-1)?.reason||'已在目标播放列表中，无需重复添加';break;
        }
        case 'quality':quality(state.record,config,comparisonRecords,id);message='已检查完整度、极短片段与补档候选';break;
        case 'copyright':{
          const review=context.copyrightReviews?.[state.record.id]||asObject(state.record.raw.copyrightReview),status=review.status;
          if(['clear','claim','strike','unknown'].includes(status))state.record.copyrightStatus=status;
          if(['claim','strike'].includes(state.record.copyrightStatus))addIssue(state.record,'copyright_review',`人工记录版权状态：${state.record.copyrightStatus}${review.note?`；${str(review.note)}`:''}。请在 Studio 查看并决定处置`,'review',id);
          if(state.record.copyrightStatus==='unknown')addIssue(state.record,'copyright_unknown','版权申诉 / 警告状态未知；普通视频元信息无法证明无版权问题',config.holdUnknown?'review':'info',id);
          const regions=asObject(asObject(state.record.raw.contentDetails).regionRestriction);
          if(Object.keys(regions).length)addIssue(state.record,'region_restriction',`地区播放限制：${strings(regions.blocked).length?`屏蔽 ${strings(regions.blocked).join(', ')}`:`仅允许 ${strings(regions.allowed).join(', ')||'未知区域'}`}。不据此推断版权申诉或警告`,config.reviewRegions?'review':'info',id);
          message=`版权状态：${state.record.copyrightStatus}；地区限制单独核对`;break;
        }
        case 'thumbnail':{
          const seconds=Number(config.seconds);
          if(!state.record.localAssetId)addIssue(state.record,'thumbnail_source_missing','尚未关联本地源素材，不能从元信息中选取视频帧','review',id);
          else if(state.record.durationSeconds!==null&&seconds>=state.record.durationSeconds)addIssue(state.record,'thumbnail_out_of_range','选帧时间必须小于素材时长','review',id);
          else {
            state.record.thumbnailCandidate={assetId:state.record.localAssetId,seconds,status:'review'};
            state.hostIntents.push({kind:'thumbnail.setFromLocalFrame',assetId:state.record.localAssetId,seconds,reason:'本地选帧封面候选'});
            addIssue(state.record,'thumbnail_review','已保存选帧候选；需要在素材播放器确认画面，再执行封面上传','review',id);
          }
          message='封面候选待确认';break;
        }
        case 'ai':{
          state.record.aiProposal={status:'awaiting_agent',task:str(config.task),input:{title:state.record.title,description:state.record.description,creator:state.record.creator,labels:state.record.labels,evidence:state.record.evidence,localAssetId:state.record.localAssetId||null},outputSchema:{type:'object',required:['title','summary','tags','evidence','confidence'],properties:{title:{type:'string',maxLength:100},summary:{type:'string'},tags:{type:'array',items:{type:'string'}},evidence:{type:'array',items:{type:'string'}},confidence:{type:'number',minimum:0,maximum:1}},additionalProperties:false}};
          state.hostIntents.push({kind:'agent.task',task:str(config.task),input:state.record.aiProposal.input,outputSchema:state.record.aiProposal.outputSchema,reason:'Agent 尚未接入'});
          addIssue(state.record,'agent_pending','Agent 尚未接入；仅生成结构化任务，没有生成标题或视频理解结论','review',id);message='结构化 Agent 输入已就绪，等待接入与人工审阅';break;
        }
        case 'join':message='已合并实际到达分支；未发现字段冲突';break;
        case 'output':message='本地预览完成，尚未执行远端修改';break;
        default: {
          const produced=runModuleHandler(node.type,state.record as any,config,context as any,id);
          state.hostIntents.push(...produced);
          for(const intent of produced){const playlistShape=playlistIntentFromHost(intent);if(playlistShape&&!state.intents.some(i=>equal(i,playlistShape)))state.intents.push(playlistShape);}
          message=produced.length?`声明式模块产出 ${produced.length} 条意图`:'声明式模块已应用';
          break;
        }
      }
      for(const field of changedFields)if(!equal(start[field],state.record[field]))state.writes.set(field,[...(state.writes.get(field)||[]),{nodeId:id,value:clone(state.record[field])}]);
      const newIssues=state.record.issues.slice(issuesBefore);trace.push({nodeId:id,status:newIssues.some(i=>i.severity==='error')?'blocked':newIssues.some(i=>i.severity==='review')?'review':'done',message});
      states.set(id,state);
    }
    const outputNode=graph.nodes.find(n=>canonicalType(n.type)==='output')!;
    const output=states.get(outputNode.id);
    const after=output?.record||clone(before),issues=after.issues,changes:RecordChange[]=changedFields.filter(field=>!equal(before[field],after[field])).map(field=>({field,before:before[field],after:after[field]}));
    const intents=output?.intents||[],hostIntents=output?.hostIntents||[],blocked=output?.blocked||issues.some(i=>i.severity==='error');
    const status:RecordPreview['status']=!output?'skipped':blocked?'blocked':issues.some(i=>i.severity==='review')?'review':changes.length||intents.length||hostIntents.length?'ready':'unchanged';
    return{id:before.id,before,after,changes,trace,playlistIntents:intents,intents:hostIntents,issues,status};
  });
  summary.total=previews.length;summary.ready=previews.filter(p=>p.status==='ready').length;summary.review=previews.filter(p=>p.status==='review').length;summary.blocked=previews.filter(p=>p.status==='blocked').length;summary.changed=previews.filter(p=>p.changes.length||p.playlistIntents.length||p.intents.length).length;summary.skipped=previews.filter(p=>p.status==='skipped'||p.status==='unchanged').length;
  return{valid:true,errors:[],records:previews,summary};
}

let builtinsReady=false;
function ensureBuiltinsRegistered() {
  if(builtinsReady) return;
  for(const spec of BUILTIN_SPECS) {
    registerBuiltin({
      id:spec.id,
      version:'0.7.0',
      apiVersion:1,
      kind:'builtin',
      alias:spec.type,
      label:spec.label,
      description:spec.description,
      group:spec.group,
      color:spec.color,
      capabilities:spec.capabilities,
      fields:spec.fields,
      defaults:spec.defaults,
      ports:spec.type==='filter'?['yes','no']:spec.type==='output'?[]:['out'],
      sideEffect:'pure',
    },()=>{ /* builtin logic stays in runWorkflow switch */ });
  }
  builtinsReady=true;
}
ensureBuiltinsRegistered();
