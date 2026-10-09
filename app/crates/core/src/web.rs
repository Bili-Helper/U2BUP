use crate::{catalog, jobs, media, model::*, planner, scanner, AppState};
use anyhow::{anyhow, bail, Context};
use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use tower_http::services::ServeFile;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
struct Ui;
#[derive(Debug)]
pub(crate) struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}
type Result<T> = std::result::Result<T, ApiError>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/session", post(session))
        .merge(crate::youtube::router())
        .merge(crate::workflow::router())
        .merge(crate::library::router())
        .route("/api/snapshot", get(snapshot))
        .route("/api/status", get(status))
        .route("/api/tasks", get(tasks))
        .route("/api/tasks/{id}/{action}", post(task_action))
        .route("/api/scan", post(scan))
        .route("/api/plans", post(plan))
        .route("/api/plans/{id}/execute", post(execute))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/rooms/{id}/refresh", post(refresh_room))
        .route("/api/titles/preview", post(preview_titles))
        .route("/api/titles/apply", post(apply_titles))
        .route("/api/assets/{id}/thumbnail", get(thumbnail))
        .route("/api/assets/{id}/media", get(video))
        .fallback(ui)
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
async fn guard(State(s): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let host = req.headers().get("host").and_then(|v| v.to_str().ok());
    let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
    if host != Some(s.authority.as_str()) || origin.is_some_and(|v| v != s.origin) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"访问来源不匹配，请使用应用提供的本机链接"})),
        )
            .into_response();
    }
    if s.config.headless
        && (req.uri().path() == "/oauth/youtube/callback"
            || (req.uri().path().starts_with("/api/youtube")
                && req.method() != axum::http::Method::GET))
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":crate::youtube::HEADLESS_REASON})),
        )
            .into_response();
    }
    if req.uri().path().starts_with("/api/") {
        // Cookies are host scoped, not port scoped: multiple local instances must not collide.
        let cookie_prefix = format!("u2bup_session_{}=", s.authority.replace(['.', ':'], "_"));
        let bearer = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let cookie = req
            .headers()
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.split(';')
                    .find_map(|c| c.trim().strip_prefix(&cookie_prefix))
            });
        if bearer != Some(&s.token) && cookie != Some(&s.token) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"请通过启动链接建立本机会话"})),
            )
                .into_response();
        }
    }
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
async fn session(State(s): State<Arc<AppState>>) -> Response {
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_str(&format!(
            "u2bup_session_{}={}; HttpOnly; SameSite=Strict; Path=/{}",
            s.authority.replace(['.', ':'], "_"),
            s.token,
            if s.origin.starts_with("https://") {
                "; Secure"
            } else {
                ""
            }
        ))
        .unwrap(),
    );
    response
}
async fn ui(req: Request) -> Response {
    if req.uri().path().starts_with("/api/") {
        return (StatusCode::NOT_FOUND, Json(json!({"error":"接口不存在"}))).into_response();
    }
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Ui::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            let mut r = (
                [(axum::http::header::CONTENT_TYPE, mime)],
                file.data.into_owned(),
            )
                .into_response();
            r.headers_mut().insert("content-security-policy",HeaderValue::from_static("default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'"));
            r
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SnapshotQuery {
    library_id: Option<String>,
}
async fn snapshot(
    State(s): State<Arc<AppState>>,
    Query(q): Query<SnapshotQuery>,
) -> Result<Json<Value>> {
    let (library, scan) = if let Some(id) = q.library_id {
        let root: LibraryRoot = s.db.library_get(&id)?.context("素材库不存在")?;
        let library = if root.last_scanned_at.is_none()
            && s.db.asset_count_for_library(&id)? == 0
            && std::path::Path::new(&root.path).canonicalize().ok()
                == s.config.library.canonicalize().ok()
            && std::path::Path::new(&root.path).exists()
        {
            s.library.read().await.clone()
        } else {
            catalog::project_library(&s.db, &root)?
        };
        let count = library.assets.len();
        let scan = ScanStatus {
            running: root.scan_status == "scanning",
            completed: count,
            total: count,
            message: root.scan_error.unwrap_or_else(|| {
                if root.scan_status == "scanning" {
                    "正在扫描素材库".into()
                } else {
                    format!("已索引 {count} 个视频")
                }
            }),
        };
        (library, scan)
    } else {
        (
            s.library.read().await.clone(),
            s.scan_status.lock().await.clone(),
        )
    };
    let library_path = library.root.clone();
    Ok(Json(
        json!({"library":library,"scan":scan,"jobs":s.db.list::<Job>("job")?,"plans":s.db.list::<Plan>("plan")?,"settings":{"library":if library_path.is_empty() { s.config.library.to_string_lossy().into_owned() } else { library_path },"output":s.config.output,"data":s.config.data,"ffmpeg":s.config.ffmpeg,"ffprobe":s.config.ffprobe,"version":env!("CARGO_PKG_VERSION"),"mode":if s.config.headless {"Headless · 单用户"} else {"本机 · 单用户"}}}),
    ))
}
async fn status(State(s): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"scan":s.scan_status.lock().await.clone(),"jobs":s.db.list::<Job>("job")?}),
    ))
}

