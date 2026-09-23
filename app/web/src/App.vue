<script setup lang="ts">
import {computed,nextTick,onMounted,onUnmounted,ref,watch} from 'vue';
import {Archive, ArrowDownUp, ArrowRight, Check, CheckCheck, ChevronLeft, ChevronRight, CircleAlert, CirclePlay, Clock3, Copy, Database, FileVideo, FolderOpen, HardDrive, Layers3, LayoutDashboard, ListChecks, LoaderCircle, Monitor, Moon, MoreHorizontal, Play, Plus, Radio, RefreshCw, Search, Settings2, ShieldCheck, Sparkles, Square, Sun, Terminal, WandSparkles, Workflow, X, Youtube} from 'lucide-vue-next';
import YouTube from './YouTube.vue';
import WorkflowStudio from './WorkflowStudio.vue';
import AppearanceSettings from './AppearanceSettings.vue';
import TaskCenter from './TaskCenter.vue';
import SelectionToolbar from './SelectionToolbar.vue';
import {routes,resolveRoute,type AppRoute} from './navigation';
import {selectIds,toggleId,selectionCounts} from './selection';
import {useTheme} from './theme';
import type {Asset,Job,Plan,Room,Snapshot,UnifiedTask,TaskAction,TaskSnapshot} from './types';

const route=ref<AppRoute>(resolveRoute(location.hash));
const expanded=ref<Record<string,boolean>>({[route.value.group]:true}),mobileMenu=ref(false);
const scrollPositions=new Map<string,number>();
const data=ref<Snapshot|null>(null),roomFilter=ref(''),roleFilter=ref<string>(route.value.group==='library'?route.value.mode:'all'),page=ref(1),busy=ref(false),locked=ref(false),tokenInput=ref(''),toast=ref('');
const view=computed({get:()=>route.value.group as string,set:(group:string)=>navigate(routes.find(r=>r.group===group)!.path)});
const searches=ref<Record<string,string>>({});
const search=computed({get:()=>searches.value[view.value]??'',set:(value:string)=>{searches.value[view.value]=value;}});
const taskSnapshot=ref<TaskSnapshot>({tasks:[],scan:{running:false,completed:0,total:0,message:''}}),taskError=ref('');
const youtubeVisible=computed(()=>view.value==='youtube'||route.value.path==='/settings/connections');
const youtubeVisited=ref(youtubeVisible.value);
const workflowRequest=ref<{kind:'local'|'youtube';ids:string[];nonce:number}>();
function openWorkflow(kind:'local'|'youtube',ids:string[]){workflowRequest.value={kind,ids,nonce:Date.now()};navigate('/pipeline/workflows');}
const youtubePage=computed(()=>route.value.path==='/settings/connections'?'account':view.value==='youtube'?route.value.mode:'videos');
watch(youtubeVisible,value=>{if(value)youtubeVisited.value=true;});
const selected=ref<Set<string>>(new Set()), detail=ref<Asset|null>(null), showPlan=ref(false), showRename=ref(false), chosenPlan=ref<Plan|null>(null), maxHours=ref(11+55/60), maxGap=ref(30), includeLegacy=ref(false);
const naming=ref({template:'{主播} · {日期} · {标题}',find:'',replace:'',regex:false});
const titleChanges=ref<{id:string;before:string;after:string}[]>([]), sort=ref('date'), dateFrom=ref(''), dateTo=ref('');
const {resolvedTheme,toggleDark}=useTheme();
let interval:ReturnType<typeof setInterval>|undefined,toastTimer:ReturnType<typeof setTimeout>|undefined;
const nav=[{id:'library',label:'媒体库',icon:Archive},{id:'rooms',label:'直播间',icon:Radio},{id:'pipeline',label:'处理管线',icon:Workflow},{id:'youtube',label:'YouTube',icon:Youtube},{id:'tasks',label:'任务中心',icon:ListChecks},{id:'settings',label:'设置',icon:Settings2}].map(group=>({...group,children:routes.filter(r=>r.group===group.id)}));
const taskCount=computed(()=>taskSnapshot.value.tasks.filter(t=>['pending','queued','running'].includes(t.status)).length);
const assets=computed(()=>data.value?.library.assets??[]),rooms=computed(()=>data.value?.library.rooms??[]),jobs=computed(()=>data.value?.jobs??[]);
const activeJobs=computed(()=>jobs.value.filter(j=>['pending','running'].includes(j.status)));
const knownHours=computed(()=>assets.value.reduce((s,a)=>s+(a.metadata?.duration??0),0)/3600);
const totalBytes=computed(()=>assets.value.reduce((s,a)=>s+a.bytes,0));
const warningCount=computed(()=>assets.value.filter(a=>a.warnings.length).length);
const filtered=computed(()=>{
  let list=assets.value.filter(a=>(!roomFilter.value||a.room_id===roomFilter.value)&&(roleFilter.value==='all'||(roleFilter.value==='review'?a.warnings.length:a.role===roleFilter.value))&&(!search.value||`${a.name} ${a.display_title??''} ${a.title} ${a.room_name} ${a.room_id}`.toLowerCase().includes(search.value.toLowerCase()))&&(!dateFrom.value||dateOnly(a.started_at)>=dateFrom.value)&&(!dateTo.value||dateOnly(a.started_at)<=dateTo.value));
  return list.toSorted((a,b)=>sort.value==='size'?b.bytes-a.bytes:(b.started_at?Date.parse(b.started_at):0)-(a.started_at?Date.parse(a.started_at):0));
});
const pages=computed(()=>Math.max(1,Math.ceil(filtered.value.length/30))),visible=computed(()=>filtered.value.slice((page.value-1)*30,page.value*30));
const allSelected=computed(()=>filtered.value.length>0&&filtered.value.every(a=>selected.value.has(a.id)));
const selection=computed(()=>selectionCounts(selected.value,filtered.value.map(a=>a.id),visible.value.map(a=>a.id)));
let selectionAnchor:string|null=null;
const filteredRooms=computed(()=>rooms.value.filter(r=>(route.value.mode!=='errors'||r.refresh_error)&&`${r.id} ${r.name} ${r.aliases.join(' ')}`.toLowerCase().includes(search.value.toLowerCase())));
const scanRunning=computed(()=>data.value?.scan.running??false);
const statusLabels:Record<string,string>={pending:'等待中',running:'处理中',completed:'已完成',failed:'失败',cancelled:'已取消',interrupted:'已中断'};
watch([search,roomFilter,roleFilter,sort,dateFrom,dateTo],()=>page.value=1);
watch(naming,()=>titleChanges.value=[],{deep:true});
watch(()=>route.value.path,()=>{expanded.value[route.value.group]=true;if(route.value.group==='library')roleFilter.value=route.value.mode;detail.value=null;showRename.value=false;showPlan.value=false;mobileMenu.value=false;});
function changeRoute(next:AppRoute){if(next.path===route.value.path)return;scrollPositions.set(route.value.path,window.scrollY);route.value=next;nextTick(()=>window.scrollTo({top:scrollPositions.get(next.path)??0,behavior:'instant'}));}
function navigate(path:string){changeRoute(resolveRoute(path));if(location.hash!=='#'+route.value.path)location.hash=route.value.path;}
function syncRoute(){changeRoute(resolveRoute(location.hash));}
function clearSelection(){selected.value=new Set();selectionAnchor=null;}
function onKeydown(event:KeyboardEvent){const target=event.target as HTMLElement;if(event.key==='Escape'&&!event.isComposing&&!target.closest('input,textarea,select,[contenteditable="true"]')&&!detail.value&&!showRename.value&&!showPlan.value&&view.value==='library')clearSelection();}
function showSavedPlan(id:string){const plan=data.value?.plans.find(p=>p.id===id);if(plan){chosenPlan.value=plan;navigate('/pipeline/current');}else notify('原计划不存在，请重新生成。');}
async function refreshTasks(){try{taskSnapshot.value=await api<TaskSnapshot>('/tasks');taskError.value='';}catch(e){taskError.value=(e as Error).message;}}
function taskAction(task:UnifiedTask,action:TaskAction){perform(async()=>{await api('/tasks/'+encodeURIComponent(task.id)+'/'+action,{});await poll();notify('任务操作已提交。');});}
function uploadSubmitted(){navigate('/tasks/active');refreshTasks();}

