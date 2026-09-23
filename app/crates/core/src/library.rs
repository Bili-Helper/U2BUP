//! Multi-library management: CRUD API for `LibraryRoot` entries and
//! the `folder` / `webdav` / `openlist` scanner.
//! The legacy `liverec` scanner lives in `scanner.rs` and is called from `web.rs`.

use crate::{db::Db, model::*, web::ApiError, AppState};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path, sync::Arc};
use walkdir::WalkDir;

macro_rules! bail_api {
    ($($arg:tt)*) => { return Err(anyhow::anyhow!($($arg)*).into()) };
}

type HttpResult<T> = std::result::Result<Json<T>, ApiError>;

/// Video file extensions we recognize when scanning a generic folder.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "flv", "ts", "m2ts", "wmv", "webm", "m4v", "3gp",
];

// ── Router ────────────────────────────────────────────────────────────────────

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/libraries", get(list_libraries).post(create_library))
        .route(
            "/api/libraries/{id}",
            get(get_library).put(update_library).delete(delete_library),
        )
        .route("/api/libraries/{id}/scan", post(trigger_scan))
        .route("/api/libraries/{id}/status", get(library_status))
        .route("/api/assets", get(list_assets))
        .route("/api/assets/{id}", get(get_asset).put(update_asset_meta))
}

// ── Library CRUD ──────────────────────────────────────────────────────────────

async fn list_libraries(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let libs: Vec<LibraryRoot> = s.db.library_list()?;
    Ok(Json(json!({ "libraries": libs })))
}

