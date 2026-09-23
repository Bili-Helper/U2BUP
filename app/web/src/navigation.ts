export const routes = [
  {path:'/library/all',group:'library',label:'全部素材',mode:'all'},
  {path:'/library/source',group:'library',label:'原始片段',mode:'source'},
  {path:'/library/legacy',group:'library',label:'历史成品',mode:'legacy'},
  {path:'/library/review',group:'library',label:'待检查素材',mode:'review'},
  {path:'/rooms/all',group:'rooms',label:'直播间档案',mode:'all'},
  {path:'/rooms/errors',group:'rooms',label:'资料更新异常',mode:'errors'},
  {path:'/pipeline/current',group:'pipeline',label:'合并与切割',mode:'current'},
  {path:'/pipeline/history',group:'pipeline',label:'已保存计划',mode:'history'},
  {path:'/pipeline/workflows',group:'pipeline',label:'自定义管线',mode:'workflows'},
  {path:'/youtube/videos',group:'youtube',label:'频道视频',mode:'videos'},
  {path:'/youtube/upload',group:'youtube',label:'上传准备',mode:'upload'},
  {path:'/youtube/history',group:'youtube',label:'批量变更记录',mode:'history'},
  {path:'/tasks/all',group:'tasks',label:'全部任务',mode:'all'},
  {path:'/tasks/active',group:'tasks',label:'进行中与等待',mode:'active'},
  {path:'/tasks/attention',group:'tasks',label:'需要处理',mode:'attention'},
  {path:'/tasks/completed',group:'tasks',label:'已完成',mode:'completed'},
  {path:'/settings/appearance',group:'settings',label:'界面外观',mode:'appearance'},
  {path:'/settings/connections',group:'settings',label:'YouTube 账号',mode:'account'},
  {path:'/settings/environment',group:'settings',label:'运行环境',mode:'environment'},
] as const;
export type AppRoute = typeof routes[number];
export function resolveRoute(hash:string):AppRoute {
  const path=hash.replace(/^#/,'').split('?')[0];
  return routes.find(route=>route.path===path)??routes[0];
}
