import type { ConfigField, InstalledModule, ModuleManifest, ModulePackageFile, ModuleRegistryState } from './types.ts';
import { applyDeclarativeRules, type DeclarativeRecord } from './declarative.ts';
import type { HostIntent } from './types.ts';

export interface NodeDefinitionView {
  type: string;
  id: string;
  label: string;
  description: string;
  group: string;
  color: string;
  fields: ConfigField[];
  defaults: Record<string, unknown>;
  capabilities: string[];
  kind: 'builtin' | 'declarative';
  enabled: boolean;
  source: 'builtin' | 'local';
  alias?: string;
}

type BuiltinHandler = (
  record: DeclarativeRecord,
  config: Record<string, unknown>,
  context: Record<string, unknown>,
  nodeId: string,
) => HostIntent[] | void;

interface BuiltinSpec {
  manifest: ModuleManifest;
  handler: BuiltinHandler;
}

const builtins = new Map<string, BuiltinSpec>();
let localModules: InstalledModule[] = [];
let disabledBuiltinIds = new Set<string>();

export function registerBuiltin(manifest: ModuleManifest, handler: BuiltinHandler) {
  builtins.set(manifest.id, { manifest, handler });
}

export function resolveModuleType(type: string): { id: string; alias?: string } | null {
  if (builtins.has(type)) return { id: type, alias: builtins.get(type)!.manifest.alias };
  for (const [id, spec] of builtins) {
    if (spec.manifest.alias === type) return { id, alias: type };
  }
  const local = localModules.find((m) => m.id === type || m.alias === type || m.manifest.alias === type);
  if (local) return { id: local.id, alias: local.alias ?? local.manifest.alias };
  return null;
}

export function isModuleEnabled(type: string): boolean {
  const resolved = resolveModuleType(type);
  if (!resolved) return false;
  if (builtins.has(resolved.id)) return !disabledBuiltinIds.has(resolved.id);
  const local = localModules.find((m) => m.id === resolved.id);
  return Boolean(local?.enabled);
}

export function getModuleDefinition(type: string): NodeDefinitionView | undefined {
  const resolved = resolveModuleType(type);
  if (!resolved) return undefined;
  if (builtins.has(resolved.id)) {
    const spec = builtins.get(resolved.id)!;
    return {
      type: spec.manifest.alias ?? spec.manifest.id,
      id: spec.manifest.id,
      label: spec.manifest.label,
      description: spec.manifest.description,
      group: spec.manifest.group,
      color: spec.manifest.color,
      fields: spec.manifest.fields,
      defaults: spec.manifest.defaults,
      capabilities: spec.manifest.capabilities,
      kind: 'builtin',
      enabled: !disabledBuiltinIds.has(spec.manifest.id),
      source: 'builtin',
      alias: spec.manifest.alias,
    };
  }
  const local = localModules.find((m) => m.id === resolved.id);
  if (!local) return undefined;
  return {
    type: local.alias ?? local.manifest.alias ?? local.id,
    id: local.id,
    label: local.manifest.label,
    description: local.manifest.description,
    group: local.manifest.group,
    color: local.manifest.color,
    fields: local.manifest.fields,
    defaults: local.manifest.defaults,
    capabilities: local.manifest.capabilities,
    kind: 'declarative',
    enabled: local.enabled,
    source: 'local',
    alias: local.alias ?? local.manifest.alias,
  };
}

/** Catalog for the canvas — enabled modules only, keyed by short type for builtins. */
export function listEnabledCatalog(): NodeDefinitionView[] {
  const list: NodeDefinitionView[] = [];
  for (const [id] of builtins) {
    const def = getModuleDefinition(id);
    if (def?.enabled) list.push(def);
  }
  for (const local of localModules) {
    if (!local.enabled) continue;
    const def = getModuleDefinition(local.id);
    if (def) list.push(def);
  }
  return list;
}

export function listAllModules(): NodeDefinitionView[] {
  const list: NodeDefinitionView[] = [];
  for (const [id] of builtins) {
    const def = getModuleDefinition(id);
    if (def) list.push(def);
  }
  for (const local of localModules) {
    const def = getModuleDefinition(local.id);
    if (def) list.push(def);
  }
  return list;
}

