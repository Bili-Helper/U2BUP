import assert from 'node:assert/strict';
import {test} from 'node:test';
import {selectIds,toggleId,selectionCounts} from '../src/selection.ts';
import {routes,resolveRoute} from '../src/navigation.ts';

test('selection survives filters and page additions without selecting future matches',()=>{
  const original=new Set(['hidden','a']);
  const selected=selectIds(original,['a','b'],'add');
  assert.deepEqual([...original],['hidden','a']);
  assert.deepEqual([...selected],['hidden','a','b']);
  assert.deepEqual(selectionCounts(selected,['a','b','new'],['b','new']),{total:3,matched:2,hidden:1,pageCount:1});
  assert.deepEqual([...selectIds(selected,['a','b'],'remove')],['hidden']);
});
test('invert affects only the current result set',()=>{
  assert.deepEqual([...selectIds(new Set(['hidden','a']),['a','b'],'invert')],['hidden','b']);
});
test('shift selection works in both directions and resets when anchor is filtered out',()=>{
  const order=['a','b','c','d'];
  assert.deepEqual([...toggleId(new Set(['outside']),'b',order,'d',true)],['outside','b','c','d']);
  assert.deepEqual([...toggleId(new Set(['a','b','c','d']),'d',order,'b',true)],['a']);
  assert.deepEqual([...toggleId(new Set(),'c',order,'missing',true)],['c']);
});
test('deep links resolve deterministically, invalid and token hashes fall back to library',()=>{
  for(const route of routes)assert.equal(resolveRoute('#'+route.path),route);
  assert.equal(resolveRoute('#/tasks/attention?from=upload').mode,'attention');
  for(const invalid of ['#token=private','/unknown',''])assert.equal(resolveRoute(invalid).path,'/library/all');
  assert.equal(new Set(routes.map(r=>r.path)).size,routes.length);
});
