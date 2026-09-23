<script setup lang="ts">
import {computed,ref} from 'vue';
import {ArrowRight,Check,Copy,ListChecks,LoaderCircle,RefreshCw,ShieldCheck,Upload,Workflow} from 'lucide-vue-next';
import type {UnifiedTask,TaskAction,Scan} from './types';
const props=defineProps<{tasks:UnifiedTask[];filter:string;busy:boolean;error:string;scan:Scan}>();
defineEmits<{action:[task:UnifiedTask,action:TaskAction];plan:[id:string];refresh:[];copy:[path:string]}>();
const kind=ref('all'),search=ref('');
const labels:Record<string,string>={pending:'等待中',queued:'排队中',running:'进行中',paused:'已暂停',interrupted:'已中断',completed:'已完成',failed:'失败',cancelled:'已取消'};
const actions:Record<TaskAction,string>={cancel:'取消任务',pause:'暂停上传',resume:'继续上传',retry:'重试失败任务'};
const active=computed(()=>props.tasks.filter(t=>['pending','queued','running'].includes(t.status)).length);
const filtered=computed(()=>props.tasks.filter(t=>(kind.value==='all'||t.kind===kind.value)&&(!search.value||`${t.title} ${t.message} ${t.source_id}`.toLowerCase().includes(search.value.toLowerCase()))&&(props.filter==='all'||(props.filter==='active'&&['pending','queued','running'].includes(t.status))||(props.filter==='attention'&&['failed','paused','interrupted','cancelled'].includes(t.status))||(props.filter==='completed'&&t.status==='completed'))));
const page=ref(1),pageSize=30;
const pages=computed(()=>Math.max(1,Math.ceil(filtered.value.length/pageSize)));
const currentPage=computed(()=>Math.min(page.value,pages.value));
const visible=computed(()=>filtered.value.slice((currentPage.value-1)*pageSize,currentPage.value*pageSize));
const date=(s:string|null)=>s?new Date(s).toLocaleString('zh-CN'):'历史任务 · 未记录时间';
const size=(n:number)=>n>=1073741824?`${(n/1073741824).toFixed(2)} GiB`:`${(n/1048576).toFixed(1)} MiB`;
function available(t:UnifiedTask){return (Object.keys(actions) as TaskAction[]).filter(a=>t.capabilities[a]&&!(a==='retry'&&t.capabilities.resume));}
</script>
<template>
  <div class="page-heading"><div><div class="eyebrow">TASK CENTER</div><h1>所有任务，一处跟进<span class="heading-dot">.</span></h1><p>媒体处理和 YouTube 上传统一管理。切换页面不会停止后台任务。</p></div><span class="badge green">{{active}} 个进行中与等待</span></div>
  <div v-if="error" class="warning-panel" role="alert"><p>{{error}} · 当前保留上次读取的状态。</p><button class="subtle" @click="$emit('refresh')">重试读取</button></div>
  <div class="task-filters panel"><label>任务类型<select v-model="kind" @change="page=1"><option value="all">全部类型</option><option value="media">媒体处理</option><option value="upload">YouTube 上传</option></select></label><label class="task-search">查找任务<input v-model="search" placeholder="标题、进度说明或任务 ID" @input="page=1"/></label><button class="subtle" :disabled="busy" @click="$emit('refresh')"><RefreshCw :size="15"/>刷新任务</button></div>
  <div v-if="scan.running" class="scan-banner"><LoaderCircle :size="16" class="spin"/>素材扫描 · {{scan.completed}} / {{scan.total}} · {{scan.message}}</div>
  <div v-if="!filtered.length" class="panel empty"><ListChecks :size="36"/><h3>{{tasks.length?'没有符合条件的任务':'还没有任务'}}</h3><p>合并、切割和上传提交后会出现在这里。</p></div>
  <article v-for="task in visible" :key="task.id" class="job-card panel" :data-task-id="task.id">
    <div class="job-heading"><div class="job-icon" :class="task.status"><LoaderCircle v-if="task.status==='running'" class="spin" :size="22"/><Check v-else-if="task.status==='completed'" :size="22"/><Upload v-else-if="task.kind==='upload'" :size="22"/><Workflow v-else :size="22"/></div><div><h3>{{task.title}}</h3><small>{{task.kind==='upload'?'YouTube 上传':'媒体处理'}} · {{date(task.created_at)}}</small></div><span class="badge" :class="task.status==='completed'?'green':task.status==='failed'?'amber':'neutral'">{{labels[task.status]??task.status}}</span></div>
    <progress v-if="task.progress!==null" :value="task.progress" max="1" :aria-label="task.title+'进度'"></progress>
    <div class="job-message"><span>{{task.message}}</span><strong v-if="task.progress!==null">{{Math.round(task.progress*100)}}%</strong></div>
    <div v-if="task.kind==='upload'&&task.bytes" class="muted task-transfer">已确认 {{size(task.offset??0)}} / {{size(task.bytes)}}<span v-if="task.status==='completed'"> · 上传请求已完成，在线播放处理状态以 YouTube 为准</span></div>
    <div class="task-actions"><button v-for="action in available(task)" :key="action" class="subtle" :disabled="busy" @click="$emit('action',task,action)">{{actions[action]}}</button><button v-if="task.plan_id" class="subtle" @click="$emit('plan',task.plan_id!)">查看原计划<ArrowRight :size="14"/></button><a v-if="task.video_id" class="subtle" :href="'https://www.youtube.com/watch?v='+encodeURIComponent(task.video_id)" target="_blank" rel="noreferrer">查看 YouTube 视频 ↗</a></div>
    <details class="task-details"><summary>任务详情 · {{task.source_id.slice(0,8)}}</summary><dl><dt>任务 ID</dt><dd>{{task.id}}</dd><dt>最近更新</dt><dd>{{date(task.updated_at)}}</dd></dl><div v-for="artifact in task.completed_outputs??[]" :key="artifact.output_id" class="artifact"><ShieldCheck :size="18"/><div><strong>{{artifact.path.split(/[\\/]/).pop()}}</strong><small>{{artifact.validation}} · {{size(artifact.bytes)}}</small><code>{{artifact.path}}</code></div><button class="icon-button" aria-label="复制成品路径" @click="$emit('copy',artifact.path)"><Copy :size="16"/></button></div></details>
  </article>
  <div v-if="filtered.length" class="table-footer"><span>{{filtered.length}} 个任务</span><div class="pager"><button :disabled="currentPage<=1" aria-label="上一页任务" @click="page=currentPage-1">‹</button><span>{{currentPage}} / {{pages}}</span><button :disabled="currentPage>=pages" aria-label="下一页任务" @click="page=currentPage+1">›</button></div></div>
</template>
