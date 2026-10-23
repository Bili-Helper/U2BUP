import {test} from 'node:test';
import assert from 'node:assert/strict';
import {cleanupCsv,isLocalVideoDeleted} from '../src/cleanup-view.ts';
test('cleanup CSV preserves Unicode and quotes, prevents spreadsheet formulas, and includes excluded rows',()=>{
 const csv=cleanupCsv({id:'one',created_at:'today',method:'user_confirmed',platform:'YouTube',note:'=1+1',items:[{asset_id:'1',path:'中文,"视频".mp4',bytes:12,status:'excluded',message:'保留\n例外'}]});
 assert.ok(csv.startsWith('\ufeff'));assert.ok(csv.includes('"中文,""视频"".mp4"'));assert.ok(csv.includes('"\'\=1+1"'));assert.ok(csv.includes('"excluded"'));assert.ok(csv.includes('"保留\n例外"'));
});
test('local cleanup is distinguished from unrelated warnings',()=>{
 assert.ok(isLocalVideoDeleted({warnings:['本地视频已清理；弹幕和元数据保留']}));
 assert.ok(isLocalVideoDeleted({warnings:['文件状态：local_deleted；请重新扫描素材库']}));
 assert.equal(isLocalVideoDeleted({warnings:['文件状态：missing']}),false);
});
