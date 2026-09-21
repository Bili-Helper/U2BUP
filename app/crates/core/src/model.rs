use serde::{Deserialize, Serialize};
use serde_json::Value;

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