async fn get_library(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> HttpResult<Value> {
    let lib: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
    let count = s.db.asset_count_for_library(&id)?;
    Ok(Json(json!({ "library": lib, "assetCount": count })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateLibraryBody {
    name: String,
    kind: LibraryKind,
    path: String,
    #[serde(default = "default_tz_body")]
    display_tz: String,
    #[serde(default)]
    readonly: bool,
    #[serde(default)]
    scan_exclude: Vec<String>,
}
fn default_tz_body() -> String {
    "Asia/Shanghai".into()
}

async fn create_library(
    State(s): State<Arc<AppState>>,
    Json(body): Json<CreateLibraryBody>,
) -> HttpResult<Value> {
    validate_library_body(&body.name, &body.path, &body.scan_exclude)?;
    let canonical = std::fs::canonicalize(&body.path)
        .with_context(|| format!("路径不存在或无法访问：{}", body.path))?;
    let path_str = canonical.to_string_lossy().to_string();

    // Prevent overlapping paths within the same kind (warn only, don't block).
    let existing: Vec<LibraryRoot> = s.db.library_list()?;
    let overlap = existing.iter().any(|lib| {
        let lib_path = path::Path::new(&lib.path);
        let new_path = canonical.as_path();
        new_path.starts_with(lib_path) || lib_path.starts_with(new_path)
    });

    let id = uuid::Uuid::new_v4().to_string();
    let readonly = body.readonly || body.kind.is_readonly();
    let lib = LibraryRoot {
        id: id.clone(),
        name: body.name,
        kind: body.kind,
        path: path_str,
        display_tz: body.display_tz,
        readonly,
        enabled: true,
        scan_exclude: body.scan_exclude,
        created_at: now(),
        last_scanned_at: None,
        scan_status: "idle".into(),
        scan_error: None,
        asset_count: 0,
    };
    s.db.library_put(&id, &lib)?;
    Ok(Json(json!({
        "library": lib,
        "warning": if overlap { Some("与已有素材库路径存在重叠，可能导致素材重复") } else { None }
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateLibraryBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_tz: Option<String>,
    #[serde(default)]
    readonly: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    scan_exclude: Option<Vec<String>>,
}

async fn update_library(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateLibraryBody>,
) -> HttpResult<Value> {
    let mut lib: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
    if let Some(name) = body.name {
        if name.trim().is_empty() || name.chars().count() > 80 {
            bail_api!("素材库名称应为 1–80 字");
        }
        lib.name = name;
    }
    if let Some(tz) = body.display_tz {
        lib.display_tz = tz;
    }
    if let Some(ro) = body.readonly {
        // Can only make more restrictive via API; kind-enforced readonly stays.
        lib.readonly = ro || lib.kind.is_readonly();
    }
    if let Some(enabled) = body.enabled {
        lib.enabled = enabled;
    }
    if let Some(exclude) = body.scan_exclude {
        if exclude.len() > 50 {
            bail_api!("排除规则过多");
        }
        lib.scan_exclude = exclude;
    }
    s.db.library_put(&id, &lib)?;
    Ok(Json(json!({ "library": lib })))
}

async fn delete_library(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> HttpResult<Value> {
    let lib: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
    let delete_assets = params
        .get("deleteAssets")
        .map(|v| v == "true")
        .unwrap_or(false);
    let asset_count = s.db.asset_count_for_library(&id)?;
    if delete_assets {
        s.db.asset_delete_for_library(&id)?;
    }
    s.db.library_delete(&id)?;
    Ok(Json(json!({
        "deleted": id,
        "name": lib.name,
        "assetsDeleted": if delete_assets { asset_count } else { 0 },
        "assetsOrphaned": if !delete_assets { asset_count } else { 0 },
    })))
}

async fn library_status(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HttpResult<Value> {
    let lib: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
    let count = s.db.asset_count_for_library(&id)?;
    Ok(Json(json!({
        "id": id,
        "scanStatus": lib.scan_status,
        "scanError": lib.scan_error,
        "lastScannedAt": lib.last_scanned_at,
        "assetCount": count,
    })))
}

// ── Scan ──────────────────────────────────────────────────────────────────────

async fn trigger_scan(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> HttpResult<Value> {
    let lib: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
    if !lib.enabled {
        bail_api!("素材库已禁用，请先启用后再扫描");
    }
    if lib.scan_status == "scanning" {
        bail_api!("该素材库正在扫描中");
    }
    // Mark as scanning.
    {
        let mut lib2 = lib.clone();
        lib2.scan_status = "scanning".into();
        lib2.scan_error = None;
        s.db.library_put(&id, &lib2)?;
    }

    // Spawn background scan task.
    let state = s.clone();
    let lib_id = id.clone();
    tokio::spawn(async move {
        let result = match lib.kind {
            LibraryKind::Liverec => {
                // Legacy liverec scan is handled by the existing web.rs /api/scan route.
                // Here we just report that it should be triggered via that route.
                Err(anyhow::anyhow!(
                    "liverec 库请使用 /api/scan 路由触发扫描（兼容模式）"
                ))
            }
            LibraryKind::Folder | LibraryKind::Webdav | LibraryKind::Openlist => {
                scan_folder_library(&state.db, &lib).await
            }
        };
        let mut lib_updated: LibraryRoot = match state.db.library_get(&lib_id) {
            Ok(Some(l)) => l,
            _ => return,
        };
        lib_updated.last_scanned_at = Some(now());
        match result {
            Ok(count) => {
                lib_updated.scan_status = "idle".into();
                lib_updated.scan_error = None;
                lib_updated.asset_count = count;
            }
            Err(e) => {
                lib_updated.scan_status = "error".into();
                lib_updated.scan_error = Some(format!("{e:#}"));
            }
        }
        let _ = state.db.library_put(&lib_id, &lib_updated);
    });

    Ok(Json(json!({ "id": id, "scanStatus": "scanning" })))
}

/// Scan a folder-type library, writing discovered video files into `assets_v2`.
/// Returns the total number of assets found (new + existing).
async fn scan_folder_library(db: &Db, lib: &LibraryRoot) -> Result<usize> {
    let root = path::Path::new(&lib.path);
    if !root.exists() {
        // Mark library as offline rather than erroring out.
        bail!("库路径不可访问，已标记为离线：{}", lib.path);
    }

    let exclude_patterns: Vec<glob::Pattern> = lib
        .scan_exclude
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();

    let mut found = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // Check extension.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !VIDEO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        // Apply exclude patterns against path relative to root.
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel.to_string_lossy();
        if exclude_patterns
            .iter()
            .any(|p| p.matches(&rel_str) || p.matches(path.to_string_lossy().as_ref()))
        {
            continue;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let file_size = meta.len();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_millis() as u64)
            })
            .unwrap_or(0);

        let source_path = path.to_string_lossy().to_string();

        // Stable ID: hash of library_id + canonical source_path.
        let id = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(lib.id.as_bytes());
            h.update(b":");
            h.update(source_path.as_bytes());
            format!("{:x}", h.finalize())[..20].to_string()
        };

        // Skip if file hasn't changed (same size + mtime).
        if let Ok(Some(existing)) = db.asset_get(&id) {
            if existing.modified_ms == modified_ms && existing.file_size == file_size {
                found += 1;
                continue;
            }
        }

        // Compute fast content hash.
        let content_hash = fast_file_hash(path).ok();

        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let asset = AssetV2 {
            id: id.clone(),
            library_id: lib.id.clone(),
            source_path: source_path.clone(),
            content_hash: content_hash.clone(),
            is_linked: true, // folder scan always references, never copies
            file_size,
            modified_ms,
            extension: ext,
            title: title.clone(),
            display_title: None,
            room_id: None,
            room_name: None,
            started_at: None,
            time_source: Some("mtime".into()),
            sidecars: vec![],
            warnings: vec![],
            // Tech info left empty; filled by async ffprobe (future).
            duration_sec: None,
            width: None,
            height: None,
            video_codec: None,
            resolution: None,
            // Publish metadata starts empty.
            pub_title: None,
            pub_description: None,
            pub_tags: vec![],
            pub_category_id: "22".into(),
            pub_privacy: "private".into(),
            pub_language: None,
            pub_audio_lang: None,
            custom_tags: vec![],
            custom_meta: None,
            upload_targets: vec![],
            file_status: "ok".into(),
        };

        let record = AssetRecord {
            id,
            library_id: lib.id.clone(),
            source_path,
            content_hash,
            pub_title: None,
            custom_tags_json: None,
            modified_ms,
            file_size,
            asset,
        };
        db.asset_put(&record)?;
        found += 1;
    }
    Ok(found)
}

// ── Asset query ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetQuery {
    library_id: Option<String>,
    #[serde(default = "default_page_size")]
    per_page: usize,
    #[serde(default)]
    page: usize,
}
fn default_page_size() -> usize {
    50
}

async fn list_assets(
    State(s): State<Arc<AppState>>,
    Query(q): Query<AssetQuery>,
) -> HttpResult<Value> {
    let per_page = q.per_page.clamp(1, 200);
    let offset = q.page * per_page;

    if let Some(lib_id) = &q.library_id {
        // Single-library query.
        let total = s.db.asset_count_for_library(lib_id)?;
        let records = s.db.asset_list_for_library(lib_id, per_page, offset)?;
        let assets: Vec<&AssetV2> = records.iter().map(|r| &r.asset).collect();
        Ok(Json(json!({
            "assets": assets,
            "total": total,
            "page": q.page,
            "perPage": per_page,
        })))
    } else {
        // Cross-library query: list all libraries and paginate across them.
        // Simple approach: load per-library counts to compute offset, then fetch.
        // Good enough for Phase 1; Phase 2 will add a proper cross-library index query.
        let libs: Vec<LibraryRoot> = s.db.library_list()?;
        let total: usize = libs
            .iter()
            .map(|lib| s.db.asset_count_for_library(&lib.id).unwrap_or(0))
            .sum();
        let mut remaining_offset = offset;
        let mut results: Vec<AssetV2> = Vec::new();
        'outer: for lib in &libs {
            let lib_count = s.db.asset_count_for_library(&lib.id)?;
            if remaining_offset >= lib_count {
                remaining_offset -= lib_count;
                continue;
            }
            let records =
                s.db.asset_list_for_library(&lib.id, per_page - results.len(), remaining_offset)?;
            remaining_offset = 0;
            for r in records {
                results.push(r.asset);
                if results.len() >= per_page {
                    break 'outer;
                }
            }
        }
        Ok(Json(json!({
            "assets": results,
            "total": total,
            "page": q.page,
            "perPage": per_page,
        })))
    }
}

async fn get_asset(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> HttpResult<Value> {
    let record = s.db.asset_get(&id)?.context("素材不存在")?;
    Ok(Json(serde_json::to_value(&record.asset)?))
}

// ── Asset metadata update ─────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateMetaBody {
    #[serde(default)]
    pub_title: Option<String>,
    #[serde(default)]
    pub_description: Option<String>,
    #[serde(default)]
    pub_tags: Option<Vec<String>>,
    #[serde(default)]
    pub_category_id: Option<String>,
    #[serde(default)]
    pub_privacy: Option<String>,
    #[serde(default)]
    pub_language: Option<String>,
    #[serde(default)]
    pub_audio_lang: Option<String>,
    #[serde(default)]
    custom_tags: Option<Vec<String>>,
    #[serde(default)]
    display_title: Option<String>,
}

async fn update_asset_meta(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMetaBody>,
) -> HttpResult<Value> {
    let mut record = s.db.asset_get(&id)?.context("素材不存在")?;
    let a = &mut record.asset;

    if let Some(v) = body.pub_title {
        validate_pub_title(&v)?;
        a.pub_title = if v.is_empty() { None } else { Some(v.clone()) };
        record.pub_title = a.pub_title.clone();
    }
    if let Some(v) = body.pub_description {
        if v.len() > 5000 {
            bail_api!("描述不能超过 5000 字节");
        }
        a.pub_description = if v.is_empty() { None } else { Some(v) };
    }
    if let Some(v) = body.pub_tags {
        validate_tags(&v)?;
        a.pub_tags = v;
    }
    if let Some(v) = body.pub_category_id {
        if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) {
            bail_api!("无效 YouTube 分类 ID");
        }
        a.pub_category_id = v;
    }
    if let Some(v) = body.pub_privacy {
        if !["private", "unlisted", "public"].contains(&v.as_str()) {
            bail_api!("无效公开状态");
        }
        a.pub_privacy = v;
    }
    if let Some(v) = body.pub_language {
        a.pub_language = Some(v);
    }
    if let Some(v) = body.pub_audio_lang {
        a.pub_audio_lang = Some(v);
    }
    if let Some(v) = body.custom_tags {
        if v.len() > 100 {
            bail_api!("自定义标签过多");
        }
        record.custom_tags_json = Some(serde_json::to_string(&v)?);
        a.custom_tags = v;
    }
    if let Some(v) = body.display_title {
        a.display_title = if v.is_empty() { None } else { Some(v) };
    }

    s.db.asset_put(&record)?;
    Ok(Json(serde_json::to_value(&record.asset)?))
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn validate_library_body(
    name: &str,
    path: &str,
    exclude: &[String],
) -> std::result::Result<(), ApiError> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        bail_api!("素材库名称应为 1–80 字");
    }
    if path.trim().is_empty() {
        bail_api!("路径不能为空");
    }
    if exclude.len() > 50 {
        bail_api!("排除规则最多 50 条");
    }
    Ok(())
}

fn validate_pub_title(v: &str) -> std::result::Result<(), ApiError> {
    if !v.is_empty() && (v.chars().count() > 100 || v.contains(['<', '>'])) {
        bail_api!("发布标题应为 1–100 字且不含尖括号");
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> std::result::Result<(), ApiError> {
    if tags
        .iter()
        .any(|t| t.trim().is_empty() || t.contains(['<', '>']))
    {
        bail_api!("标签为空或含尖括号");
    }
    let total: usize = tags
        .iter()
        .map(|t| t.chars().count() + if t.contains(' ') { 2 } else { 0 })
        .sum::<usize>()
        + tags.len().saturating_sub(1);
    if total > 500 {
        bail_api!("标签总长度超过 500");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use axum::{extract::State, Json};

    use std::{fs, sync::Arc};

    // ── helpers ──────────────────────────────────────────────────────────

    fn open_db(dir: &std::path::Path) -> Db {
        Db::open(&dir.join("test.sqlite")).unwrap()
    }

    fn make_state(dir: &std::path::Path) -> Arc<crate::AppState> {
        Arc::new(crate::youtube::protocol_tests::state(dir, String::new()))
    }

    fn lib_body(name: &str, kind: LibraryKind, path: &str) -> CreateLibraryBody {
        CreateLibraryBody {
            name: name.into(),
            kind,
            path: path.into(),
            display_tz: "Asia/Shanghai".into(),
            readonly: false,
            scan_exclude: vec![],
        }
    }

    // ── LibraryRoot CRUD ─────────────────────────────────────────────────

    #[tokio::test]
    async fn create_and_list_library() {
        let tmp = tempfile::tempdir().unwrap();
        let lib_dir = tmp.path().join("media");
        fs::create_dir_all(&lib_dir).unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "我的素材库",
                LibraryKind::Folder,
                lib_dir.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();

        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();
        assert_eq!(resp.0["library"]["name"], "我的素材库");
        assert_eq!(resp.0["library"]["kind"], "folder");
        assert!(resp.0["warning"].is_null());

        let list = list_libraries(State(s.clone())).await.unwrap();
        assert_eq!(list.0["libraries"].as_array().unwrap().len(), 1);

        let detail = get_library(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        assert_eq!(detail.0["library"]["id"], lib_id);
        assert_eq!(detail.0["assetCount"], 0);
    }

    #[tokio::test]
    async fn create_library_rejects_nonexistent_path() {
        let tmp = tempfile::tempdir().unwrap();
        let s = make_state(tmp.path());
        let result = create_library(
            State(s),
            Json(lib_body(
                "bad",
                LibraryKind::Folder,
                "/nonexistent/path/xyz",
            )),
        )
        .await;
        assert!(result.is_err(), "should reject nonexistent path");
    }

    #[tokio::test]
    async fn create_library_rejects_empty_name() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let s = make_state(tmp.path());
        let result = create_library(
            State(s),
            Json(lib_body("", LibraryKind::Folder, media.to_str().unwrap())),
        )
        .await;
        assert!(result.is_err(), "empty name should be rejected");
    }

    #[tokio::test]
    async fn update_library_name_and_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "原始名称",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let id = resp.0["library"]["id"].as_str().unwrap().to_string();

        let updated = update_library(
            State(s.clone()),
            axum::extract::Path(id.clone()),
            Json(UpdateLibraryBody {
                name: Some("新名称".into()),
                enabled: Some(false),
                display_tz: None,
                readonly: None,
                scan_exclude: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(updated.0["library"]["name"], "新名称");
        assert_eq!(updated.0["library"]["enabled"], false);
    }

    #[tokio::test]
    async fn delete_library_without_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "删除测试",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let id = resp.0["library"]["id"].as_str().unwrap().to_string();

        let mut params = std::collections::HashMap::new();
        params.insert("deleteAssets".to_string(), "true".to_string());
        let del = delete_library(
            State(s.clone()),
            axum::extract::Path(id.clone()),
            axum::extract::Query(params),
        )
        .await
        .unwrap();
        assert_eq!(del.0["deleted"], id);

        let list = list_libraries(State(s)).await.unwrap();
        assert_eq!(list.0["libraries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn delete_nonexistent_library_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let s = make_state(tmp.path());
        let result = delete_library(
            State(s),
            axum::extract::Path("nonexistent-id".to_string()),
            axum::extract::Query(std::collections::HashMap::new()),
        )
        .await;
        assert!(result.is_err());
    }

    // ── WebDAV / OpenList kind forces readonly ─────────────────────────

    #[tokio::test]
    async fn webdav_library_is_forced_readonly() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = tmp.path().join("mount");
        fs::create_dir_all(&mount).unwrap();
        let s = make_state(tmp.path());

        let mut body = lib_body("NAS", LibraryKind::Webdav, mount.to_str().unwrap());
        body.readonly = false; // explicitly false, kind should override
        let resp = create_library(State(s), Json(body)).await.unwrap();
        assert_eq!(
            resp.0["library"]["readonly"], true,
            "webdav should always be readonly"
        );
    }

    // ── Folder scan ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn folder_scan_discovers_video_files() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();

        // Create fake video files.
        fs::write(media.join("clip1.mp4"), b"fake mp4").unwrap();
        fs::write(media.join("clip2.mkv"), b"fake mkv").unwrap();
        // Non-video should be ignored.
        fs::write(media.join("notes.txt"), b"text").unwrap();
        // Nested.
        let sub = media.join("subdir");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("part1.ts"), b"fake ts").unwrap();

        let s = make_state(tmp.path());

        // Create the library.
        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "scan test",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        // Trigger scan (this runs inline via tokio::spawn inside trigger_scan,
        // so we give the task a moment to complete).
        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Check asset count.
        let count = s.db.asset_count_for_library(&lib_id).unwrap();
        assert_eq!(count, 3, "should find clip1.mp4, clip2.mkv, part1.ts");
    }

    #[tokio::test]
    async fn folder_scan_respects_exclude_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();

        fs::write(media.join("keep.mp4"), b"keep").unwrap();
        fs::write(media.join("skip.mp4"), b"skip").unwrap();
        let raw = media.join("RAW");
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("raw.mkv"), b"raw").unwrap();

        let s = make_state(tmp.path());

        let mut body = lib_body("exclude test", LibraryKind::Folder, media.to_str().unwrap());
        body.scan_exclude = vec!["**/skip.mp4".into(), "**/RAW/**".into()];

        let resp = create_library(State(s.clone()), Json(body)).await.unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let count = s.db.asset_count_for_library(&lib_id).unwrap();
        assert_eq!(count, 1, "only keep.mp4 should be indexed");
    }

    #[tokio::test]
    async fn folder_scan_is_incremental() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("a.mp4"), b"video a").unwrap();

        let s = make_state(tmp.path());
        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "inc test",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        assert_eq!(s.db.asset_count_for_library(&lib_id).unwrap(), 1);

        // Add a second file and re-scan.
        fs::write(media.join("b.mkv"), b"video b").unwrap();
        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        assert_eq!(s.db.asset_count_for_library(&lib_id).unwrap(), 2);
    }

    #[tokio::test]
    async fn disabled_library_cannot_be_scanned() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "disabled",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        update_library(
            State(s.clone()),
            axum::extract::Path(lib_id.clone()),
            Json(UpdateLibraryBody {
                enabled: Some(false),
                name: None,
                display_tz: None,
                readonly: None,
                scan_exclude: None,
            }),
        )
        .await
        .unwrap();

        let result = trigger_scan(State(s), axum::extract::Path(lib_id)).await;
        assert!(result.is_err(), "disabled library should not be scannable");
    }

    // ── Asset query API ───────────────────────────────────────────────────

    #[tokio::test]
    async fn asset_list_returns_paged_results() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        for i in 0..5 {
            fs::write(media.join(format!("v{i}.mp4")), b"x").unwrap();
        }
        let s = make_state(tmp.path());
        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "paged",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Page 0, 3 per page.
        let q = AssetQuery {
            library_id: Some(lib_id.clone()),
            per_page: 3,
            page: 0,
        };
        let result = list_assets(State(s.clone()), axum::extract::Query(q))
            .await
            .unwrap();
        assert_eq!(result.0["total"], 5);
        assert_eq!(result.0["assets"].as_array().unwrap().len(), 3);

        // Page 1.
        let q2 = AssetQuery {
            library_id: Some(lib_id),
            per_page: 3,
            page: 1,
        };
        let p2 = list_assets(State(s), axum::extract::Query(q2))
            .await
            .unwrap();
        assert_eq!(p2.0["assets"].as_array().unwrap().len(), 2);
    }

    // ── Asset metadata update ─────────────────────────────────────────────

    #[tokio::test]
    async fn update_asset_pub_meta_validates_and_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("v.mp4"), b"x").unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "meta test",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();

        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let assets = list_assets(
            State(s.clone()),
            axum::extract::Query(AssetQuery {
                library_id: Some(lib_id),
                per_page: 10,
                page: 0,
            }),
        )
        .await
        .unwrap();
        let asset_id = assets.0["assets"][0]["id"].as_str().unwrap().to_string();

        // Valid update.
        let updated = update_asset_meta(
            State(s.clone()),
            axum::extract::Path(asset_id.clone()),
            Json(UpdateMetaBody {
                pub_title: Some("我的发布标题".into()),
                pub_tags: Some(vec!["直播".into(), "ASMR".into()]),
                pub_privacy: Some("unlisted".into()),
                pub_description: None,
                pub_category_id: None,
                pub_language: None,
                pub_audio_lang: None,
                custom_tags: Some(vec!["测试".into()]),
                display_title: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.0["pubTitle"], "我的发布标题");
        assert_eq!(updated.0["pubPrivacy"], "unlisted");
        assert_eq!(updated.0["pubTags"].as_array().unwrap().len(), 2);
        assert_eq!(updated.0["customTags"][0], "测试");

        // Reload from DB and confirm persisted.
        let reloaded = get_asset(State(s.clone()), axum::extract::Path(asset_id.clone()))
            .await
            .unwrap();
        assert_eq!(reloaded.0["pubTitle"], "我的发布标题");
    }

    #[tokio::test]
    async fn update_asset_rejects_invalid_privacy() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("v.mp4"), b"x").unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "invalid",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();
        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let assets = list_assets(
            State(s.clone()),
            axum::extract::Query(AssetQuery {
                library_id: Some(lib_id),
                per_page: 10,
                page: 0,
            }),
        )
        .await
        .unwrap();
        let asset_id = assets.0["assets"][0]["id"].as_str().unwrap().to_string();

        let result = update_asset_meta(
            State(s),
            axum::extract::Path(asset_id),
            Json(UpdateMetaBody {
                pub_privacy: Some("everyone".into()), // invalid
                pub_title: None,
                pub_description: None,
                pub_tags: None,
                pub_category_id: None,
                pub_language: None,
                pub_audio_lang: None,
                custom_tags: None,
                display_title: None,
            }),
        )
        .await;
        assert!(result.is_err(), "invalid privacy value should be rejected");
    }

    #[tokio::test]
    async fn update_asset_rejects_title_over_100_chars() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("v.mp4"), b"x").unwrap();
        let s = make_state(tmp.path());

        let resp = create_library(
            State(s.clone()),
            Json(lib_body(
                "long title",
                LibraryKind::Folder,
                media.to_str().unwrap(),
            )),
        )
        .await
        .unwrap();
        let lib_id = resp.0["library"]["id"].as_str().unwrap().to_string();
        trigger_scan(State(s.clone()), axum::extract::Path(lib_id.clone()))
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let assets = list_assets(
            State(s.clone()),
            axum::extract::Query(AssetQuery {
                library_id: Some(lib_id),
                per_page: 10,
                page: 0,
            }),
        )
        .await
        .unwrap();
        let asset_id = assets.0["assets"][0]["id"].as_str().unwrap().to_string();

        let long_title = "超".repeat(101);
        let result = update_asset_meta(
            State(s),
            axum::extract::Path(asset_id),
            Json(UpdateMetaBody {
                pub_title: Some(long_title),
                pub_description: None,
                pub_tags: None,
                pub_privacy: None,
                pub_category_id: None,
                pub_language: None,
                pub_audio_lang: None,
                custom_tags: None,
                display_title: None,
            }),
        )
        .await;
        assert!(result.is_err(), "title over 100 chars should be rejected");
    }

    // ── DB schema migration ───────────────────────────────────────────────

    #[test]
    fn db_schema_migrates_to_v2_on_fresh_open() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open(&tmp.path().join("test.sqlite")).unwrap();
        assert!(
            db._schema_version_is_current(),
            "fresh DB should be at current schema version"
        );
    }

    #[test]
    fn db_schema_migration_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.sqlite");
        // Open twice; second open should not error.
        let _db1 = Db::open(&path).unwrap();
        let db2 = Db::open(&path).unwrap();
        assert!(db2._schema_version_is_current());
    }

    #[test]
    fn db_v1_documents_preserved_after_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open(&tmp.path().join("test.sqlite")).unwrap();
        // Write a v1-style document.
        db.put("test-kind", "test-id", &serde_json::json!({"value": 42}))
            .unwrap();
        // Re-open (simulates upgrade).
        let db2 = Db::open(&tmp.path().join("test.sqlite")).unwrap();
        let v: Option<serde_json::Value> = db2.get("test-kind", "test-id").unwrap();
        assert_eq!(
            v.unwrap()["value"],
            42,
            "v1 documents must survive migration"
        );
    }

    // ── fast_file_hash ────────────────────────────────────────────────────

    #[test]
    fn fast_file_hash_is_stable_and_size_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.mp4");
        std::fs::write(&path, b"hello world").unwrap();

        let h1 = fast_file_hash(&path).unwrap();
        let h2 = fast_file_hash(&path).unwrap();
        assert_eq!(h1, h2, "same file → same hash");
        assert_eq!(h1.len(), 16, "hash should be 16 hex chars");

        // Changing content changes hash.
        std::fs::write(&path, b"hello world!").unwrap();
        let h3 = fast_file_hash(&path).unwrap();
        assert_ne!(h1, h3, "different content → different hash");
    }
}
