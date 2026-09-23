export interface ConfigField {
  key: string;
  label: string;
  type: 'text' | 'textarea' | 'select' | 'checkbox' | 'number';
  options?: { value: string; label: string }[];
  help?: string;
}

export type ModuleKind = 'builtin' | 'declarative';
export type ModuleSource = 'builtin' | 'local';
export type ModuleApiVersion = 1;

/** Manifest shipped inside a shareable package (u2bup-module.json). */
export interface ModuleManifest {
  id: string;
  version: string;
  apiVersion: ModuleApiVersion;
  kind: ModuleKind;
  /** Short canvas type such as `parse`; optional for pure declarative packages. */
  alias?: string;
  label: string;
  description: string;
  group: string;
  color: string;
  capabilities: string[];
  fields: ConfigField[];
  defaults: Record<string, unknown>;
  /** Ports the node exposes; filter uses yes/no. */
  ports?: Array<'out' | 'yes' | 'no'>;
  sideEffect?: 'pure';
}

export interface DeclarativeKeywordRule {
  label: string;
  keywords: string[];
}

export interface DeclarativeRules {
  /** Append labels when any keyword matches title/description/tags. */
  keywords?: DeclarativeKeywordRule[];
  /** Optional title template using {主播}{主题}{分片}{日期}{平台}{房间号}. */
  titleTemplate?: string;
  writeTitle?: boolean;
  writeDescription?: boolean;
  onlyEmptyDescription?: boolean;
  allowUnconfirmed?: boolean;
  descriptionFooter?: string;
  /** Extra YouTube tags to merge. */
  extraTags?: string[];
  includeLabelsAsTags?: boolean;
  includeCreatorAsTag?: boolean;
  /** Map source field values onto another field (simple equality). */
  fieldMaps?: Array<{ from: string; when: string; set: string; value: string }>;
  /** Identity ledger rows packaged with the plugin. */
  identities?: Array<{ platform: string; roomId: string; creator: string; aliases?: string[]; playlistId?: string }>;
}

export interface ModulePackageFile {
  id: string;
  version: string;
  apiVersion: ModuleApiVersion;
  kind: 'declarative';
  manifest: ModuleManifest;
  rules: DeclarativeRules;
}

export interface InstalledModule {
  id: string;
  version: string;
  apiVersion: ModuleApiVersion;
  kind: ModuleKind;
  alias?: string;
  enabled: boolean;
  source: ModuleSource;
  trusted: boolean;
  installedAt: string;
  manifest: ModuleManifest;
  rules?: DeclarativeRules;
}

export interface ModuleRegistryState {
  modules: InstalledModule[];
  disabledBuiltinIds: string[];
}

/** Host-recognized intents. Plugins may only emit these; apply is host-owned. */
export type HostIntent =
  | {
      kind: 'playlist.add';
      playlistId: string;
      identityKey: string;
      title: string;
      description: string;
      reason: string;
    }
  | {
      kind: 'playlist.create';
      identityKey: string;
      title: string;
      description: string;
      reason: string;
    }
  | {
      kind: 'playlist.review';
      identityKey: string;
      title: string;
      description: string;
      reason: string;
    }
  | {
      kind: 'thumbnail.setFromLocalFrame';
      assetId: string;
      seconds: number;
      reason: string;
    }
  | {
      kind: 'identity.upsert';
      platform: string;
      roomId: string;
      creator: string;
      aliases?: string[];
      playlistId?: string;
      reason: string;
    }
  | {
      kind: 'agent.task';
      task: string;
      input: Record<string, unknown>;
      outputSchema: Record<string, unknown>;
      reason: string;
    }
  | {
      kind: 'publish.settings';
      privacy?: string;
      categoryId?: string;
      reason: string;
      status: 'review';
    };

export function playlistIntentFromHost(intent: HostIntent): {
  action: 'add' | 'create' | 'review';
  playlistId?: string;
  identityKey: string;
  title: string;
  description: string;
  reason: string;
} | null {
  if (intent.kind === 'playlist.add') {
    return {
      action: 'add',
      playlistId: intent.playlistId,
      identityKey: intent.identityKey,
      title: intent.title,
      description: intent.description,
      reason: intent.reason,
    };
  }
  if (intent.kind === 'playlist.create') {
    return {
      action: 'create',
      identityKey: intent.identityKey,
      title: intent.title,
      description: intent.description,
      reason: intent.reason,
    };
  }
  if (intent.kind === 'playlist.review') {
    return {
      action: 'review',
      identityKey: intent.identityKey,
      title: intent.title,
      description: intent.description,
      reason: intent.reason,
    };
  }
  return null;
}

export function hostIntentFromPlaylist(intent: {
  action: string;
  playlistId?: string;
  identityKey: string;
  title: string;
  description: string;
  reason: string;
}): HostIntent {
  if (intent.action === 'add' && intent.playlistId) {
    return {
      kind: 'playlist.add',
      playlistId: intent.playlistId,
      identityKey: intent.identityKey,
      title: intent.title,
      description: intent.description,
      reason: intent.reason,
    };
  }
  if (intent.action === 'create') {
    return {
      kind: 'playlist.create',
      identityKey: intent.identityKey,
      title: intent.title,
      description: intent.description,
      reason: intent.reason,
    };
  }
  return {
    kind: 'playlist.review',
    identityKey: intent.identityKey,
    title: intent.title,
    description: intent.description,
    reason: intent.reason,
  };
}

export const KNOWN_INTENT_KINDS = [
  'metadata.patch',
  'playlist.add',
  'playlist.create',
  'playlist.review',
  'playlist.updateSnippet',
  'thumbnail.setFromLocalFrame',
  'local.metadata.save',
  'identity.upsert',
  'agent.task',
  'publish.settings',
] as const;

export function isKnownIntentKind(kind: string): boolean {
  return (KNOWN_INTENT_KINDS as readonly string[]).includes(kind);
}
