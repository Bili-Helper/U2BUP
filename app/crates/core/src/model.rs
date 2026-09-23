use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Legacy in-memory library model (liverec; unchanged for v0.7 compatibility) ──

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Library {
    pub scanned_at: Option<String>,
    pub root: String,
    pub assets: Vec<Asset>,
    pub rooms: Vec<Room>,
    pub sidecar_count: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub directories: Vec<String>,
    pub historical_title: Option<String>,
    pub online: Option<Value>,
    pub refresh_error: Option<String>,
    pub refreshed_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Asset {
    pub id: String,
    pub relative_path: String,
    pub name: String,
    pub room_id: String,
    pub room_name: String,
    pub title: String,
    pub display_title: Option<String>,
    pub bytes: u64,
    pub modified_ms: u64,
    pub extension: String,
    pub role: String,
    pub started_at: Option<String>,
    pub time_source: String,
    pub metadata: Option<MediaInfo>,
    pub sidecars: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub aspect: String,
    pub signature: String,
    pub streams: Vec<Value>,
}

// ── Multi-library models (v0.8+) ──────────────────────────────────────────────

/// The kind of a library root, determines which scanner is used.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    /// BililiveRecorder-compatible directory (`{room_id}-{name}/` + XML).
    Liverec,
    /// Arbitrary folder; any video file is indexed.
    Folder,
    /// Locally-mounted WebDAV path (read-only).
    Webdav,
    /// Locally-mounted OpenList / Alist path (read-only).
    Openlist,
}

impl LibraryKind {
    pub fn is_readonly(&self) -> bool {
        matches!(self, Self::Webdav | Self::Openlist)
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Liverec => "录播库",
            Self::Folder => "文件夹",
            Self::Webdav => "WebDAV 挂载",
            Self::Openlist => "OpenList 挂载",
        }
    }
}

/// A persisted library root entry (stored in the `libraries` table).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRoot {
    pub id: String,
    pub name: String,
    pub kind: LibraryKind,
    /// Absolute path to the root directory (local mount point for webdav/openlist).
    pub path: String,
    /// IANA timezone for display, e.g. "Asia/Shanghai".
    #[serde(default = "default_tz")]
    pub display_tz: String,
    /// Force read-only even for folder/liverec.
    #[serde(default)]
    pub readonly: bool,
    #[serde(default = "bool_true")]
    pub enabled: bool,
    /// JSON array of glob patterns to exclude during scan.
    #[serde(default)]
    pub scan_exclude: Vec<String>,
    pub created_at: String,
    #[serde(default)]
    pub last_scanned_at: Option<String>,
    /// "idle" | "scanning" | "offline" | "error"
    #[serde(default = "idle")]
    pub scan_status: String,
    #[serde(default)]
    pub scan_error: Option<String>,
    #[serde(default)]
    pub asset_count: usize,
}

fn default_tz() -> String {
    "Asia/Shanghai".into()
}
fn bool_true() -> bool {
    true
}
fn idle() -> String {
    "idle".into()
}

/// Enriched asset row for the `assets_v2` table.
/// The inner `asset` carries all fields; the outer fields are indexed columns.
#[derive(Clone, Debug)]
pub struct AssetRecord {
    pub id: String,
    pub library_id: String,
    pub source_path: String,
    pub content_hash: Option<String>,
    /// Redundant column for full-text / prefix search.
    pub pub_title: Option<String>,
    /// JSON array string, e.g. `["直播","ASMR"]`.
    pub custom_tags_json: Option<String>,
    pub modified_ms: u64,
    pub file_size: u64,
    /// Full serialised `AssetV2`.
    pub asset: AssetV2,
}

