<script setup lang="ts">
import {computed,onMounted,ref,watch} from 'vue';
import type {Asset} from './types';
import {cleanupCsv,isLocalVideoDeleted} from './cleanup-view';
const props=defineProps<{assets:Asset[]}>();
const emit=defineEmits<{refresh:[]}>();
const method=ref('user_confirmed');
const selected=ref<string[]>([]),excluded=ref<string[]>([]),platform=ref(''),note=ref(''),confirmed=ref(false),permanent=ref(false),busy=ref(false),error=ref(''),plans=ref<any[]>([]),plan=ref<any>(null),supported=ref(false),search=ref('');
const visible=computed(()=>props.assets.filter(a=>!isLocalVideoDeleted(a)&&a.relative_path.toLowerCase().includes(search.value.toLowerCase())));
const ready=computed(()=>plan.value?.items.filter((i:any)=>i.status==='planned')??[]);
const bytes=computed(()=>ready.value.reduce((n:number,i:any)=>n+i.bytes,0));
watch([selected,excluded,platform,note,confirmed,method],()=>{plan.value=null;permanent.value=false;},{deep:true});
async function api(path:string,body?:unknown){const r=await fetch('/api/workflows/cleanup'+path,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});const v=await r.json();if(!r.ok)throw Error(v.error??'请求失败');return v;}
async function perform(fn:()=>Promise<void>){busy.value=true;error.value='';try{await fn();}catch(e){error.value=(e as Error).message;}finally{busy.value=false;}}
async function load(){const v=await api('');plans.value=v.plans;supported.value=v.supported;}
async function preview(){await perform(async()=>{plan.value=await api('',{asset_ids:selected.value,excluded_ids:excluded.value,platform:platform.value,note:note.value,confirmed_uploaded:confirmed.value,method:method.value});permanent.value=false;await load();});}
async function apply(){await perform(async()=>{plan.value=await api('/'+encodeURIComponent(plan.value.id)+'/apply',{confirm_permanent_delete:permanent.value});permanent.value=false;await load();emit('refresh');});}
function download(){const u=URL.createObjectURL(new Blob([JSON.stringify(plan.value,null,2)],{type:'application/json'}));const a=document.createElement('a');a.href=u;a.download='video-cleanup-'+plan.value.id+'.json';a.click();URL.revokeObjectURL(u);}
function downloadCsv(){const u=URL.createObjectURL(new Blob([cleanupCsv(plan.value)],{type:'text/csv;charset=utf-8'}));const a=document.createElement('a');a.href=u;a.download='video-cleanup-'+plan.value.id+'.csv';a.click();URL.revokeObjectURL(u);}
const labels:Record<string,string>={planned:'待执行',excluded:'保留',blocked:'已阻断',conflict:'冲突，请重新预览',deleted:'已清理',failed:'失败，请重新预览',deleting:'中断待核对',needs_review:'待人工核对'};
onMounted(()=>perform(load));
</script>
<template>
<section class="panel cleanup-panel">
 <h2>上传后清理本地视频</h2>
 <p>确认在线副本完整可用后，删除所选本地视频。全部弹幕、直播间信息及其他非视频文件保留。</p>
 <p v-if="!supported" class="warning-panel">此预览版仅支持 Windows 安全删除。</p>
 <p v-if="error" role="alert" class="warning-panel">{{error}}</p>
 <fieldset :disabled="busy"><legend>上传确认与保留例外</legend>
 <label>确认方式<select v-model="method" aria-label="确认方式"><option value="user_confirmed">人工确认（含外部平台和历史上传）</option><option value="platform_verified">YouTube 核验（本应用完整上传的同一文件）</option></select></label><label v-if="method==='user_confirmed'">目标平台 / 账号<input v-model="platform" placeholder="例如 YouTube / 我的频道"/></label>
 <label>核对说明 / 在线地址<textarea v-model="note" placeholder="说明已核对完整内容；可附视频或播放列表地址。"/></label>
 <p>人工确认绑定本次文件；YouTube 核验检查当前频道、同一文件的上传凭据与处理结果。历史上传或合并切片来源缺少凭据时会保留。</p>
 <input v-model="search" aria-label="筛选视频路径" placeholder="筛选目录或文件名"/>
 <button @click="selected=[...new Set([...selected,...visible.map(a=>a.id)])]">选择筛选结果</button><button @click="selected=[]">取消全部</button>
 <p>已选 {{selected.length}} 项 · 每批最多 500 项。保留项优先于上传确认。</p>
 <div class="table-wrap"><table><thead><tr><th>选择</th><th>视频路径</th><th>大小</th><th>保留</th></tr></thead><tbody>
 <tr v-for="a in visible" :key="a.id"><td><input v-model="selected" type="checkbox" :value="a.id" :aria-label="'选择 '+a.relative_path"/></td><td>{{a.relative_path}}</td><td>{{(a.bytes/1024**3).toFixed(2)}} GiB</td><td><input v-model="excluded" type="checkbox" :value="a.id" :aria-label="'保留 '+a.relative_path"/></td></tr>
 </tbody></table></div>
 <label><input v-model="confirmed" type="checkbox"/>我已核对所选视频的完整内容已上传并可在线访问（保留项除外）</label>
 <button :disabled="!supported||!selected.length||selected.length>500||(method==='user_confirmed'&&(!confirmed||!platform.trim()||!note.trim()))" @click="preview">生成冻结清理预览</button>
 </fieldset>
 <section v-if="plan"><h3>清理清单 · {{plan.created_at}}</h3><p>{{plan.platform}} · {{plan.method==='platform_verified'?'YouTube 处理状态已核验（执行前复查）':'用户已确认（未自动核验）'}}</p><p>{{plan.note}}</p>
 <p>待删除 {{ready.length}} 项 · 逻辑大小 {{(bytes/1024**3).toFixed(2)}} GiB。实际可用空间变化以磁盘为准。</p>
 <div class="table-wrap"><table><thead><tr><th>视频路径</th><th>结果</th><th>说明</th></tr></thead><tbody><tr v-for="i in plan.items" :key="i.asset_id"><td>{{i.path}}</td><td>{{labels[i.status]??i.status}}</td><td>{{i.message}}<a v-if="i.remote" :href="'https://www.youtube.com/watch?v='+encodeURIComponent(i.remote.video_id)" target="_blank" rel="noreferrer">查看在线视频</a></td></tr></tbody></table></div>
 <label><input v-model="permanent" type="checkbox" :disabled="busy"/>永久删除此冻结清单中的待执行视频，不移入回收站，无法撤销</label>
 <button :disabled="busy||!permanent||!ready.length||!supported" @click="apply">永久删除 {{ready.length}} 个视频</button><button @click="download">导出 JSON 清单</button><button @click="downloadCsv">导出 CSV 清单</button>
 </section>
 <h3>历史清理记录</h3><p v-if="!plans.length">暂无清理记录</p><button v-for="p in plans" :key="p.id" :disabled="busy" @click="plan=p;permanent=false">{{p.created_at}} · {{p.platform}} · {{p.items.length}} 项</button>
