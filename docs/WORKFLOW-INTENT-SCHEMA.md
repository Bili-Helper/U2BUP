# Host Intent Schema (moduleApi 1)

Plugins and builtin preview modules may only **emit** these intents. The host apply path in `workflow.rs` executes a subset; unrecognized kinds fail freeze/apply rather than running arbitrary code.

完整模块功能说明、YouTube Data API v3 对照与第三方 L1 开发指南见 [WORKFLOW-MODULE-API.md](WORKFLOW-MODULE-API.md)。

## Kinds

| kind | Emitted by | Apply status |
| --- | --- | --- |
| `metadata.patch` | metadata / tags (via field diffs) | Implemented as videos.update / local fields |
| `playlist.add` | playlist | Implemented |
| `playlist.create` | playlist | Implemented (private list) |
| `playlist.review` | playlist | Preview only |
| `playlist.updateSnippet` | playlist (name/description) | Implemented with add/create |
| `thumbnail.setFromLocalFrame` | thumbnail | Implemented when local MP4 linked |
| `local.metadata.save` | output / host | Implemented as workflow-metadata |
| `identity.upsert` | identity / declarative | Saved to identity ledger; not a YouTube API call |
| `agent.task` | ai | Preview only until a provider is wired |
| `publish.settings` | future publish module | Review placeholder |

## Shape (TypeScript)

See `app/web/src/modules/types.ts` `HostIntent`. Playlist intents remain mirrored as `PlaylistIntent` for the existing freeze payload.

## Rules

1. Preview never performs network or filesystem side effects.
2. Apply validates channel, previous values, and ETag before remote writes.
3. A disabled or unknown module type blocks preview with `module_disabled`.
4. Third-party packages are declarative (L1) only in this release.
