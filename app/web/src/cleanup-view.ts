export function isLocalVideoDeleted(asset:{warnings:string[]}):boolean {
  return asset.warnings.some(w=>w.includes('local_deleted')||w==='本地视频已清理；弹幕和元数据保留');
}
export function cleanupCsv(plan:{id:string;created_at:string;method:string;platform:string;note:string;items:Array<{asset_id:string;path:string;bytes:number;status:string;message:string;remote?:{video_id:string}|null}>}):string {
  const cell=(v:unknown)=>{let s=String(v??'');if(/^[\s]*[=+@-]/.test(s))s="'"+s;return '"'+s.replaceAll('"','""')+'"';};
  const rows=[['plan_id','created_at','method','platform','note','asset_id','path','bytes','status','message','video_url'],...plan.items.map(i=>[plan.id,plan.created_at,plan.method,plan.platform,plan.note,i.asset_id,i.path,i.bytes,i.status,i.message,i.remote?'https://www.youtube.com/watch?v='+encodeURIComponent(i.remote.video_id):''])];
  return '\ufeff'+rows.map(row=>row.map(cell).join(',')).join('\r\n')+'\r\n';
}
