import assert from 'node:assert/strict';
import {test} from 'node:test';
import {
  applyRegistryState,
  createNode,
  createTemplate,
  exampleTagDictionaryPackage,
  installLocalPackage,
  isKnownIntentKind,
  listAllModules,
  normalizeYoutube,
  parseModulePackage,
  runWorkflow,
  setBuiltinEnabled,
  uninstallLocalModule,
  validateGraph,
} from '../src/workflow.ts';

function video(id, title, description = '') {
  return normalizeYoutube({
    id,
    snippet: { title, description, tags: [], categoryId: '20' },
    status: { privacyStatus: 'private' },
    contentDetails: { duration: 'PT1H' },
  });
}

test('builtin modules register with stable ids and short aliases', () => {
  const modules = listAllModules().filter((m) => m.source === 'builtin');
  assert.ok(modules.length >= 14);
  assert.ok(modules.some((m) => m.id === 'com.u2bup.builtin.parse' && m.type === 'parse'));
  assert.ok(modules.some((m) => m.id === 'com.u2bup.builtin.identity'));
  assert.equal(validateGraph(createTemplate('standard')).valid, true);
  const full = createNode('com.u2bup.builtin.tags', 3);
  assert.equal(full.type, 'tags');
});

test('disabling a builtin blocks graphs that still reference it', () => {
  setBuiltinEnabled('com.u2bup.builtin.parse', false);
  try {
    const result = validateGraph(createTemplate('standard'));
    assert.equal(result.valid, false);
    assert.ok(result.errors.some((e) => e.includes('已禁用')));
  } finally {
    setBuiltinEnabled('com.u2bup.builtin.parse', true);
  }
});

test('declarative L1 package installs, appears in catalog, and writes tags without network', () => {
  applyRegistryState({ modules: [], disabledBuiltinIds: [] });
  const pkg = exampleTagDictionaryPackage();
  assert.deepEqual(parseModulePackage(pkg).id, pkg.id);
  installLocalPackage(pkg, true);
  assert.ok(listAllModules().some((m) => m.id === pkg.id && m.enabled));
  const nodes = [
    createNode('input', 0),
    createNode('parse', 1),
    createNode('tag-dictionary', 2),
    createNode('output', 3),
  ];
  const graph = {
    version: 1,
    moduleApi: 1,
    id: 'plugin-test',
    name: '插件测试',
    nodes,
    edges: nodes.slice(1).map((n, i) => ({ id: `e${i}`, source: nodes[i].id, target: n.id, port: 'out' })),
  };
  assert.equal(validateGraph(graph).valid, true);
  const [result] = runWorkflow(graph, [video('v1', '今晚跳舞热舞')]).records;
  assert.ok(result.after.labels.includes('Dance'));
  assert.ok(result.after.tags.includes('Dance'));
  assert.ok(result.after.tags.includes('直播录像'));
  uninstallLocalModule(pkg.id);
  assert.equal(listAllModules().some((m) => m.id === pkg.id), false);
});

test('identity ledger module confirms room bindings and emits identity.upsert intent', () => {
  const nodes = ['input', 'parse', 'identity', 'output'].map((type, index) => createNode(type, index));
  const graph = {
    version: 1,
    id: 'identity-test',
    name: '身份',
    nodes,
    edges: nodes.slice(1).map((n, i) => ({ id: `e${i}`, source: nodes[i].id, target: n.id, port: 'out' })),
  };
  const source = video('id1', '【旧名】 999 20260901 ASMR', 'https://live.bilibili.com/999');
  const [result] = runWorkflow(graph, [source], {
    identityLedger: [{ platform: 'bilibili', roomId: '999', creator: '台账主播', aliases: ['旧名'] }],
  }).records;
  assert.equal(result.after.creator, '台账主播');
  assert.equal(result.after.platform, 'bilibili');
  assert.ok(result.intents.some((i) => i.kind === 'identity.upsert' && i.creator === '台账主播'));
  assert.ok(isKnownIntentKind('identity.upsert'));
});
