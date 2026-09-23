import type { DeclarativeRules, HostIntent } from './types.ts';

export interface DeclarativeRecord {
  title: string;
  description: string;
  tags: string[];
  labels: string[];
  creator: string;
  roomId: string;
  platform: string;
  recordedAt: string;
  recordedEnd: string;
  sessionTitle: string;
  part: string;
  sourceUrl: string;
  originalTitle: string;
  technicalFields: string[];
  evidence: Array<{ field: string; value: string; source: string; confidence: 'high' | 'medium' | 'low' }>;
  issues: Array<{ code: string; message: string; severity: 'info' | 'review' | 'error'; nodeId?: string }>;
  [key: string]: unknown;
}

const unique = (values: string[]) =>
  [...new Map(values.map((x) => [x.trim().toLocaleLowerCase(), x.trim()])).values()].filter(Boolean);

function addIssue(
  record: DeclarativeRecord,
  code: string,
  message: string,
  severity: 'info' | 'review' | 'error' = 'review',
  nodeId?: string,
) {
  if (!record.issues.some((i) => i.code === code && i.message === message)) {
    record.issues.push({ code, message, severity, ...(nodeId ? { nodeId } : {}) });
  }
}

function evidence(
  record: DeclarativeRecord,
  field: string,
  value: string,
  source: string,
  confidence: 'high' | 'medium' | 'low',
) {
  if (!record.evidence.some((e) => e.field === field && e.value === value && e.source === source)) {
    record.evidence.push({ field, value, source, confidence });
  }
}

/** Pure L1 rule application — no network, no filesystem. */
export function applyDeclarativeRules(
  record: DeclarativeRecord,
  rules: DeclarativeRules,
  config: Record<string, unknown>,
  nodeId: string,
): HostIntent[] {
  const intents: HostIntent[] = [];
  const text = `${record.title}\n${record.description}\n${record.tags.join(' ')}`.toLocaleLowerCase();

  for (const rule of rules.keywords ?? []) {
    if (rule.keywords.some((keyword) => text.includes(keyword.toLocaleLowerCase()))) {
      record.labels = unique([...record.labels, rule.label]);
      evidence(record, 'labels', rule.label, '声明式插件关键词规则', 'medium');
    }
  }

  for (const map of rules.fieldMaps ?? []) {
    const current = String((record as Record<string, unknown>)[map.from] ?? '');
    if (current === map.when) {
      (record as Record<string, unknown>)[map.set] = map.value;
      evidence(record, map.set, map.value, `声明式字段映射 ${map.from}=${map.when}`, 'medium');
    }
  }

  for (const identity of rules.identities ?? []) {
    if (record.roomId && identity.roomId === record.roomId) {
      const platformOk = record.platform === 'unknown' || record.platform === identity.platform;
      if (platformOk) {
        record.platform = identity.platform;
        if (!record.creator) record.creator = identity.creator;
        evidence(record, 'platform', identity.platform, '声明式插件身份台账', 'high');
        evidence(record, 'creator', identity.creator, '声明式插件身份台账', 'high');
        intents.push({
          kind: 'identity.upsert',
          platform: identity.platform,
          roomId: identity.roomId,
          creator: identity.creator,
          aliases: identity.aliases,
          playlistId: identity.playlistId,
          reason: '声明式插件提供的来源身份',
        });
      }
    }
  }

  const writeTitle = Boolean(config.writeTitle ?? rules.writeTitle);
  const writeDescription = Boolean(config.writeDescription ?? rules.writeDescription);
  const onlyEmpty = Boolean(config.onlyEmptyDescription ?? rules.onlyEmptyDescription);
  const allowUnconfirmed = Boolean(config.allowUnconfirmed ?? rules.allowUnconfirmed);
  const template = String(config.titleTemplate ?? rules.titleTemplate ?? '');
  const footer = String(config.footer ?? rules.descriptionFooter ?? '').trim();

  if (writeTitle && template) {
    const low =
      record.evidence.some((e) => e.field === 'creator' && e.value === record.creator && e.confidence === 'low') &&
      !record.evidence.some((e) => e.field === 'creator' && e.value === record.creator && e.confidence !== 'low');
    if (!record.creator || !record.sessionTitle || (low && !allowUnconfirmed)) {
      addIssue(record, 'title_needs_identity', '标题未改：声明式模板需要主播与场次主题', 'review', nodeId);
    } else {
      const vars: Record<string, string> = {
        主播: record.creator,
        主题: record.sessionTitle,
        分片: record.part,
        日期: record.recordedAt,
        平台: record.platform === 'unknown' ? '' : record.platform,
        房间号: record.roomId,
      };
      const title = template
        .replace(/\{([^}]+)\}/g, (_, key: string) => vars[key] ?? '')
        .replace(/\s+/g, ' ')
        .trim();
      if (!title || [...title].length > 100 || /[<>]/.test(title)) {
        addIssue(record, 'title_limit', '声明式标题无效或超长', 'review', nodeId);
      } else record.title = title;
    }
  }

  if (writeDescription && (!onlyEmpty || !record.description.trim())) {
    const lines = [
      ['主播', record.creator],
      ['平台', record.platform === 'unknown' ? '' : record.platform],
      ['录像时间', record.recordedAt],
      ['录像结束', record.recordedEnd],
      ['场次主题', record.sessionTitle],
      ['房间号', record.roomId],
      ['分片', record.part],
      ['来源', record.sourceUrl],
      ['技术标记', record.technicalFields.join(' / ')],
      ['原始标题', record.originalTitle],
    ]
      .filter(([, v]) => v)
      .map(([k, v]) => `${k}：${v}`);
    if (footer) lines.push(footer);
    const block = `[U2BUP 元信息]\n${lines.join('\n')}\n[/U2BUP 元信息]`;
    const existing = record.description
      .replace(/\n*\[U2BUP 元信息\]\n[\s\S]*?\n\[\/U2BUP 元信息\]\n*/g, '\n')
      .trim();
    const next = [existing, block].filter(Boolean).join('\n\n');
    if ([...next].length > 5000) addIssue(record, 'description_limit', '描述超过 5000 字符', 'review', nodeId);
    else record.description = next;
  }

  const includeLabels = config.includeLabels !== undefined ? Boolean(config.includeLabels) : Boolean(rules.includeLabelsAsTags ?? true);
  const includeCreator = config.includeCreator !== undefined ? Boolean(config.includeCreator) : Boolean(rules.includeCreatorAsTag ?? false);
  const extraSource =
    typeof config.extraTags === 'string' && config.extraTags.trim()
      ? config.extraTags
      : Array.isArray(rules.extraTags)
        ? rules.extraTags.join(',')
        : '';
  const extra = extraSource
    .split(/[,，\n]/)
    .map((x) => x.trim())
    .filter(Boolean);
  if (includeLabels || includeCreator || extra.length) {
    const tags = unique([
      ...record.tags,
      ...(includeLabels ? record.labels : []),
      ...(includeCreator && record.creator ? [record.creator] : []),
      ...extra,
    ]);
    const length = tags.reduce((sum, tag) => sum + tag.length + (tag.includes(' ') ? 2 : 0), Math.max(0, tags.length - 1));
    if (length > 500) addIssue(record, 'tags_limit', '合并标签超过 500 字符预算', 'review', nodeId);
    else record.tags = tags;
  }

  return intents;
}
