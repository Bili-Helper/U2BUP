use crate::{jobs, media, model::*, planner, scanner, AppState};
use anyhow::{anyhow, bail, Context};
use axum::{
    body::Body,
    extract::{Path, Request, State},
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
        .route("/api/snapshot", get(snapshot))
        .route("/api/status", get(status))
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
    if host != Some(s.authority.as_str())
        || origin.is_some_and(|v| v != format!("http://{}", s.authority))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"访问来源不匹配，请使用应用提供的本机链接"})),
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
            "u2bup_session_{}={}; HttpOnly; SameSite=Strict; Path=/",
            s.authority.replace(['.', ':'], "_"),
            s.token
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
async fn snapshot(State(s): State<Arc<AppState>>) -> Result<Json<Value>> {
    let library = s.library.read().await.clone();
    Ok(Json(
        json!({"library":library,"scan":s.scan_status.lock().await.clone(),"jobs":s.db.list::<Job>("job")?,"plans":s.db.list::<Plan>("plan")?,"settings":{"library":s.config.library,"output":s.config.output,"data":s.config.data,"ffmpeg":s.config.ffmpeg,"ffprobe":s.config.ffprobe,"version":env!("CARGO_PKG_VERSION"),"mode":"本机 · 单用户"}}),
    ))
}
async fn status(State(s): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(
        json!({"scan":s.scan_status.lock().await.clone(),"jobs":s.db.list::<Job>("job")?}),
    ))
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
    let p = planner::build(&*s.library.read().await, req)?;
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
async fn asset(s: &AppState, id: &str) -> Result<Asset> {
    Ok(s.library
        .read()
        .await
        .assets
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .context("素材不存在")?)
}
async fn thumbnail(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response> {
    let a = asset(&s, &id).await?;
    let input = media::resolve_input(&s.config.library, &a.relative_path)?;
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
    let path = media::resolve_input(&s.config.library, &a.relative_path)?;
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
    if !s.library.read().await.rooms.iter().any(|r| r.id == id) {
        return Err(anyhow!("直播间不存在").into());
    }
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
    let mut l = s.library.write().await;
    let r = l.rooms.iter_mut().find(|r| r.id == id).unwrap();
    r.refreshed_at = Some(now());
    match result {
        Ok(v) => {
            r.online = Some(v);
            r.refresh_error = None
        }
        Err(e) => r.refresh_error = Some(format!("{e:#}")),
    };
    let result = r.clone();
    s.db.put("library", "main", &*l)?;
    Ok(Json(result))
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
    let l = s.library.read().await;
    let mut changes = Vec::new();
    for (index, id) in r.asset_ids.iter().enumerate() {
        let a = l
            .assets
            .iter()
            .find(|a| &a.id == id)
            .context("素材已不存在")?;
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
    let mut l = s.library.write().await;
    for c in &changes {
        let a = l
            .assets
            .iter()
            .find(|a| a.id == c.id)
            .context("素材已不存在")?;
        if a.display_title.as_ref().unwrap_or(&a.title) != &c.before {
            return Err(anyhow!("标题已变化，请重新预览").into());
        }
        if c.after.trim().is_empty() || c.after.chars().count() > 300 {
            return Err(anyhow!("显示标题不合法").into());
        }
    }
    for c in changes {
        l.assets
            .iter_mut()
            .find(|a| a.id == c.id)
            .unwrap()
            .display_title = Some(c.after);
    }
    s.db.put("library", "main", &*l)?;
    Ok(Json(json!({"ok":true})))
}