function size(n:number){return n>=1073741824?`${(n/1073741824).toFixed(2)} GiB`:`${(n/1048576).toFixed(1)} MiB`;}
function duration(n:number|null|undefined){if(n==null)return '时长未知';const t=Math.round(n);return `${Math.floor(t/3600).toString().padStart(2,'0')}:${Math.floor(t%3600/60).toString().padStart(2,'0')}:${(t%60).toString().padStart(2,'0')}`;}
function dateOnly(s:string|null){if(!s)return '';return new Intl.DateTimeFormat('sv-SE',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit'}).format(new Date(s));}
function date(s:string|null){if(!s)return '时间待确认';return new Intl.DateTimeFormat('zh-CN',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false}).format(new Date(s));}
function notify(s:string){toast.value=s;if(toastTimer)clearTimeout(toastTimer);toastTimer=setTimeout(()=>toast.value='',9000);}
async function api<T=any>(path:string,body?:unknown):Promise<T>{const r=await fetch(`/api${path}`,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});const value=await r.json();if(r.status===401)locked.value=true;if(!r.ok)throw new Error(value.error??`请求失败 ${r.status}`);return value;}
async function perform(f:()=>Promise<void>){busy.value=true;try{await f();}catch(e){notify(String((e as Error).message));}finally{busy.value=false;}}
async function load(){data.value=await api<Snapshot>('/snapshot');if(!chosenPlan.value&&data.value.plans.length)chosenPlan.value=data.value.plans[0];await refreshTasks();}
async function connect(token:string){const r=await fetch('/api/session',{method:'POST',headers:{Authorization:`Bearer ${token}`}});if(!r.ok)throw new Error('启动令牌无效，请重新打开本次启动提供的链接');locked.value=false;history.replaceState(null,'',location.pathname+location.search+'#'+route.value.path);await load();}
let polling=false;
async function poll(){if(polling||locked.value||!data.value)return;polling=true;await refreshTasks();try{const p=await api<{scan:Snapshot['scan'];jobs:Job[]}>('/status');const wasScanning=data.value.scan.running;data.value.scan=p.scan;data.value.jobs=p.jobs;if(wasScanning&&!p.scan.running){await load();notify(p.scan.message);}}catch{/* Keep the current view during transient restarts. */}finally{polling=false;}}
function toggle(id:string,event?:Event){selected.value=toggleId(selected.value,id,filtered.value.map(a=>a.id),selectionAnchor,(event as MouseEvent|undefined)?.shiftKey??false);selectionAnchor=id;}
function selectPage(){selected.value=selectIds(selected.value,visible.value.map(a=>a.id),'add');}
function invertResults(){selected.value=selectIds(selected.value,filtered.value.map(a=>a.id),'invert');}
function selectAll(){const s=new Set(selected.value);if(allSelected.value)filtered.value.forEach(a=>s.delete(a.id));else filtered.value.forEach(a=>s.add(a.id));selected.value=s;}
function startScan(){perform(async()=>{await api('/scan',{});await poll();});}
function buildPlan(){perform(async()=>{const p=await api<Plan>('/plans',{asset_ids:[...selected.value],max_duration:Math.round(maxHours.value*3600),max_bytes:250_000_000_000,max_gap:maxGap.value*60,include_legacy:includeLegacy.value});chosenPlan.value=p;showPlan.value=false;view.value='pipeline';await load();});}
function executePlan(){if(!chosenPlan.value)return;perform(async()=>{await api(`/plans/${chosenPlan.value!.id}/execute`,{});view.value='tasks';await poll();notify('合并任务已启动，源录像会保留。');});}
function refreshRoom(r:Room){perform(async()=>{const result=await api<Room>(`/rooms/${r.id}/refresh`,{});await load();notify(result.refresh_error??'直播间当前资料已更新，历史标题保持不变。');});}
function roomAssets(id:string){return assets.value.filter(a=>a.room_id===id);}
function openRoom(id:string){view.value='library';roomFilter.value=id;roleFilter.value='all';}
function showTitles(){showRename.value=true;titleChanges.value=[];}
function previewTitles(){perform(async()=>{titleChanges.value=await api('/titles/preview',{asset_ids:[...selected.value],...naming.value});});}
function applyTitles(){perform(async()=>{await api('/titles/apply',titleChanges.value);showRename.value=false;await load();notify('显示标题已保存，原始文件名未改动。');});}
function retry(j:Job){const p=data.value?.plans.find(p=>p.id===j.plan_id);if(p){chosenPlan.value=p;view.value='pipeline';}else notify('原计划不存在，请重新生成。');}
async function copy(text:string){try{await navigator.clipboard.writeText(text);notify('已复制');}catch{notify('浏览器未允许剪贴板访问');}}
onMounted(async()=>{window.addEventListener('hashchange',syncRoute);window.addEventListener('keydown',onKeydown);await perform(async()=>{const token=new URLSearchParams(location.hash.slice(1)).get('token');if(token)await connect(token);else await load();});interval=setInterval(poll,3000);});
onUnmounted(()=>{window.removeEventListener('hashchange',syncRoute);window.removeEventListener('keydown',onKeydown);if(interval)clearInterval(interval);if(toastTimer)clearTimeout(toastTimer);});
</script>

