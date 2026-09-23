/**
 * Example declarative module package (share as u2bup-module.json).
 * Install via 自定义管线 → 模块管理, or POST /api/workflows/modules/install.
 */
export default {
  id: 'com.example.tag-dictionary',
  version: '1.0.0',
  apiVersion: 1,
  kind: 'declarative',
  manifest: {
    id: 'com.example.tag-dictionary',
    version: '1.0.0',
    apiVersion: 1,
    kind: 'declarative',
    alias: 'tag-dictionary',
    label: '示例标签词典',
    description: '声明式关键词 → 内容标签，并可合并到 YouTube tags。',
    group: '修改',
    color: '#6aa84f',
    capabilities: ['read.record', 'write.fields:labels,tags'],
    fields: [
      { key: 'includeLabels', label: '写入内容标签到 YouTube tags', type: 'checkbox' },
      { key: 'extraTags', label: '额外标签', type: 'textarea' },
    ],
    defaults: { includeLabels: true, extraTags: '' },
    ports: ['out'],
    sideEffect: 'pure',
  },
  rules: {
    keywords: [
      { label: 'Dance', keywords: ['跳舞', '热舞', 'dance'] },
      { label: 'ASMR', keywords: ['asmr', '助眠'] },
      { label: '汉服', keywords: ['汉服', '常服'] },
    ],
    includeLabelsAsTags: true,
    extraTags: ['直播录像'],
  },
};