// A local-only projection of the existing executors. Upload sessions and credentials
// stay private in youtube.rs; failure to read a keychain cannot hide media tasks.
async fn task_list(s: &AppState) -> anyhow::Result<Vec<Value>> {
    let media_jobs = s.db.list::<Job>("job")?;
    let uploads = crate::youtube::upload_summaries(&s.db)?;
    let plans = s.db.list::<Plan>("plan")?;
    let tokens = s.cancellations.lock().await;
    let mut tasks = Vec::with_capacity(media_jobs.len() + uploads.len());
    for j in media_jobs {
        let plan = plans.iter().find(|p| p.id == j.plan_id);
        let title = plan
            .map(|p| {
                let cuts = p.outputs.iter().filter(|o| o.cut_start.is_some()).count();
                let operation = if cuts == 0 {
                    "场次合并"
                } else if cuts == p.outputs.len() {
                    "自动切割"
                } else {
                    "合并与切割"
                };
                format!("{operation} · {} 个成品", p.outputs.len())
            })
            .unwrap_or_else(|| "媒体处理".into());
        let active = tokens.get(&j.id).is_some_and(|t| !t.is_cancelled());
        tasks.push(json!({
            "id": format!("media:{}", j.id), "source_id": j.id, "kind": "media",
            "title": title, "status": j.status, "created_at": j.created_at,
            "updated_at": j.updated_at, "progress": j.progress.clamp(0.0, 1.0),
            "message": j.message, "plan_id": j.plan_id,
            "completed_outputs": j.completed_outputs,
            "capabilities": {
                "cancel": active && ["pending", "running"].contains(&j.status.as_str()),
                "pause": false, "resume": false,
                "retry": !tokens.contains_key(&j.id) && plan.is_some()
                    && ["failed", "cancelled", "interrupted"].contains(&j.status.as_str())
            }
        }));
    }
    for j in uploads {
        let id = j["id"].as_str().context("上传任务 ID 缺失")?;
        let key = crate::youtube::upload_token_key(id);
        let status = j["status"].as_str().unwrap_or("");
        let bytes = j["bytes"].as_u64().unwrap_or(0);
        let offset = j["offset"].as_u64().unwrap_or(0);
        let active = tokens.get(&key).is_some_and(|t| !t.is_cancelled());
        let available = !s.config.headless && !tokens.contains_key(&key);
        tasks.push(json!({
            "id": format!("upload:{id}"), "source_id": id, "kind": "upload",
            "title": j["title"], "status": status, "created_at": j["created_at"],
            "updated_at": j["updated_at"],
            "progress": if bytes == 0 { None } else { Some((offset as f64 / bytes as f64).clamp(0.0, 1.0)) },
            "message": j["message"], "artifact_id": j["artifact_id"],
            "video_id": j["video_id"], "bytes": bytes, "offset": offset,
            "channel_id": j["channel_id"], "privacy": j["privacy"],
            "capabilities": {
                "cancel": false,
                "pause": !s.config.headless && active && ["queued", "running"].contains(&status),
                "resume": available && ["paused", "interrupted"].contains(&status),
                "retry": available && status == "failed"
            }
        }));
    }
    tasks.sort_by(|a, b| {
        b["created_at"]
            .as_str()
            .cmp(&a["created_at"].as_str())
            .then_with(|| b["updated_at"].as_str().cmp(&a["updated_at"].as_str()))
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    Ok(tasks)
}

async fn tasks(State(s): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(json!({
        "tasks": task_list(&s).await?, "scan": s.scan_status.lock().await.clone()
    })))
}

async fn task_action(
    State(s): State<Arc<AppState>>,
    Path((id, action)): Path<(String, String)>,
) -> Result<Json<Value>> {
    if !["cancel", "pause", "resume", "retry"].contains(&action.as_str()) {
        return Err(anyhow!("不支持的任务操作").into());
    }
    let list = task_list(&s).await?;
    let task = list.iter().find(|t| t["id"] == id).context("任务不存在")?;
    if task["capabilities"][&action] != true {
        return Err(anyhow!("当前任务状态不支持此操作，请刷新任务列表").into());
    }
    let (kind, source_id) = id.split_once(':').context("无效任务 ID")?;
    match (kind, action.as_str()) {
        ("media", "cancel") => cancel(State(s), Path(source_id.into())).await,
        ("media", "retry") => {
            let plan_id = task["plan_id"].as_str().context("任务缺少处理计划")?;
            let job = execute(State(s), Path(plan_id.into())).await?.0;
            Ok(Json(
                json!({"ok":true,"task_id":format!("media:{}",job.id)}),
            ))
        }
        ("upload", "pause") => crate::youtube::pause(State(s), Path(source_id.into())).await,
        ("upload", "resume" | "retry") => {
            crate::youtube::resume(State(s), Path(source_id.into())).await
        }
        _ => Err(anyhow!("不支持的任务操作").into()),
    }
}
async fn scan(State(s): State<Arc<AppState>>) -> Result<Json<Value>> {
    let permit = s
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| anyhow!("已有扫描或处理任务正在运行"))?;
    *s.scan_status.lock().await = ScanStatus {
        running: true,
        message: "发现素材文件".into(),
        ..Default::default()
    };
    tokio::spawn(async move {
        let _permit = permit;
        let result = scanner::scan(s.clone()).await;
        let mut status = s.scan_status.lock().await;
        status.running = false;
        status.message = match result {
            Ok(()) => format!("已完成 {} 个视频的索引", status.completed),
            Err(e) => format!("扫描失败：{e:#}"),
        };
    });
    Ok(Json(json!({"accepted":true})))
}
async fn plan(State(s): State<Arc<AppState>>, Json(req): Json<PlanRequest>) -> Result<Json<Plan>> {
    let selected = catalog::selection(&s, &req.asset_ids).await?;
    let p = planner::build(&selected, req)?;
    s.db.put("plan", &p.id, &p)?;
    Ok(Json(p))
}
async fn execute(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Job>> {
    let permit = s
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| anyhow!("已有扫描或合并正在运行，请等待当前任务完成"))?;
    let plan = s.db.get::<Plan>("plan", &id)?.context("计划不存在")?;
    if plan.outputs.is_empty() {
        return Err(anyhow!("计划中没有可执行输出").into());
    }
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: id,
        status: "pending".into(),
        created_at: now(),
        updated_at: now(),
        progress: 0.0,
        message: "准备执行".into(),
        completed_outputs: vec![],
    };
    s.db.put("job", &job.id, &job)?;
    let cancel = tokio_util::sync::CancellationToken::new();
    s.cancellations
        .lock()
        .await
        .insert(job.id.clone(), cancel.clone());
    let cloned = job.clone();
    tokio::spawn(async move {
        let _permit = permit;
        jobs::execute(s, plan, cloned, cancel).await;
    });
    Ok(Json(job))
}
async fn cancel(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let tokens = s.cancellations.lock().await;
    tokens.get(&id).context("任务未在运行")?.cancel();
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod task_tests {
    use super::*;
    use crate::{db::Db, Config};
    use axum::http::Request as HttpRequest;
    use tokio::sync::{Mutex, RwLock, Semaphore};
    use tokio_util::sync::CancellationToken;

    fn state(root: &std::path::Path, headless: bool) -> Arc<AppState> {
        Arc::new(AppState {
            config: Config {
                library: root.into(),
                data: root.into(),
                output: root.into(),
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
                port: 0,
                bind: std::net::Ipv4Addr::LOCALHOST.into(),
                public_origin: None,
                headless,
            },
            db: Db::open(&root.join("tasks.sqlite")).unwrap(),
            library: RwLock::new(Library::default()),
            scan_status: Mutex::new(ScanStatus::default()),
            operation: Arc::new(Mutex::new(())),
            cancellations: Mutex::new(Default::default()),
            thumbnails: Semaphore::new(1),
            token: "test-session".into(),
            authority: "127.0.0.1:4173".into(),
            origin: "http://127.0.0.1:4173".into(),
            shutdown: CancellationToken::new(),
            _lock: std::fs::File::create(root.join("lock")).unwrap(),
            youtube: crate::youtube::Runtime::default(),
        })
    }

    fn seed(s: &AppState, media_status: &str, upload_status: &str) {
        s.db.put(
            "job",
            "same-id",
            &Job {
                id: "same-id".into(),
                plan_id: "plan".into(),
                status: media_status.into(),
                created_at: "2026-09-21T10:00:00Z".into(),
                updated_at: now(),
                progress: 0.5,
                message: "媒体处理".into(),
                completed_outputs: vec![],
            },
        )
        .unwrap();
        // A real pre-v0.4 upload record, including its private resumable session.
        s.db.put(
            "yt-upload",
            "same-id",
            &json!({
                "id":"same-id", "artifact_id":"artifact", "path":"private-path.mp4",
                "bytes":100, "modified_ms":0, "metadata":{"title":"上传标题","made_for_kids":false},
                "channel_id":"channel", "status":upload_status, "offset":25,
                "session":"https://www.googleapis.com/upload/private-session-secret",
                "video_id":null, "message":"等待处理"
            }),
        )
        .unwrap();
        s.db.put(
            "plan",
            "plan",
            &Plan {
                id: "plan".into(),
                created_at: now(),
                request: serde_json::from_value(json!({"asset_ids":[]})).unwrap(),
                outputs: vec![],
                blocked: vec![],
                estimated_bytes: 0,
            },
        )
        .unwrap();
    }

    async fn request(s: Arc<AppState>, method: &str, path: &str) -> Response {
        router(s)
            .oneshot(
                HttpRequest::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "127.0.0.1:4173")
                    .header("authorization", "Bearer test-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    fn seed_registered(s: &AppState, root: &std::path::Path) -> Asset {
        let path = root.join("主播/merged/recording.mp4");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"registered-video").unwrap();
        let root = LibraryRoot {
            id: "registered".into(),
            name: "外部录播".into(),
            kind: LibraryKind::Folder,
            path: root.canonicalize().unwrap().to_string_lossy().into_owned(),
            display_tz: "Asia/Shanghai".into(),
            readonly: false,
            enabled: true,
            scan_exclude: vec![],
            created_at: now(),
            last_scanned_at: Some(now()),
            scan_status: "idle".into(),
            scan_error: None,
            asset_count: 1,
        };
        s.db.library_put(&root.id, &root).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let asset = AssetV2 {
            id: "registered-asset".into(),
            library_id: root.id.clone(),
            source_path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
            title: "直播标题".into(),
            extension: "mp4".into(),
            room_id: Some("123456".into()),
            room_name: Some("主播".into()),
            started_at: Some("2026-09-25T09:53:02+08:00".into()),
            time_source: Some("filename".into()),
            file_status: "ok".into(),
            file_size: meta.len(),
            modified_ms: media::modified_ms(&meta),
            custom_meta: Some(json!({"scanner":{"version":1,"role":"legacy","metadata":{
                "duration":120.0,"width":1080,"height":1920,"codec":"h264","aspect":"9:16",
                "signature":"sig","streams":[{"codec_type":"video","codec_name":"h264"}]
            }}})),
            ..Default::default()
        };
        let record = AssetRecord {
            id: asset.id.clone(),
            library_id: root.id.clone(),
            source_path: asset.source_path.clone(),
            content_hash: None,
            pub_title: None,
            custom_tags_json: None,
            modified_ms: asset.modified_ms,
            file_size: asset.file_size,
            asset,
        };
        s.db.asset_put(&record).unwrap();
        catalog::project_asset(&root, &record).unwrap()
    }

    async fn request_json(s: Arc<AppState>, path: &str, value: Value) -> Value {
        let response = router(s)
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(path)
                    .header("host", "127.0.0.1:4173")
                    .header("authorization", "Bearer test-session")
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn registered_library_snapshot_preview_plan_and_titles_use_its_own_root() {
        let legacy = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let s = state(legacy.path(), false);
        let asset = seed_registered(&s, external.path());
        assert!(s.library.read().await.assets.is_empty());
        let response = request(s.clone(), "GET", "/api/snapshot?libraryId=registered").await;
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["library"]["assets"][0]["id"], asset.id);
        assert_eq!(value["library"]["rooms"][0]["id"], "123456");
        assert_eq!(value["scan"]["completed"], 1);
        assert_eq!(
            catalog::resolve_input(&s, &asset).await.unwrap(),
            external
                .path()
                .join("主播/merged/recording.mp4")
                .canonicalize()
                .unwrap()
        );

        let response = router(s.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/assets/registered-asset/media")
                    .header("host", "127.0.0.1:4173")
                    .header("authorization", "Bearer test-session")
                    .header("range", "bytes=1-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            b"egi"
        );

        let plan = request_json(
            s.clone(),
            "/api/plans",
            json!({"asset_ids":[asset.id],"include_legacy":true}),
        )
        .await;
        assert_eq!(plan["outputs"].as_array().unwrap().len(), 1);
        assert_eq!(
            plan["outputs"][0]["inputs"][0]["relative_path"],
            asset.relative_path
        );
        let changes = request_json(
            s.clone(),
            "/api/titles/preview",
            json!({"asset_ids":[asset.id],"template":"{主播} {日期} {标题}"}),
        )
        .await;
        assert_eq!(changes[0]["after"], "主播 2026-09-25 直播标题");
        request_json(s.clone(), "/api/titles/apply", changes).await;
        assert_eq!(
            catalog::asset(&s, &asset.id)
                .await
                .unwrap()
                .display_title
                .as_deref(),
            Some("主播 2026-09-25 直播标题")
        );
        assert!(s.library.read().await.assets.is_empty());
    }

    #[tokio::test]
    async fn saved_plans_cannot_switch_roots_after_library_removal_or_record_changes() {
        let legacy = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let s = state(legacy.path(), false);
        let asset = seed_registered(&s, external.path());
        let mut altered = asset.clone();
        altered.relative_path = "different.mp4".into();
        assert!(catalog::resolve_input(&s, &altered).await.is_err());
        s.db.asset_delete_for_library("registered").unwrap();
        s.db.library_delete("registered").unwrap();
        assert!(catalog::resolve_input(&s, &asset).await.is_err());
        let fallback = legacy.path().join(&asset.relative_path);
        std::fs::create_dir_all(fallback.parent().unwrap()).unwrap();
        std::fs::write(fallback, b"registered-video").unwrap();
        assert!(catalog::resolve_input(&s, &asset).await.is_err());
    }

    #[tokio::test]
    async fn plan_keeps_unselected_recording_context_for_legacy_and_registered_libraries() {
        let temp = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        let mut clips = vec![];
        for (id, offset, signature) in [("a", 0, "tall"), ("b", 100, "wide"), ("c", 200, "tall")] {
            clips.push(Asset {
                id: id.into(),
                room_id: "123456".into(),
                title: "直播标题".into(),
                role: "source".into(),
                started_at: Some(
                    chrono::DateTime::from_timestamp(1_790_300_000 + offset, 0)
                        .unwrap()
                        .to_rfc3339(),
                ),
                bytes: 100,
                metadata: Some(MediaInfo {
                    duration: Some(100.0),
                    signature: signature.into(),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        s.library.write().await.assets = clips.clone();
        let plan = request_json(s.clone(), "/api/plans", json!({"asset_ids":["a","c"]})).await;
        assert_eq!(plan["outputs"].as_array().unwrap().len(), 2);
        assert_eq!(plan["outputs"][1]["reason"], "中间有未纳入该连续段的素材");

        let original = seed_registered(&s, external.path());
        let base = s.db.asset_get(&original.id).unwrap().unwrap();
        s.db.asset_delete_for_library("registered").unwrap();
        for clip in clips {
            let path = external.path().join(format!("{}.mp4", clip.id));
            std::fs::write(&path, b"video").unwrap();
            let mut record = base.clone();
            record.id = format!("registered-{}", clip.id);
            record.source_path = path.canonicalize().unwrap().to_string_lossy().into_owned();
            record.asset.id = record.id.clone();
            record.asset.source_path = record.source_path.clone();
            record.asset.started_at = clip.started_at;
            record.asset.custom_meta =
                Some(json!({"scanner":{"version":1,"role":"source","metadata":clip.metadata}}));
            s.db.asset_put(&record).unwrap();
        }
        let plan = request_json(
            s,
            "/api/plans",
            json!({"asset_ids":["registered-a","registered-c"]}),
        )
        .await;
        assert_eq!(plan["outputs"].as_array().unwrap().len(), 2);
        assert_eq!(plan["outputs"][1]["reason"], "中间有未纳入该连续段的素材");
    }

    #[tokio::test]
    async fn plan_rejects_cross_library_inputs_even_with_matching_room_and_streams() {
        let temp = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        let asset = seed_registered(&s, first.path());
        let mut root: LibraryRoot = s.db.library_get("registered").unwrap().unwrap();
        root.id = "second-library".into();
        root.path = second
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        s.db.library_put(&root.id, &root).unwrap();
        let mut record = s.db.asset_get(&asset.id).unwrap().unwrap();
        let path = second.path().join("recording.mp4");
        std::fs::write(&path, b"video").unwrap();
        record.id = "second-asset".into();
        record.library_id = root.id.clone();
        record.source_path = path.canonicalize().unwrap().to_string_lossy().into_owned();
        record.asset.id = record.id.clone();
        record.asset.library_id = root.id;
        record.asset.source_path = record.source_path.clone();
        s.db.asset_put(&record).unwrap();
        s.library.write().await.assets.push(Asset {
            id: "legacy".into(),
            ..asset.clone()
        });
        for other in ["second-asset", "legacy"] {
            let response = router(s.clone())
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/api/plans")
                        .header("host", "127.0.0.1:4173")
                        .header("authorization", "Bearer test-session")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"asset_ids":[asset.id, other],"include_legacy":true})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert!(String::from_utf8_lossy(&body).contains("同一个素材库"));
        }
    }

    #[tokio::test]
    async fn registered_default_root_keeps_legacy_assets_until_first_scan() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        seed_registered(&s, temp.path());
        s.db.asset_delete_for_library("registered").unwrap();
        let mut root: LibraryRoot = s.db.library_get("registered").unwrap().unwrap();
        root.last_scanned_at = None;
        s.db.library_put(&root.id, &root).unwrap();
        s.library.write().await.assets.push(Asset {
            id: "legacy".into(),
            ..Default::default()
        });
        let response = request(s.clone(), "GET", "/api/snapshot?libraryId=registered").await;
        let value: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["library"]["assets"][0]["id"], "legacy");
        root.last_scanned_at = Some(now());
        s.db.library_put(&root.id, &root).unwrap();
        let response = request(s, "GET", "/api/snapshot?libraryId=registered").await;
        let value: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(value["library"]["assets"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn task_polling_needs_no_youtube_credentials_and_redacts_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        seed(&s, "failed", "paused");
        let response = request(s, "GET", "/api/tasks").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["tasks"].as_array().unwrap().len(), 2);
        let media = &value["tasks"][0];
        let upload = &value["tasks"][1];
        assert_eq!(media["id"], "media:same-id");
        assert_eq!(media["capabilities"]["retry"], true);
        assert_eq!(upload["id"], "upload:same-id");
        assert_eq!(upload["created_at"], Value::Null);
        assert_eq!(upload["progress"], 0.25);
        assert_eq!(upload["capabilities"]["resume"], true);
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("private-session-secret"));
        assert!(!text.contains("private-path"));
    }

    #[tokio::test]
    async fn task_actions_cannot_cancel_the_other_executor_with_the_same_id() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        seed(&s, "running", "running");
        let media = CancellationToken::new();
        let upload = CancellationToken::new();
        s.cancellations.lock().await.extend([
            ("same-id".into(), media.clone()),
            (crate::youtube::upload_token_key("same-id"), upload.clone()),
        ]);
        assert_eq!(
            request(s.clone(), "POST", "/api/tasks/upload:same-id/cancel")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(!media.is_cancelled());
        assert!(!upload.is_cancelled());
        assert_eq!(
            request(s.clone(), "POST", "/api/tasks/upload:same-id/pause")
                .await
                .status(),
            StatusCode::OK
        );
        assert!(upload.is_cancelled());
        assert!(!media.is_cancelled());
        assert_eq!(
            request(s.clone(), "POST", "/api/tasks/media:same-id/cancel")
                .await
                .status(),
            StatusCode::OK
        );
        assert!(media.is_cancelled());
        assert_eq!(
            request(s, "POST", "/api/tasks/media:same-id/cancel")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn capabilities_follow_executor_state_and_headless_disables_only_upload_actions() {
        let temp = tempfile::tempdir().unwrap();
        let s = state(temp.path(), false);
        for (status, resume, retry) in [
            ("queued", false, false),
            ("running", false, false),
            ("completed", false, false),
            ("failed", false, true),
            ("paused", true, false),
            ("interrupted", true, false),
        ] {
            seed(&s, "failed", status);
            let tasks = task_list(&s).await.unwrap();
            let upload = tasks.iter().find(|t| t["kind"] == "upload").unwrap();
            assert_eq!(upload["capabilities"]["resume"], resume, "{status}");
            assert_eq!(upload["capabilities"]["retry"], retry, "{status}");
            assert_eq!(
                upload["capabilities"]["pause"], false,
                "no live token: {status}"
            );
        }
        let headless_root = temp.path().join("headless");
        std::fs::create_dir(&headless_root).unwrap();
        let headless = state(&headless_root, true);
        seed(&headless, "failed", "paused");
        let tasks = task_list(&headless).await.unwrap();
        assert_eq!(tasks[0]["capabilities"]["retry"], true);
        assert_eq!(
            tasks[1]["capabilities"],
            json!({"cancel":false,"pause":false,"resume":false,"retry":false})
        );
        assert_eq!(
            request(headless.clone(), "POST", "/api/youtube/connect")
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            request(headless.clone(), "GET", "/oauth/youtube/callback?code=x")
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let response = request(headless, "GET", "/api/youtube/status").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["enabled"], false);
        assert!(value.get("videos").is_none());
        assert_eq!(value["uploads"].as_array().unwrap().len(), 1);
    }
}
async fn asset(s: &AppState, id: &str) -> Result<Asset> {
    Ok(catalog::asset(s, id).await?)
}
async fn thumbnail(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response> {
    let a = asset(&s, &id).await?;
    let input = catalog::resolve_input(&s, &a).await?;
    let dir = s.config.data.join("thumbnails");
    std::fs::create_dir_all(&dir)?;
    let thumb = dir.join(format!("{}-{}.jpg", a.id, a.modified_ms));
    if !thumb.exists() {
        let _permit = s.thumbnails.acquire().await?;
        if !thumb.exists() {
            let result = tokio::time::timeout(
                Duration::from_secs(25),
                media::command(&s.config.ffmpeg)
                    .args(["-nostdin", "-v", "error", "-ss", "1", "-i"])
                    .arg(input)
                    .args(["-frames:v", "1", "-vf", "scale=400:-2", "-q:v", "4", "-y"])
                    .arg(&thumb)
                    .output(),
            )
            .await??;
            if !result.status.success() {
                let _ = std::fs::remove_file(&thumb);
                return Err(anyhow!("缩略图生成失败").into());
            }
        }
    }
    Ok((
        [(axum::http::header::CONTENT_TYPE, "image/jpeg")],
        tokio::fs::read(thumb).await?,
    )
        .into_response())
}
async fn video(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    req: Request,
) -> Result<Response> {
    let a = asset(&s, &id).await?;
    if a.extension != "mp4" {
        return Err(anyhow!("本版浏览器直接预览仅支持 MP4；FLV 可查看缩略图及媒体参数").into());
    }
    let path = catalog::resolve_input(&s, &a).await?;
    Ok(ServeFile::new(path)
        .oneshot(req)
        .await
        .unwrap()
        .map(Body::new))
}
async fn refresh_room(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Room>> {
    let _permit = s
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| anyhow!("请等待扫描或合并完成后刷新资料"))?;
    if id.parse::<u64>().is_err() {
        return Err(anyhow!("不是有效 B 站房间号").into());
    }
    let mut room = catalog::room(&s, &id).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 U2BUP/0.1")
        .no_proxy()
        .build()?;
    let result=async {
        let response:Value=client.get(format!("https://api.live.bilibili.com/room/v1/Room/get_info?id={id}")).send().await?.error_for_status()?.json().await?;
        if response["code"].as_i64()!=Some(0){bail!("B站返回 {}：{}",response["code"],response["message"]);}
        let data=&response["data"];
        let mut online=json!({"room_id":data["room_id"],"uid":data["uid"],"title":data["title"],"live_status":data["live_status"],"area_name":data["area_name"],"live_time":data["live_time"]});
        if let Some(uid)=data["uid"].as_u64(){
            if let Ok(resp)=client.get(format!("https://api.live.bilibili.com/live_user/v1/Master/info?uid={uid}")).send().await{
                if let Ok(v)=resp.json::<Value>().await {if v["code"].as_i64()==Some(0){online["name"]=v["data"]["info"]["uname"].clone();}}
            }
        }
        Ok::<_,anyhow::Error>(online)
    }.await;
    room.refreshed_at = Some(now());
    match result {
        Ok(v) => {
            room.online = Some(v);
            room.refresh_error = None
        }
        Err(e) => room.refresh_error = Some(format!("{e:#}")),
    };
    s.db.put("room-info", &id, &room)?;
    let mut l = s.library.write().await;
    if let Some(r) = l.rooms.iter_mut().find(|r| r.id == id) {
        *r = room.clone();
        s.db.put("library", "main", &*l)?;
    }
    Ok(Json(room))
}

#[derive(Deserialize)]
struct RenameRequest {
    asset_ids: Vec<String>,
    template: String,
    #[serde(default)]
    find: String,
    #[serde(default)]
    replace: String,
    #[serde(default)]
    regex: bool,
}
#[derive(serde::Serialize, Deserialize)]
struct TitleChange {
    id: String,
    before: String,
    after: String,
}
async fn preview_titles(
    State(s): State<Arc<AppState>>,
    Json(r): Json<RenameRequest>,
) -> Result<Json<Vec<TitleChange>>> {
    if r.template.len() > 1000 || r.find.len() > 1000 || r.replace.len() > 1000 {
        return Err(anyhow!("规则过长").into());
    }
    let regex = if r.regex && !r.find.is_empty() {
        Some(regex::Regex::new(&r.find)?)
    } else {
        None
    };
    let mut changes = Vec::new();
    for (index, id) in r.asset_ids.iter().enumerate() {
        let a = catalog::asset(&s, id).await?;
        let before = a.display_title.as_ref().unwrap_or(&a.title).clone();
        let date = a
            .started_at
            .as_ref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|d| {
                d.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or("未知日期".into());
        let mut after = r
            .template
            .replace("{主播}", &a.room_name)
            .replace("{日期}", &date)
            .replace("{标题}", &before)
            .replace("{序号}", &format!("{:02}", index + 1));
        if !r.find.is_empty() {
            after = if let Some(re) = &regex {
                re.replace_all(&after, r.replace.as_str()).to_string()
            } else {
                after.replace(&r.find, &r.replace)
            };
        }
        if after.trim().is_empty() || after.chars().count() > 300 {
            return Err(anyhow!("生成的显示标题为空或超过 300 字符").into());
        }
        changes.push(TitleChange {
            id: id.clone(),
            before,
            after,
        });
    }
    Ok(Json(changes))
}
async fn apply_titles(
    State(s): State<Arc<AppState>>,
    Json(changes): Json<Vec<TitleChange>>,
) -> Result<Json<Value>> {
    let _permit = s
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| anyhow!("当前有任务运行，请稍后应用标题"))?;
    for c in &changes {
        let a = catalog::asset(&s, &c.id).await?;
        if a.display_title.as_ref().unwrap_or(&a.title) != &c.before {
            return Err(anyhow!("标题已变化，请重新预览").into());
        }
        if c.after.trim().is_empty() || c.after.chars().count() > 300 {
            return Err(anyhow!("显示标题不合法").into());
        }
    }
    for c in changes {
        catalog::set_display_title(&s, &c.id, c.after).await?;
    }
    Ok(Json(json!({"ok":true})))
}