<template>
  <div class="workspace">
    <button v-if="mobileMenu" class="nav-backdrop" aria-label="关闭导航" @click="mobileMenu=false"></button>
    <aside class="sidebar" :class="{'mobile-open':mobileMenu}">
      <div class="brand"><div class="brand-mark"><Layers3 :size="23"/></div><span>U2BUP<small>录播工作台</small></span></div>
      <div class="workspace-label">个人工作空间 <span>LOCAL</span></div>
      <nav class="nav-tree" aria-label="主导航"><div v-for="group in nav" :key="group.id" class="nav-group">
        <button class="nav-group-button" :class="{active:view===group.id}" :aria-expanded="!!expanded[group.id]" :aria-controls="'nav-'+group.id" @click="expanded[group.id]=!expanded[group.id]"><component :is="group.icon" :size="18"/><span>{{group.label}}</span><b v-if="group.id==='tasks'&&taskCount">{{taskCount}}</b><ChevronRight class="nav-chevron" :class="{expanded:expanded[group.id]}" :size="14"/></button>
        <div v-show="expanded[group.id]" :id="'nav-'+group.id" class="nav-children"><a v-for="child in group.children" :key="child.path" :href="'#'+child.path" :class="{active:route.path===child.path}" :aria-current="route.path===child.path?'page':undefined" @click.prevent="navigate(child.path)">{{child.label}}</a></div>
      </div></nav>
      <div class="sidebar-divider"></div>
      <div class="sidebar-heading">当前媒体库</div>
      <button class="library-root" @click="navigate('/settings/environment')"><FolderOpen :size="17"/><span>LiveRec<small>{{assets.length}} 个录像文件</small></span><span class="dot"></span></button>

      <div class="sidebar-bottom"><div class="version"><span class="dot"></span>U2BUP <span>v0.7.0</span></div></div>
    </aside>
    <main>
      <header class="topbar"><button class="icon-button mobile-menu-button" aria-label="展开或收起导航" :aria-expanded="mobileMenu" @click="mobileMenu=!mobileMenu"><ListChecks :size="19"/></button><div class="breadcrumb">{{nav.find(n=>n.id===view)?.label}} <ChevronRight :size="13"/><strong>{{route.label}}</strong></div><div class="top-actions"><span class="local-chip"><Monitor :size="14"/> {{data?.settings.mode??'工作台'}}</span><button class="icon-button theme-toggle" :title="resolvedTheme==='dark'?'切换到浅色模式':'切换到深色模式'" :aria-label="resolvedTheme==='dark'?'切换到浅色模式':'切换到深色模式'" @click="toggleDark"><Sun v-if="resolvedTheme==='dark'" :size="17"/><Moon v-else :size="17"/></button><button class="icon-button" title="刷新视图" aria-label="刷新视图" @click="perform(load)"><RefreshCw :size="17" :class="{spin:busy}"/></button><div class="avatar">U</div></div></header>
      <section v-if="locked" class="connection-card"><div class="eyebrow">连接工作台</div><h1>建立本机会话</h1><p>请从服务启动链接打开，或粘贴本次启动令牌。令牌保存在应用数据目录的 connection.json 中。</p><input v-model="tokenInput" type="password" placeholder="启动令牌"/><button class="primary" @click="perform(()=>connect(tokenInput))">连接</button></section>
      <div v-else-if="data" class="content">
        <div v-if="scanRunning" class="scan-banner"><LoaderCircle :size="18" class="spin"/><span>{{data.scan.message}} · {{data.scan.completed}} / {{data.scan.total}}</span><progress :max="Math.max(data.scan.total,1)" :value="data.scan.completed"></progress></div>
        <template v-if="view==='library'">
          <div class="page-heading"><div><div class="eyebrow">YOUR RECORDING LIBRARY</div><h1>每一场直播，都有迹可循<span class="heading-dot">.</span></h1><p>整理历史录播，连接直播间，从素材到成品一目了然。</p></div><button class="primary" :disabled="busy||scanRunning||activeJobs.length>0" @click="startScan"><RefreshCw :size="16" :class="{spin:scanRunning}"/>{{data.library.scanned_at?'重新扫描素材':'扫描 LiveRec'}}</button></div>
          <div class="stats-grid"><div class="stat-card"><span>录像文件<Archive :size="18"/></span><strong>{{assets.length}}<small>个</small></strong><footer>{{assets.filter(a=>a.role==='source').length}} 个原始片段 · {{assets.filter(a=>a.role==='legacy').length}} 个历史成品</footer></div><div class="stat-card"><span>关联直播间<Radio :size="18"/></span><strong>{{rooms.length}}<small>个</small></strong><footer>以房间号关联历史名称与目录</footer></div><div class="stat-card"><span>已知录像时长<Clock3 :size="18"/></span><strong>{{knownHours.toFixed(1)}}<small>小时</small></strong><footer>{{assets.filter(a=>!a.metadata?.duration).length}} 个文件时长待确认 · 未去重</footer></div><div class="stat-card"><span>素材总容量<HardDrive :size="18"/></span><strong>{{(totalBytes/1073741824).toFixed(1)}}<small>GiB</small></strong><footer>原片只读导入，成品单独保存</footer></div></div>
          <div class="section-header"><h2>录像素材 <span class="count">{{assets.length}}</span></h2><div class="caption"><span class="dot"></span>{{data.library.scanned_at?'最近扫描 '+date(data.library.scanned_at):'等待首次导入'}}</div></div>
          <section class="panel library-panel">
            <div class="tab-row"><button v-for="t in [{id:'all',label:'全部素材',n:assets.length},{id:'source',label:'原始片段',n:assets.filter(a=>a.role==='source').length},{id:'legacy',label:'历史成品',n:assets.filter(a=>a.role==='legacy').length},{id:'review',label:'待检查',n:warningCount}]" :key="t.id" :class="{selected:roleFilter===t.id}" @click="navigate('/library/'+t.id)">{{t.label}}<span>{{t.n}}</span></button></div>
            <div class="filter-row"><div class="search-input"><Search :size="17"/><input v-model="search" placeholder="搜索标题、主播或房间号…" aria-label="搜索素材"/><kbd>/</kbd></div><select v-model="roomFilter" aria-label="筛选直播间"><option value="">全部直播间</option><option v-for="r in rooms" :key="r.id" :value="r.id">{{r.name}} · {{r.id}}</option></select><select v-model="sort" aria-label="排序"><option value="date">录制时间 ↓</option><option value="size">文件大小 ↓</option></select><button class="subtle" :disabled="!selected.size" @click="showTitles"><WandSparkles :size="16"/>批量标题</button></div>
            <div class="selection-row"><label><input type="checkbox" :checked="allSelected" @change="selectAll"/>全选筛选结果 <span>({{filtered.length}})</span></label><div class="date-filter"><span>录制日期</span><input type="date" v-model="dateFrom" aria-label="开始日期"/><span>—</span><input type="date" v-model="dateTo" aria-label="结束日期"/></div><button class="text-action" :disabled="!visible.length" @click="selectPage">选本页</button><button class="text-action" :disabled="!filtered.length" @click="invertResults">反选筛选结果</button><span v-if="selected.size" class="selection-text">已选 {{selected.size}} 项 · 筛选外 {{selection.hidden}} 项 <button @click="clearSelection">取消全部选择</button></span></div>
            <div v-if="!assets.length" class="empty"><div class="empty-icon"><FolderOpen :size="34"/></div><h3>把散落的录播，带进工作台</h3><p>扫描已配置的 LiveRec，自动提取媒体参数和 XML 历史资料。</p><button class="primary" :disabled="scanRunning||busy" @click="startScan"><Plus :size="17"/>开始扫描</button></div>
            <div v-else-if="!filtered.length" class="empty"><Search :size="30"/><h3>没有符合筛选条件的素材</h3><button class="subtle" @click="search='';roomFilter='';roleFilter='all';dateFrom='';dateTo=''">清除筛选</button></div>
            <div v-else class="table-wrap"><table><thead><tr><th class="check-cell"></th><th>录像 / 直播间</th><th>录制时间 <span class="muted">UTC+8</span></th><th>规格</th><th>时长</th><th>大小</th><th>状态</th><th></th></tr></thead><tbody><tr v-for="a in visible" :key="a.id" :class="{checked:selected.has(a.id)}"><td class="check-cell"><input type="checkbox" :checked="selected.has(a.id)" :aria-label="'选择 '+a.name" @click="toggle(a.id,$event)"/></td><td><button class="asset-cell" @click="detail=a"><div class="thumb"><FileVideo :size="20"/><img :src="`/api/assets/${a.id}/thumbnail`" loading="lazy" alt="" @error="($event.target as HTMLImageElement).style.display='none'"/><span>{{a.extension.toUpperCase()}}</span></div><div class="asset-text"><strong :title="a.display_title??a.title">{{a.display_title??a.title}}</strong><small>{{a.room_name}}<span>·</span>{{a.room_id}}</small></div></button></td><td class="date-cell">{{date(a.started_at)}}</td><td><span class="resolution">{{a.metadata?`${a.metadata.width} × ${a.metadata.height}`:'未知规格'}}</span><small class="muted block">{{a.metadata?.codec?.toUpperCase()??'—'}} · {{a.metadata?.aspect??'—'}}</small></td><td class="mono">{{duration(a.metadata?.duration)}}</td><td class="mono muted">{{size(a.bytes)}}</td><td><span class="badge" :class="a.warnings.length?'amber':a.role==='legacy'?'blue':'green'"><span class="dot"></span>{{a.warnings.length?'待检查':a.role==='legacy'?'历史成品':'可规划'}}</span></td><td><button class="icon-button" @click="detail=a" aria-label="查看素材详情"><MoreHorizontal :size="19"/></button></td></tr></tbody></table></div>
            <div class="table-footer"><span>{{filtered.length?`${(page-1)*30+1}–${Math.min(page*30,filtered.length)}`:'0'}} / {{filtered.length}} 个文件</span><div class="pager"><button :disabled="page<=1" @click="page--" aria-label="上一页"><ChevronLeft :size="16"/></button><span>{{page}} / {{pages}}</span><button :disabled="page>=pages" @click="page++" aria-label="下一页"><ChevronRight :size="16"/></button></div></div>
          </section>
          <div class="bottom-hint"><ShieldCheck :size="15"/>批量标题仅改变库内显示名称。合并先生成计划，执行结果写入独立目录。</div>
          <SelectionToolbar v-bind="selection" scope="媒体库" @clear="clearSelection"><button class="subtle" @click="openWorkflow('local',[...selected])"><Workflow :size="16"/>加入自定义管线</button><button class="subtle" @click="showTitles"><WandSparkles :size="16"/>编辑显示标题</button><button class="primary" @click="showPlan=true"><Workflow :size="16"/>生成合并计划<ArrowRight :size="16"/></button></SelectionToolbar>
        </template>

        <template v-else-if="view==='rooms'">
          <div class="page-heading"><div><div class="eyebrow">CHANNEL DIRECTORY</div><h1>直播间，一处管理<span class="heading-dot">.</span></h1><p>历史名称来自本地目录与 XML，当前资料按需从 B 站刷新。</p></div><div class="search-input"><Search :size="17"/><input v-model="search" placeholder="搜索主播或房间号"/></div></div>
          <div v-if="!filteredRooms.length" class="panel empty"><Radio :size="30"/><h3>{{route.mode==='errors'?'暂无资料更新异常':'没有匹配的直播间'}}</h3></div><div class="room-grid"><article v-for="r in filteredRooms" :key="r.id" class="room-card"><div class="room-top"><div class="room-avatar">{{r.name.slice(0,1)}}</div><span class="badge" :class="r.online?.live_status===1?'green':'neutral'">{{r.online?(r.online.live_status===1?'直播中':'未开播'):'本地档案'}}</span></div><h3>{{r.online?.name??r.name}}</h3><small class="muted">BILIBILI · {{r.id}}</small><p class="room-title">{{r.online?.title??r.historical_title??'暂无历史标题'}}</p><div class="room-metrics"><span><strong>{{roomAssets(r.id).length}}</strong> 个录像</span><span><strong>{{r.directories.length}}</strong> 个目录</span></div><div v-if="r.aliases.length>1" class="aliases">历史名称：{{r.aliases.join(' / ')}}</div><p v-if="r.refresh_error" class="inline-error">{{r.refresh_error}}</p><div class="room-actions"><button class="subtle" @click="openRoom(r.id)">查看素材<ArrowRight :size="14"/></button><button class="icon-button" :disabled="busy||scanRunning||activeJobs.length>0" @click="refreshRoom(r)" title="抓取当前直播间资料" aria-label="抓取当前直播间资料"><RefreshCw :size="16"/></button></div></article></div>
          <div v-if="!filteredRooms.length" class="empty"><Radio :size="32"/><h3>暂无直播间</h3><p>先扫描素材库，即可建立本地直播间档案。</p></div>
        </template>

        <template v-else-if="view==='pipeline'&&route.mode==='history'">
          <div class="page-heading"><div><div class="eyebrow">SAVED PLANS</div><h1>已保存的处理计划</h1><p>查看输入、拆分原因和输出，再执行处理。</p></div><button class="subtle" @click="navigate('/library/all')">选择素材</button></div>
          <div v-if="!data.plans.length" class="panel empty"><Workflow :size="32"/><h3>还没有保存的计划</h3></div>
          <article v-for="plan in data.plans" :key="plan.id" class="panel output-card"><div class="output-header"><div><h3>{{date(plan.created_at)}}</h3><p>{{plan.request.asset_ids.length}} 个素材 · {{plan.outputs.length}} 个输出 · {{plan.blocked.length}} 项待检查</p></div><button class="subtle" @click="showSavedPlan(plan.id)">查看计划<ArrowRight :size="15"/></button></div></article>
        </template>
        <WorkflowStudio v-else-if="view==='pipeline'&&route.mode==='workflows'" :library="data" :selection-request="workflowRequest" @refresh="load" @navigate="navigate"/>
        <template v-else-if="view==='pipeline'">
          <div class="page-heading"><div><div class="eyebrow">PROCESSING PIPELINE</div><h1>先看计划，再生成成品<span class="heading-dot">.</span></h1><p>按直播间、场次与连续兼容规格分组，保留直播原本的顺序。</p></div><button class="subtle" @click="view='library'"><Plus :size="16"/>选择素材</button></div>
          <div class="pipeline-steps"><span><Database :size="17"/>素材索引</span><ArrowRight :size="16"/><span><Layers3 :size="17"/>识别连续片段</span><ArrowRight :size="16"/><span class="current"><Workflow :size="17"/>预览合并计划</span><ArrowRight :size="16"/><span><ShieldCheck :size="17"/>执行与验证</span></div>
          <div v-if="!chosenPlan" class="panel empty"><Workflow :size="36"/><h3>让合并过程清晰可见</h3><p>在素材库勾选录像，生成可以检查每个输入和输出的处理计划。</p><button class="primary" @click="view='library'">前往素材库<ArrowRight :size="16"/></button></div>
          <template v-else>
            <div class="plan-summary panel"><div><h2>{{chosenPlan.outputs.length}} 个输出 <span class="muted">/ {{chosenPlan.request.asset_ids.length}} 个已选素材</span></h2><p>预计占用 {{size(chosenPlan.estimated_bytes)}} · 单文件上限 {{duration(chosenPlan.request.max_duration)}} · 成品保留来源记录</p></div><button class="primary" :disabled="busy||scanRunning||activeJobs.length>0||!chosenPlan.outputs.length" @click="executePlan"><Play :size="16"/>执行 {{chosenPlan.outputs.length}} 项合并</button></div>
            <div class="plan-selector"><span>已保存的计划</span><select :value="chosenPlan.id" @change="chosenPlan=data.plans.find(p=>p.id===($event.target as HTMLSelectElement).value)??chosenPlan"><option v-for="p in data.plans" :key="p.id" :value="p.id">{{date(p.created_at)}} · {{p.outputs.length}} 个输出 · {{p.id.slice(0,8)}}</option></select></div>
            <div v-if="chosenPlan.blocked.length" class="warning-panel"><h3><CircleAlert :size="18"/>{{chosenPlan.blocked.length}} 项不会执行</h3><p v-for="b in chosenPlan.blocked" :key="b.asset_id"><strong>{{b.name}}</strong><br/>{{b.reason}}</p></div>
            <article v-for="(o,index) in chosenPlan.outputs" :key="o.id" class="output-card panel"><div class="output-header"><span class="output-index">{{String(index+1).padStart(2,'0')}}</span><div><h3>{{o.room_name}} · {{o.title}}</h3><p>{{o.reason}}</p></div><span class="badge green">{{o.aspect}}</span><span class="mono">{{duration(o.duration)}}</span></div><div class="output-name"><FileVideo :size="16"/><span>{{o.name}}</span></div><details><summary>{{o.inputs.length}} 个输入片段 · {{size(o.bytes)}}<ChevronRight :size="15"/></summary><div class="input-list"><div v-for="a in o.inputs" :key="a.id"><span>{{a.name}}</span><span class="mono">{{duration(a.metadata?.duration)}}</span></div></div></details></article>
          </template>
        </template>

        <TaskCenter v-else-if="view==='tasks'" :tasks="taskSnapshot.tasks" :filter="route.mode" :busy="busy" :error="taskError" :scan="taskSnapshot.scan" @action="taskAction" @plan="showSavedPlan" @refresh="refreshTasks" @copy="copy"/>

        <template v-else-if="view==='settings'&&route.mode!=='account'">
          <div class="page-heading"><div><div class="eyebrow">LOCAL ENVIRONMENT</div><h1>{{route.label}}<span class="heading-dot">.</span></h1><p>{{route.mode==='appearance'?'选择适合你的颜色与显示模式。':'查看媒体目录、处理组件和服务信息。'}}</p></div></div><AppearanceSettings v-if="route.mode==='appearance'"/><div v-if="route.mode==='environment'" class="panel settings-panel"><h2><Terminal :size="20"/>运行配置</h2><div v-for="(label,key) in {library:'素材根目录',output:'成品输出目录',data:'数据库与缓存',ffmpeg:'FFmpeg',ffprobe:'FFprobe',version:'应用版本',mode:'访问模式'}" :key="key" class="setting"><span>{{label}}</span><code>{{data.settings[key]}}</code></div></div><div v-if="route.mode==='environment'" class="panel settings-panel"><h2><ShieldCheck :size="20"/>本版能力边界</h2><p>已实现扫描、XML 历史资料、直播间信息抓取、显示标题编辑、合并计划、流复制合并及抽样验证。</p><p>支持超长文件自动转码切割及 YouTube 上传与批量管理。FLV 在线播放和实时录制尚未接入；未知时长素材仍阻止执行。</p><p>媒体原片不会自动删除。批量标题不会修改磁盘文件名。录制集成和远程 WebUI 将分阶段加入。</p></div><div v-if="route.mode==='environment'&&data.library.errors.length" class="warning-panel"><h3>扫描期间的异常</h3><p v-for="e in data.library.errors" :key="e">{{e}}</p></div>
        </template>
        <YouTube v-if="youtubeVisited" v-show="youtubeVisible" :page="youtubePage" :active="youtubeVisible" @navigate="navigate" @uploaded="uploadSubmitted" @workflow="ids=>openWorkflow('youtube',ids)"/>
      </div>
      <div v-else class="empty initial"><LoaderCircle class="spin" :size="30"/><h3>正在连接本机工作台…</h3><button class="subtle" @click="perform(load)">重试连接</button></div>
    </main>

    <div v-if="detail" class="drawer-backdrop" @click.self="detail=null"><aside class="drawer"><div class="drawer-heading"><span>素材详情</span><button class="icon-button" @click="detail=null" aria-label="关闭详情"><X :size="21"/></button></div><video v-if="detail.extension==='mp4'" :src="`/api/assets/${detail.id}/media`" controls preload="metadata" class="detail-preview"></video><img v-else :src="`/api/assets/${detail.id}/thumbnail`" class="detail-preview" alt="录像预览"/><div class="drawer-content"><span class="eyebrow">{{detail.extension.toUpperCase()}} · {{detail.room_name}}</span><h2>{{detail.display_title??detail.title}}</h2><p v-if="detail.extension!=='mp4'" class="help">本版 FLV / MKV / TS 仅提供缩略图和参数查看，尚未接入浏览器预览代理。</p><div v-for="w in detail.warnings" :key="w" class="inline-warning"><CircleAlert :size="16"/>{{w}}</div><dl><dt>房间号</dt><dd>{{detail.room_id}}</dd><dt>录制时间</dt><dd>{{date(detail.started_at)}}</dd><dt>时间依据</dt><dd>{{detail.time_source}}</dd><dt>时长</dt><dd>{{duration(detail.metadata?.duration)}}</dd><dt>画面</dt><dd>{{detail.metadata?.width}} × {{detail.metadata?.height}} · {{detail.metadata?.aspect}}</dd><dt>文件大小</dt><dd>{{size(detail.bytes)}}</dd><dt>来源类型</dt><dd>{{detail.role==='legacy'?'历史成品，来源待确认':'原始录像片段'}}</dd></dl><h3>原始文件</h3><code class="path-box">{{detail.relative_path}}</code><h3>关联文件 <span class="count">{{detail.sidecars.length}}</span></h3><p v-for="s in detail.sidecars" :key="s" class="sidecar">{{s}}</p><p v-if="!detail.sidecars.length" class="muted">未找到同名 XML / TXT</p><button class="primary full" @click="toggle(detail.id)"><Check v-if="selected.has(detail.id)" :size="16"/><Plus v-else :size="16"/>{{selected.has(detail.id)?'已加入选择 · 点击移除':'加入批量选择'}}</button></div></aside></div>

    <div v-if="showPlan" class="modal-backdrop" @click.self="showPlan=false"><section class="modal"><div class="modal-heading"><div class="modal-icon"><Workflow :size="25"/></div><button class="icon-button" @click="showPlan=false"><X :size="21"/></button></div><h2>为 {{selected.size}} 个素材生成计划</h2><p>这一步只生成预览，不运行 FFmpeg。按时间顺序分组，画幅或编码变化时拆开。</p><label class="field-label">单个成品时长上限<select v-model="maxHours"><option :value="11+55/60">11 小时 55 分钟（预留余量）</option><option :value="6">6 小时</option><option :value="3">3 小时</option><option :value="1">1 小时</option></select></label><label class="field-label">同一场次允许的最大断档<select v-model="maxGap"><option :value="5">5 分钟</option><option :value="15">15 分钟</option><option :value="30">30 分钟</option><option :value="60">60 分钟</option></select></label><label class="checkbox-label"><input type="checkbox" v-model="includeLegacy"/>包含历史 MP4 成品（每个独立处理，不与原片混拼）</label><div class="info-note"><ShieldCheck :size="18"/><span>原片保留 · 250 GB 体积预算 · 未知时长阻止执行 · 超长文件自动精确切割（H.264/AAC 转码）</span></div><div class="modal-footer"><button class="subtle" @click="showPlan=false">取消</button><button class="primary" :disabled="busy" @click="buildPlan"><LoaderCircle v-if="busy" class="spin" :size="16"/>生成计划<ArrowRight :size="16"/></button></div></section></div>

    <div v-if="showRename" class="modal-backdrop" @click.self="showRename=false"><section class="modal rename-modal"><div class="modal-heading"><div class="modal-icon"><WandSparkles :size="25"/></div><button class="icon-button" @click="showRename=false"><X :size="21"/></button></div><h2>批量编辑显示标题</h2><p>只修改素材库内的显示名称，原始文件名与历史标题保留。</p><label class="field-label">标题模板<input v-model="naming.template"/></label><div class="token-buttons"><button v-for="t in ['{主播}','{日期}','{标题}','{序号}']" :key="t" @click="naming.template+=t">{{t}}</button></div><div class="two-fields"><label class="field-label">查找<input v-model="naming.find" placeholder="可留空"/></label><label class="field-label">替换为<input v-model="naming.replace" placeholder="可留空"/></label></div><label class="checkbox-label"><input type="checkbox" v-model="naming.regex"/>高级：查找使用正则表达式（替换捕获组使用 $1）</label><div v-if="titleChanges.length" class="rename-preview"><div v-for="c in titleChanges" :key="c.id"><span>{{c.before}}</span><ArrowRight :size="14"/><strong>{{c.after}}</strong></div></div><div class="modal-footer"><button class="subtle" :disabled="busy" @click="previewTitles">预览 {{selected.size}} 项变更</button><button class="primary" :disabled="busy||!titleChanges.length" @click="applyTitles"><Check :size="16"/>应用显示标题</button></div></section></div>
    <div v-if="toast" class="toast" role="status"><CircleAlert :size="19"/><span>{{toast}}</span><button @click="toast=''" aria-label="关闭提示"><X :size="16"/></button></div>
  </div>
</template>