/// The video-file asset model for v0.8+.
/// Compatible with the legacy `Asset` shape so the workflow layer can consume it
/// via `kind = "local"`.
#[derive(Clone, Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AssetV2 {
    // ── identity ────────────────────────────────────────────────────────
    pub id: String,
    pub library_id: String,
    /// Absolute path on the local filesystem.
    pub source_path: String,
    /// SHA-256 of first 1 MiB + file size (hex, first 16 chars).
    #[serde(default)]
    pub content_hash: Option<String>,
    /// True if the source file lives outside the workspace (i.e. is a reference).
    #[serde(default)]
    pub is_linked: bool,

    // ── file attributes ─────────────────────────────────────────────────
    pub file_size: u64,
    pub modified_ms: u64,
    pub extension: String,

    // ── display names ───────────────────────────────────────────────────
    /// Original filename stem (immutable after first scan).
    pub title: String,
    /// UI display name (user-editable).
    #[serde(default)]
    pub display_title: Option<String>,

    // ── liverec-compat fields ────────────────────────────────────────────
    /// Only set for liverec assets.
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub room_name: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub time_source: Option<String>,
    #[serde(default)]
    pub sidecars: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,

    // ── media tech info (filled by ffprobe async) ────────────────────────
    #[serde(default)]
    pub duration_sec: Option<f64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub video_codec: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,

    // ── publish metadata (user-editable, mirrors YouTube scheme) ─────────
    #[serde(default)]
    pub pub_title: Option<String>,
    #[serde(default)]
    pub pub_description: Option<String>,
    #[serde(default)]
    pub pub_tags: Vec<String>,
    #[serde(default = "category_22")]
    pub pub_category_id: String,
    #[serde(default = "private")]
    pub pub_privacy: String,
    #[serde(default)]
    pub pub_language: Option<String>,
    #[serde(default)]
    pub pub_audio_lang: Option<String>,

    // ── classification ───────────────────────────────────────────────────
    /// User-defined classification tags (not YouTube tags).
    #[serde(default)]
    pub custom_tags: Vec<String>,
    #[serde(default)]
    pub custom_meta: Option<Value>,

    // ── upload state ─────────────────────────────────────────────────────
    /// Array of `{platform, status, url, uploaded_at}`.
    #[serde(default)]
    pub upload_targets: Vec<Value>,

    // ── scan state ───────────────────────────────────────────────────────
    /// "ok" | "missing" | "changed"
    #[serde(default = "ok_status")]
    pub file_status: String,
}

fn category_22() -> String {
    "22".into()
}
fn private() -> String {
    "private".into()
}
fn ok_status() -> String {
    "ok".into()
}

// ── Planning / Job models (unchanged) ─────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub asset_ids: Vec<String>,
    #[serde(default = "default_duration")]
    pub max_duration: f64,
    #[serde(default = "default_bytes")]
    pub max_bytes: u64,
    #[serde(default = "default_gap")]
    pub max_gap: f64,
    #[serde(default)]
    pub include_legacy: bool,
}
fn default_duration() -> f64 {
    42900.0
}
fn default_bytes() -> u64 {
    250_000_000_000
}
fn default_gap() -> f64 {
    1800.0
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub created_at: String,
    pub request: PlanRequest,
    pub outputs: Vec<PlannedOutput>,
    pub blocked: Vec<BlockedAsset>,
    pub estimated_bytes: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct BlockedAsset {
    pub asset_id: String,
    pub name: String,
    pub reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PlannedOutput {
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub room_name: String,
    pub title: String,
    pub aspect: String,
    pub duration: f64,
    pub bytes: u64,
    pub inputs: Vec<Asset>,
    pub reason: String,
    #[serde(default)]
    pub cut_start: Option<f64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub plan_id: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub progress: f64,
    pub message: String,
    pub completed_outputs: Vec<Artifact>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub output_id: String,
    pub path: String,
    pub bytes: u64,
    pub duration: f64,
    pub validation: String,
    pub source_ids: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ScanStatus {
    pub running: bool,
    pub completed: usize,
    pub total: usize,
    pub message: String,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn digest(value: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_ref()))[..20].to_string()
}

/// Compute a fast content hash: SHA-256 of (first 1 MiB of file + LE u64 file size),
/// returning the first 16 hex chars. Suitable for change detection and relink; not
/// a collision-resistant identity for duplicate detection.
pub fn fast_file_hash(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    let file_size = meta.len();
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    let mut h = Sha256::new();
    h.update(&buf);
    h.update(file_size.to_le_bytes());
    Ok(format!("{:x}", h.finalize())[..16].to_string())
}