</section>
</template>
<style scoped>
.cleanup-panel{display:grid;gap:16px;padding:24px;min-width:0}
.cleanup-panel h2,.cleanup-panel h3,.cleanup-panel p{margin:0}
.cleanup-panel h2{font-size:20px}.cleanup-panel h3{font-size:16px}
.cleanup-panel p{line-height:1.7}
.cleanup-panel fieldset{display:grid;gap:14px;min-width:0;border:0;padding:16px 0;margin:0}
.cleanup-panel legend{font-weight:600;padding:0 0 10px}
.cleanup-panel label{display:block;line-height:1.8}
.cleanup-panel label>input:not([type=checkbox]),.cleanup-panel label>select,.cleanup-panel textarea{display:block;width:100%;margin-top:6px}
.cleanup-panel input[type=checkbox]{margin-right:8px}
.cleanup-panel .table-wrap{max-height:400px;overflow:auto}
.cleanup-panel textarea{min-height:80px;border:1px solid var(--border);border-radius:8px;padding:10px;background:var(--surface);color:inherit;font:inherit}
.cleanup-panel button{justify-self:start;border:1px solid var(--border);border-radius:8px;padding:8px 14px;font:inherit;cursor:pointer}
.cleanup-panel button:disabled{opacity:.45;cursor:default}
.cleanup-panel section{display:grid;gap:14px;border-top:1px solid var(--border);padding-top:20px;min-width:0}
.cleanup-panel td{overflow-wrap:anywhere}.cleanup-panel td a{display:block;margin-top:4px}
@media(max-width:600px){.cleanup-panel{padding:16px}.cleanup-panel th,.cleanup-panel td{padding:8px}}
</style>