export function runModuleHandler(
  type: string,
  record: DeclarativeRecord,
  config: Record<string, unknown>,
  context: Record<string, unknown>,
  nodeId: string,
): HostIntent[] {
  const resolved = resolveModuleType(type);
  if (!resolved || !isModuleEnabled(type)) {
    record.issues.push({
      code: 'module_disabled',
      message: `模块 ${type} 未安装或已禁用`,
      severity: 'error',
      nodeId,
    });
    return [];
  }
  if (builtins.has(resolved.id)) {
    const result = builtins.get(resolved.id)!.handler(record, config, context, nodeId);
    return result ? [...result] : [];
  }
  const local = localModules.find((m) => m.id === resolved.id);
  if (local?.rules) return applyDeclarativeRules(record, local.rules, { ...local.manifest.defaults, ...config }, nodeId);
  return [];
}

export function applyRegistryState(state: ModuleRegistryState | null | undefined) {
  localModules = Array.isArray(state?.modules) ? state!.modules.filter((m) => m.source === 'local') : [];
  disabledBuiltinIds = new Set(state?.disabledBuiltinIds ?? []);
}

export function exportRegistryState(): ModuleRegistryState {
  return {
    modules: localModules.map((m) => ({ ...m })),
    disabledBuiltinIds: [...disabledBuiltinIds],
  };
}

export function setBuiltinEnabled(id: string, enabled: boolean) {
  const resolved = resolveModuleType(id);
  if (!resolved || !builtins.has(resolved.id)) throw new Error(`未知内置模块：${id}`);
  if (enabled) disabledBuiltinIds.delete(resolved.id);
  else disabledBuiltinIds.add(resolved.id);
}

export function installLocalPackage(file: ModulePackageFile, trusted = true): InstalledModule {
  if (file.apiVersion !== 1) throw new Error('仅支持 apiVersion: 1 的模块包');
  if (file.kind !== 'declarative') throw new Error('本地安装目前仅支持声明式（L1）模块');
  if (!file.id?.trim() || file.id.length > 160) throw new Error('无效模块 ID');
  if (builtins.has(file.id) || [...builtins.values()].some((b) => b.manifest.alias === file.id)) {
    throw new Error('不能覆盖内置模块 ID');
  }
  if (!file.manifest?.label || !file.rules) throw new Error('模块包缺少 manifest 或 rules');
  const installed: InstalledModule = {
    id: file.id,
    version: file.version || '1.0.0',
    apiVersion: 1,
    kind: 'declarative',
    alias: file.manifest.alias || file.id.replace(/[^a-zA-Z0-9_-]/g, '-').slice(0, 40),
    enabled: true,
    source: 'local',
    trusted,
    installedAt: new Date().toISOString(),
    manifest: { ...file.manifest, id: file.id, apiVersion: 1, kind: 'declarative' },
    rules: file.rules,
  };
  localModules = [...localModules.filter((m) => m.id !== installed.id), installed];
  return installed;
}

export function uninstallLocalModule(id: string) {
  const before = localModules.length;
  localModules = localModules.filter((m) => m.id !== id);
  if (localModules.length === before) throw new Error(`未找到本地模块：${id}`);
}

export function setLocalModuleEnabled(id: string, enabled: boolean) {
  const mod = localModules.find((m) => m.id === id);
  if (!mod) throw new Error(`未找到本地模块：${id}`);
  mod.enabled = enabled;
}

export function parseModulePackage(value: unknown): ModulePackageFile {
  const raw = value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
  if (!raw) throw new Error('模块包必须是 JSON 对象');
  if (raw.apiVersion !== 1) throw new Error('仅支持 apiVersion: 1');
  if (raw.kind !== 'declarative') throw new Error('仅支持 kind: declarative');
  if (typeof raw.id !== 'string' || !raw.id.trim()) throw new Error('缺少模块 id');
  const manifest = raw.manifest && typeof raw.manifest === 'object' ? (raw.manifest as ModuleManifest) : null;
  const rules = raw.rules && typeof raw.rules === 'object' ? (raw.rules as ModulePackageFile['rules']) : null;
  if (!manifest || !rules) throw new Error('模块包需要 manifest 与 rules');
  return {
    id: raw.id,
    version: typeof raw.version === 'string' ? raw.version : '1.0.0',
    apiVersion: 1,
    kind: 'declarative',
    manifest: {
      ...manifest,
      id: raw.id,
      apiVersion: 1,
      kind: 'declarative',
      fields: Array.isArray(manifest.fields) ? manifest.fields : [],
      defaults: manifest.defaults && typeof manifest.defaults === 'object' ? manifest.defaults : {},
      capabilities: Array.isArray(manifest.capabilities) ? manifest.capabilities : ['read.record'],
    },
    rules,
  };
}

export function exampleTagDictionaryPackage(): ModulePackageFile {
  return {
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
}
