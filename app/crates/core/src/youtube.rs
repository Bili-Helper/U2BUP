use crate::{db::Db, media, model::*, web::ApiError, AppState};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

const API: &str = "https://www.googleapis.com/youtube/v3";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const UPLOAD: &str = "https://www.googleapis.com/upload/youtube/v3/videos";
const CHUNK: usize = 8 * 1024 * 1024;
pub(crate) const HEADLESS_REASON: &str =
    "当前 headless 服务仅支持本地媒体处理；YouTube 账号连接和上传请使用桌面版或本机服务";
type HttpResult<T> = std::result::Result<Json<T>, ApiError>;
#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) operation: Arc<Mutex<()>>,
    credentials: Mutex<()>,
    pending: Mutex<Option<Pending>>,
    queue: Mutex<()>,
    #[cfg(test)]
    test: Option<TestEnvironment>,
}
#[cfg(test)]
struct TestEnvironment {
    base: String,
    credentials: std::sync::Mutex<Credentials>,
}
fn endpoint(s: &AppState, production: &str) -> String {
    #[cfg(test)]
    if let Some(t) = &s.youtube.test {
        return format!(
            "{}{}",
            t.base,
            if production == UPLOAD {
                "/upload"
            } else if production == TOKEN {
                "/token"
            } else {
                "/api"
            }
        );
    }
    let _ = s;
    production.into()
}
struct Pending {
    state: String,
    verifier: String,
    redirect: String,
    created: std::time::Instant,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    client_secret: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires: i64,
    #[serde(default)]
    channel_id: String,
}
fn entry(s: &AppState) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        "io.u2bup.youtube",
        &digest(s.config.data.to_string_lossy().as_bytes()),
    )?)
}
fn credentials(s: &AppState) -> Result<Credentials> {
    #[cfg(test)]
    if let Some(t) = &s.youtube.test {
        return Ok(t.credentials.lock().unwrap().clone());
    }
    match entry(s)?.get_password() {
        Ok(v) => Ok(serde_json::from_str(&v)?),
        Err(keyring::Error::NoEntry) => Ok(Credentials::default()),
        Err(e) => Err(e.into()),
    }
}
fn store_credentials(s: &AppState, c: &Credentials) -> Result<()> {
    #[cfg(test)]
    if let Some(t) = &s.youtube.test {
        *t.credentials.lock().unwrap() = c.clone();
        return Ok(());
    }
    entry(s)?.set_password(&serde_json::to_string(c)?)?;
    Ok(())
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .build()?)
}
async fn checked(r: reqwest::Response) -> Result<Value> {
    let status = r.status();
    let value: Value = r.json().await.context("YouTube 返回了无法解析的响应")?;
    if !status.is_success() {
        bail!(
            "YouTube HTTP {}: {}",
            status.as_u16(),
            value["error"]["message"]
                .as_str()
                .or(value["error_description"].as_str())
                .or(value["error"].as_str())
                .unwrap_or("请求失败，请检查授权、配额或服务状态")
        );
    }
    Ok(value)
}
async fn access(s: &AppState) -> Result<String> {
    let _guard = s.youtube.credentials.lock().await;
    let mut c = credentials(s)?;
    if c.access_token.is_empty() {
        bail!("请先连接 YouTube 账号");
    }
    if c.expires < chrono::Utc::now().timestamp() + 90 {
        if c.refresh_token.is_empty() {
            bail!("授权已过期，请重新连接账号");
        }
        let v = checked(
            client()?
                .post(endpoint(s, TOKEN))
                .form(&[
                    ("client_id", c.client_id.as_str()),
                    ("client_secret", c.client_secret.as_str()),
                    ("refresh_token", c.refresh_token.as_str()),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("刷新授权网络失败"))?,
        )
        .await?;
        c.access_token = v["access_token"]
            .as_str()
            .context("缺少 access_token")?
            .into();
        c.expires = chrono::Utc::now().timestamp() + v["expires_in"].as_i64().unwrap_or(3600);
        store_credentials(s, &c)?;
    }
    Ok(c.access_token)
}
pub(crate) async fn api_get(s: &AppState, path: &str, query: &[(&str, &str)]) -> Result<Value> {
    checked(
        client()?
            .get(format!("{}/{path}", endpoint(s, API)))
            .bearer_auth(access(s).await?)
            .query(query)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("YouTube 网络请求失败"))?,
    )
    .await
}
// Workflow endpoints share the same credential store, channel fence and operation lock.
pub(crate) fn workflow_channel(s: &AppState) -> Result<String> {
    if s.config.headless {
        bail!("{HEADLESS_REASON}");
    }
    let c = credentials(s)?;
    if c.channel_id.is_empty() || c.access_token.is_empty() {
        bail!("请先连接 YouTube 频道");
    }
    Ok(c.channel_id)
}
#[derive(Debug)]
struct WorkflowResponseError {
    definitive: bool,
    message: String,
}
impl std::fmt::Display for WorkflowResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for WorkflowResponseError {}
async fn workflow_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    checked(response).await.map_err(|e| {
        WorkflowResponseError {
            definitive: status.is_client_error() && status != reqwest::StatusCode::REQUEST_TIMEOUT,
            message: format!("{e:#}"),
        }
        .into()
    })
}
pub(crate) fn workflow_definitive_failure(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<WorkflowResponseError>()
        .is_some_and(|e| e.definitive)
}
pub(crate) async fn workflow_write(
    s: &AppState,
    method: reqwest::Method,
    path: &str,
    part: &str,
    body: &Value,
    etag: Option<&str>,
) -> Result<Value> {
    workflow_channel(s)?;
    if !["videos", "playlists", "playlistItems"].contains(&path) {
        bail!("不支持的管线写入目标");
    }
    let mut request = client()?
        .request(method, format!("{}/{path}", endpoint(s, API)))
        .query(&[("part", part)])
        .bearer_auth(access(s).await?)
        .json(body);
    if let Some(etag) = etag {
        request = request.header("If-Match", etag);
    }
    workflow_response(
        request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("更新响应未知；重试时必须先核对远端状态"))?,
    )
    .await
}
pub(crate) async fn workflow_thumbnail(
    s: &AppState,
    video_id: &str,
    bytes: Vec<u8>,
) -> Result<Value> {
    workflow_channel(s)?;
    let url = format!(
        "{}/thumbnails/set",
        endpoint(s, UPLOAD).trim_end_matches("/videos")
    );
    workflow_response(
        client()?
            .post(url)
            .query(&[("videoId", video_id), ("uploadType", "media")])
            .bearer_auth(access(s).await?)
            .header("Content-Type", "image/jpeg")
            .body(bytes)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("封面上传响应未知，请在 Studio 核对后重新预览"))?,
    )
    .await
}
pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/youtube", get(snapshot))
        .route("/api/youtube/status", get(status))
        .route("/api/youtube/config", post(configure))
        .route("/api/youtube/connect", post(connect))
        .route("/oauth/youtube/callback", get(callback))
        .route("/api/youtube/disconnect", post(disconnect))
        .route("/api/youtube/sync", post(sync))
        .route("/api/youtube/uploads", post(enqueue))
        .route("/api/youtube/uploads/{id}/resume", post(resume))
        .route("/api/youtube/uploads/{id}/pause", post(pause))
        .route("/api/youtube/batches/preview", post(preview))
        .route("/api/youtube/batches/{id}/apply", post(apply))
}
async fn snapshot(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    if s.config.headless {
        return Ok(Json(json!({
            "enabled": false, "reason": HEADLESS_REASON,
            "configured": false, "connected": false, "channel": null,
            "videos": [], "uploads": upload_summaries(&s.db)?,
            "artifacts": s.db.list::<Artifact>("artifact")?, "batches": []
        })));
    }
    let c = credentials(&s)?;
    let ids =
        s.db.get::<Vec<Value>>("yt-video-list", &c.channel_id)?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v["id"].as_str().map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
    let videos =
        s.db.list::<Value>("yt-video")?
            .into_iter()
            .filter(|v| {
                v["snippet"]["channelId"] == c.channel_id
                    && ids.contains(v["id"].as_str().unwrap_or(""))
            })
            .map(|mut v| {
                v["local_group"] =
                    s.db.get::<Value>("yt-group", v["id"].as_str().unwrap_or(""))
                        .ok()
                        .flatten()
                        .unwrap_or(json!(""));
                v
            })
            .collect::<Vec<_>>();
    let uploads = upload_summaries(&s.db)?;
    Ok(Json(
        json!({"enabled":true,"configured":!c.client_id.is_empty(),"connected":!c.access_token.is_empty(),"channel":s.db.get::<Value>("yt-channel","main")?,"last_sync":s.db.get::<Value>("yt-sync",&c.channel_id)?,"videos":videos,"uploads":uploads,"artifacts":s.db.list::<Artifact>("artifact")?,"batches":s.db.list::<Batch>("yt-batch")?}),
    ))
}
// This polling endpoint only reads local state and never refreshes OAuth tokens.
async fn status(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let uploads = upload_summaries(&s.db)?;
    if s.config.headless {
        return Ok(Json(json!({
            "enabled": false, "reason": HEADLESS_REASON,
            "configured": false, "connected": false, "channel": null,
            "uploads": uploads, "credential_error": null
        })));
    }
    let (configured, connected, channel, credential_error) = match credentials(&s) {
        Ok(c) => (
            !c.client_id.is_empty(),
            !c.access_token.is_empty(),
            s.db.get::<Value>("yt-channel", "main")?,
            None,
        ),
        Err(_) => (
            false,
            false,
            None,
            Some("无法读取账号凭据，请检查系统凭据存储；本地任务仍可使用"),
        ),
    };
    Ok(Json(json!({
        "enabled": true, "configured": configured, "connected": connected, "channel": channel,
        "uploads": uploads, "credential_error": credential_error
    })))
}

pub(crate) fn upload_summaries(db: &Db) -> Result<Vec<Value>> {
    Ok(db
        .list::<UploadJob>("yt-upload")?
        .into_iter()
        .map(|j| {
            json!({
                "id": j.id, "title": j.metadata.title, "artifact_id": j.artifact_id,
                "status": j.status, "bytes": j.bytes, "offset": j.offset,
                "message": j.message, "video_id": j.video_id, "privacy": j.metadata.privacy,
                "channel_id": j.channel_id, "created_at": j.created_at, "updated_at": j.updated_at
            })
        })
        .collect())
}

pub(crate) fn upload_token_key(id: &str) -> String {
    format!("upload:{id}")
}
async fn configure(State(s): State<Arc<AppState>>, Json(v): Json<Value>) -> HttpResult<Value> {
    let _op = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("请先暂停上传或等待远端操作结束"))?;
    let _c = s.youtube.credentials.lock().await;
    let installed = v
        .get("installed")
        .context("请导入 Google Cloud 的桌面应用 OAuth 客户端 JSON（installed）")?;
    let id = installed["client_id"]
        .as_str()
        .filter(|s| s.ends_with(".apps.googleusercontent.com"))
        .context("无效客户端 ID")?;
    let secret = installed["client_secret"]
        .as_str()
        .context("缺少客户端 secret")?;
    store_credentials(
        &s,
        &Credentials {
            client_id: id.into(),
            client_secret: secret.into(),
            ..Default::default()
        },
    )?;
    *s.youtube.pending.lock().await = None;
    Ok(Json(json!({"ok":true})))
}
async fn connect(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let c = credentials(&s)?;
    if c.client_id.is_empty() {
        return Err(anyhow::anyhow!("请先导入 OAuth 客户端配置").into());
    }
    let state = uuid::Uuid::new_v4().simple().to_string();
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let redirect = format!("http://{}/oauth/youtube/callback", s.authority);
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs(&[
        ("client_id", c.client_id.as_str()),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("scope", "https://www.googleapis.com/auth/youtube.force-ssl"),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    *s.youtube.pending.lock().await = Some(Pending {
        state,
        verifier,
        redirect,
        created: std::time::Instant::now(),
    });
    let opened = webbrowser::open(url.as_str()).is_ok();
    Ok(Json(json!({"url":url.as_str(),"opened":opened})))
}
async fn callback(
    State(s): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> std::result::Result<String, ApiError> {
    let mut guard = s.youtube.pending.lock().await;
    let p = guard.as_ref().context("没有待完成的授权")?;
    if p.created.elapsed() > Duration::from_secs(600) || q.get("state") != Some(&p.state) {
        return Err(anyhow::anyhow!("授权 state 无效或已过期").into());
    }
    let p = guard.take().unwrap();
    drop(guard);
    let code = q.get("code").context("授权未完成或已取消")?;
    let _op = s.youtube.operation.lock().await;
    let _c = s.youtube.credentials.lock().await;
    let mut c = credentials(&s)?;
    let v = checked(
        client()?
            .post(endpoint(&s, TOKEN))
            .form(&[
                ("client_id", c.client_id.as_str()),
                ("client_secret", c.client_secret.as_str()),
                ("code", code),
                ("code_verifier", &p.verifier),
                ("redirect_uri", &p.redirect),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("授权交换网络失败，请重新连接"))?,
    )
    .await?;
    c.access_token = v["access_token"]
        .as_str()
        .context("缺少 access_token")?
        .into();
    c.refresh_token = v["refresh_token"].as_str().unwrap_or("").into();
    c.expires = chrono::Utc::now().timestamp() + v["expires_in"].as_i64().unwrap_or(3600);
    let channel = checked(
        client()?
            .get(format!("{API}/channels"))
            .query(&[("part", "snippet,contentDetails"), ("mine", "true")])
            .bearer_auth(&c.access_token)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("读取频道网络失败"))?,
    )
    .await?;
    let channel = channel["items"]
        .as_array()
        .and_then(|a| a.first())
        .context("授权账号没有 YouTube 频道")?;
    c.channel_id = channel["id"].as_str().context("缺少频道 ID")?.into();
    store_credentials(&s, &c)?;
    s.db.put("yt-channel", "main", channel)?;
    Ok("YouTube 已连接。请关闭此授权标签页，返回 U2BUP 刷新频道。".into())
}
async fn disconnect(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let _op = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("请先暂停上传或等待远端操作结束"))?;
    let _c = s.youtube.credentials.lock().await;
    let mut c = credentials(&s)?;
    if !c.refresh_token.is_empty() {
        let r = client()?
            .post("https://oauth2.googleapis.com/revoke")
            .form(&[("token", &c.refresh_token)])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("撤销授权网络失败，请重试"))?;
        if !r.status().is_success() && r.status() != reqwest::StatusCode::BAD_REQUEST {
            return Err(anyhow::anyhow!("撤销授权失败").into());
        }
    }
    c.access_token.clear();
    c.refresh_token.clear();
    c.channel_id.clear();
    store_credentials(&s, &c)?;
    Ok(Json(json!({"ok":true})))
}
async fn sync(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let _op = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("YouTube 操作进行中"))?;
    let channels = api_get(
        &s,
        "channels",
        &[("part", "snippet,contentDetails"), ("mine", "true")],
    )
    .await?;
    let ch = channels["items"]
        .as_array()
        .and_then(|a| a.first())
        .context("没有频道")?;
    s.db.put("yt-channel", "main", ch)?;
    let playlist = ch["contentDetails"]["relatedPlaylists"]["uploads"]
        .as_str()
        .context("缺少上传列表")?;
    let mut token = String::new();
    let mut found = Vec::new();
    let mut page_tokens = std::collections::HashSet::new();
    let mut requested_ids = std::collections::HashSet::new();
    let mut found_ids = std::collections::HashSet::new();
    let mut records = 0usize;
    let mut duplicates = 0usize;
    loop {
        if !page_tokens.insert(token.clone()) {
            return Err(anyhow::anyhow!(
                "YouTube 返回重复分页标记，已停止同步并保留上次完整视频列表；请稍后重试"
            )
            .into());
        }
        let v = api_get(
            &s,
            "playlistItems",
            &[
                ("part", "contentDetails"),
                ("playlistId", playlist),
                ("maxResults", "50"),
                ("pageToken", &token),
            ],
        )
        .await?;
        let ids = v["items"]
            .as_array()
            .context("无效视频列表")?
            .iter()
            .filter_map(|i| i["contentDetails"]["videoId"].as_str())
            .filter(|id| {
                records += 1;
                if requested_ids.insert((*id).to_owned()) {
                    true
                } else {
                    duplicates += 1;
                    false
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        if !ids.is_empty() {
            let videos = api_get(
                &s,
                "videos",
                &[
                    ("part", "snippet,status,contentDetails,statistics"),
                    ("id", &ids),
                ],
            )
            .await?;
            for video in videos["items"].as_array().context("无效视频响应")? {
                let id = video["id"].as_str().context("视频 ID 缺失")?;
                if found_ids.insert(id.to_owned()) {
                    found.push(video.clone());
                }
            }
        }
        token = v["nextPageToken"].as_str().unwrap_or("").into();
        if token.is_empty() {
            break;
        }
    }
    // Replace only after the entire pagination succeeds.
    s.db.put("yt-video-list", &credentials(&s)?.channel_id, &found)?;
    for v in &found {
        s.db.put("yt-video", v["id"].as_str().context("视频 ID 缺失")?, v)?;
    }
    let summary =
        json!({"count":found.len(),"records":records,"duplicates":duplicates,"completed_at":now()});
    s.db.put("yt-sync", &credentials(&s)?.channel_id, &summary)?;
    Ok(Json(summary))
}
#[derive(Clone, Serialize, Deserialize)]
struct Metadata {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "private")]
    privacy: String,
    #[serde(default = "category")]
    category_id: String,
    made_for_kids: bool,
}
fn private() -> String {
    "private".into()
}
fn category() -> String {
    "22".into()
}
fn validate_metadata(m: &Metadata) -> Result<()> {
    if m.title.trim().is_empty() || m.title.chars().count() > 100 || m.title.contains(['<', '>']) {
        bail!("标题应为 1–100 字且不含尖括号");
    }
    if m.description.len() > 5000 || m.description.contains(['<', '>']) {
        bail!("描述超过 5000 字节或包含尖括号");
    }
    if !["private", "unlisted", "public"].contains(&m.privacy.as_str()) {
        bail!("无效公开状态");
    }
    if m.tags
        .iter()
        .map(|t| t.chars().count() + if t.contains(' ') { 2 } else { 0 })
        .sum::<usize>()
        + m.tags.len().saturating_sub(1)
        > 500
    {
        bail!("标签总长度超过 500");
    }
    if m.category_id.is_empty() || !m.category_id.chars().all(|c| c.is_ascii_digit()) {
        bail!("无效分类 ID");
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
struct UploadJob {
    id: String,
    artifact_id: String,
    path: String,
    bytes: u64,
    modified_ms: u64,
    metadata: Metadata,
    channel_id: String,
    status: String,
    offset: u64,
    session: Option<String>,
    video_id: Option<String>,
    message: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}
fn save_upload(db: &Db, job: &mut UploadJob) -> Result<()> {
    job.updated_at = Some(now());
    db.put("yt-upload", &job.id, job)
}
#[derive(Deserialize)]
struct Enqueue {
    artifact_ids: Vec<String>,
    metadata: Metadata,
    #[serde(default)]
    use_workflow_metadata: bool,
}
pub(crate) fn recover(db: &Db) -> Result<()> {
    for mut j in db.list::<UploadJob>("yt-upload")? {
        if ["running", "queued"].contains(&j.status.as_str()) {
            j.status = "interrupted".into();
            j.message = "服务中断，点击继续以查询远端进度".into();
            save_upload(db, &mut j)?;
        }
    }
    Ok(())
}
async fn enqueue(State(s): State<Arc<AppState>>, Json(r): Json<Enqueue>) -> HttpResult<Value> {
    let _queue = s.youtube.queue.lock().await;
    validate_metadata(&r.metadata)?;
    let channel = credentials(&s)?.channel_id;
    if channel.is_empty() {
        return Err(anyhow::anyhow!("请先连接频道").into());
    }
    if r.artifact_ids.is_empty() || r.artifact_ids.len() > 100 {
        return Err(anyhow::anyhow!("每批选择 1–100 个成品").into());
    }
    let old = s.db.list::<UploadJob>("yt-upload")?;
    let mut jobs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in &r.artifact_ids {
        if !seen.insert(id)
            || old
                .iter()
                .any(|j| j.artifact_id == *id && j.channel_id == channel)
        {
            return Err(anyhow::anyhow!("成品已存在上传记录，请继续原任务，避免重复上传").into());
        }
        let a =
            s.db.get::<Artifact>("artifact", id)?
                .context("成品不存在")?;
        let path = std::path::Path::new(&a.path).canonicalize()?;
        if !path.starts_with(&s.config.output) {
            return Err(anyhow::anyhow!("成品必须位于输出目录").into());
        }
        let stat = std::fs::metadata(&path)?;
        if stat.len() != a.bytes || a.bytes > 256_000_000_000 || a.duration > 43200.0 {
            return Err(anyhow::anyhow!("成品已变化或超过 YouTube 上传限制").into());
        }
        let mut metadata = r.metadata.clone();
        if r.use_workflow_metadata {
            let prepared = crate::workflow::prepared_upload(&s.db, &a.source_ids)?
                .context("成品没有可继承的素材元信息")?;
            let library = s.library.read().await;
            for source_id in &a.source_ids {
                let source = library
                    .assets
                    .iter()
                    .find(|asset| asset.id == *source_id)
                    .context("成品来源素材已不存在，请重新检查上传元信息")?;
                if source.display_title.as_ref().unwrap_or(&source.title) != &prepared.title {
                    return Err(anyhow::anyhow!(
                        "素材标题在管线处理后变化，请重新应用管线元信息再上传"
                    )
                    .into());
                }
            }
            metadata.title = prepared.title;
            metadata.description = prepared.description;
            metadata.tags = prepared.tags;
            metadata.category_id = prepared.category_id;
            metadata.privacy = prepared.privacy;
        }
        metadata.title = metadata.title.replace(
            "{文件名}",
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
        );
        validate_metadata(&metadata)?;
        jobs.push(UploadJob {
            id: uuid::Uuid::new_v4().to_string(),
            artifact_id: id.clone(),
            path: path.to_string_lossy().into(),
            bytes: a.bytes,
            modified_ms: media::modified_ms(&stat),
            metadata,
            channel_id: channel.clone(),
            status: "queued".into(),
            offset: 0,
            session: None,
            video_id: None,
            message: "等待上传".into(),
            created_at: Some(now()),
            updated_at: Some(now()),
        });
    }
    for j in &jobs {
        s.db.put("yt-upload", &j.id, j)?;
        schedule(s.clone(), j.clone()).await;
    }
    Ok(Json(json!({"queued":jobs.len()})))
}
async fn schedule(s: Arc<AppState>, mut j: UploadJob) {
    let cancel = CancellationToken::new();
    s.cancellations
        .lock()
        .await
        .insert(upload_token_key(&j.id), cancel.clone());
    tokio::spawn(async move {
        let permit = tokio::select! {p=s.youtube.operation.lock()=>Some(p),_=cancel.cancelled()=>None,_=s.shutdown.cancelled()=>None};
        let result = if permit.is_some() {
            tokio::select! {r=upload(&s,&mut j)=>r,_=cancel.cancelled()=>Err(anyhow::anyhow!("已暂停，可继续上传")),_=s.shutdown.cancelled()=>Err(anyhow::anyhow!("服务关闭，上传中断"))}
        } else {
            Err(anyhow::anyhow!("已暂停"))
        };
        if let Err(e) = result {
            j.status = if cancel.is_cancelled() {
                "paused"
            } else if s.shutdown.is_cancelled() {
                "interrupted"
            } else {
                "failed"
            }
            .into();
            j.message = format!("{e:#}");
        }
        let _ = save_upload(&s.db, &mut j);
        s.cancellations
            .lock()
            .await
            .remove(&upload_token_key(&j.id));
    });
}
pub(crate) async fn pause(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HttpResult<Value> {
    s.cancellations
        .lock()
        .await
        .get(&upload_token_key(&id))
        .context("上传未在运行")?
        .cancel();
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn resume(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HttpResult<Value> {
    let _queue = s.youtube.queue.lock().await;
    let mut tokens = s.cancellations.lock().await;
    if tokens.contains_key(&upload_token_key(&id)) {
        return Err(anyhow::anyhow!("任务已在队列中").into());
    }
    let mut j =
        s.db.get::<UploadJob>("yt-upload", &id)?
            .context("任务不存在")?;
    if j.status == "completed" {
        return Err(anyhow::anyhow!("视频已上传").into());
    }
    j.status = "queued".into();
    j.message = "等待继续上传".into();
    save_upload(&s.db, &mut j)?;
    tokens.insert(upload_token_key(&id), CancellationToken::new());
    drop(tokens);
    schedule(s.clone(), j).await;
    Ok(Json(json!({"ok":true})))
}
fn validate_session(s: &AppState, url: &str) -> Result<()> {
    #[cfg(test)]
    if let Some(t) = &s.youtube.test {
        if url == format!("{}/session", t.base) {
            return Ok(());
        }
    }
    let _ = s;
    session_url(url)
}
fn session_url(url: &str) -> Result<()> {
    let u = reqwest::Url::parse(url)?;
    if u.scheme() != "https"
        || u.host_str() != Some("www.googleapis.com")
        || !u.path().starts_with("/upload/")
        || !u.username().is_empty()
        || u.password().is_some()
    {
        bail!("上传会话地址不受信任");
    }
    Ok(())
}
fn remote_offset(r: &reqwest::Response, total: u64) -> Result<u64> {
    let n = match r.headers().get("range") {
        Some(v) => v
            .to_str()?
            .strip_prefix("bytes=0-")
            .context("无效续传 Range")?
            .parse::<u64>()?
            .checked_add(1)
            .context("Range 溢出")?,
        None => 0,
    };
    if n > total {
        bail!("远端进度超过文件大小");
    }
    Ok(n)
}
async fn upload(s: &AppState, j: &mut UploadJob) -> Result<()> {
    if credentials(s)?.channel_id != j.channel_id {
        bail!("当前频道与任务频道不同，请连接原频道");
    }
    let mut file = tokio::fs::File::open(&j.path).await?;
    let stat = file.metadata().await?;
    if stat.len() != j.bytes || media::modified_ms(&stat) != j.modified_ms {
        bail!("上传源成品发生变化");
    }
    j.status = "running".into();
    j.message = "连接 YouTube 断点上传服务".into();
    save_upload(&s.db, j)?;
    if j.session.is_none() {
        let r=client()?.post(endpoint(s, UPLOAD)).query(&[("uploadType","resumable"),("part","snippet,status"),("notifySubscribers","false")]).bearer_auth(access(s).await?).header("X-Upload-Content-Length",j.bytes).header("X-Upload-Content-Type","video/mp4").json(&json!({"snippet":{"title":j.metadata.title,"description":j.metadata.description,"tags":j.metadata.tags,"categoryId":j.metadata.category_id},"status":{"privacyStatus":j.metadata.privacy,"selfDeclaredMadeForKids":j.metadata.made_for_kids}})).send().await.map_err(|_|anyhow::anyhow!("创建上传会话网络失败，尚未发送媒体数据"))?;
        if !r.status().is_success() {
            checked(r).await?;
            bail!("创建上传会话失败");
        }
        let url = r
            .headers()
            .get("location")
            .context("缺少上传会话地址")?
            .to_str()?
            .to_owned();
        validate_session(s, &url)?;
        j.session = Some(url);
        save_upload(&s.db, j)?;
    }
    let url = j.session.clone().unwrap();
    validate_session(s, &url)?;
    let mut failures = 0u32;
    let mut query = true;
    loop {
        let stat = file.metadata().await?;
        if stat.len() != j.bytes || media::modified_ms(&stat) != j.modified_ms {
            bail!("上传期间源成品发生变化");
        }
        let mut request = client()?.put(&url).bearer_auth(access(s).await?);
        let is_query = query;
        if query {
            request = request
                .header("Content-Range", format!("bytes */{}", j.bytes))
                .header("Content-Length", 0);
        } else {
            if j.offset >= j.bytes {
                bail!("远端未确认完成，请继续原任务查询状态");
            }
            file.seek(std::io::SeekFrom::Start(j.offset)).await?;
            let len = (j.bytes - j.offset).min(CHUNK as u64) as usize;
            let mut bytes = vec![0; len];
            file.read_exact(&mut bytes).await?;
            request = request
                .header("Content-Type", "video/mp4")
                .header(
                    "Content-Range",
                    format!(
                        "bytes {}-{}/{}",
                        j.offset,
                        j.offset + len as u64 - 1,
                        j.bytes
                    ),
                )
                .body(bytes);
        }
        let response = request.send().await;
        match response {
            Ok(r) if r.status().as_u16() == 308 => {
                let next = remote_offset(&r, j.bytes)?;
                if !is_query && next <= j.offset {
                    failures += 1;
                    query = true;
                } else {
                    if next > j.offset {
                        failures = 0;
                    }
                    query = false;
                }
                j.offset = next;
                j.message = format!("已上传 {:.1}%", 100.0 * j.offset as f64 / j.bytes as f64);
                save_upload(&s.db, j)?;
            }
            Ok(r) if r.status().is_success() => {
                let v = checked(r).await?;
                j.video_id = Some(
                    v["id"]
                        .as_str()
                        .context("上传响应缺少视频 ID；请继续原任务查询")?
                        .into(),
                );
                j.offset = j.bytes;
                j.status = "completed".into();
                j.message = "上传完成，YouTube 仍可能在处理视频".into();
                save_upload(&s.db, j)?;
                return Ok(());
            }
            Ok(r) if r.status().is_server_error() || r.status().as_u16() == 429 => {
                failures += 1;
                query = true;
                let wait = r
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.parse::<u64>().ok())
                    .unwrap_or(1u64 << failures.min(5))
                    .min(60);
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
            Ok(r) if r.status().as_u16() == 404 || r.status().as_u16() == 410 => {
                bail!(
                    "上传会话已过期；请先核对频道是否已生成视频。为避免重复上传，不自动创建新会话"
                );
            }
            Ok(r) => {
                checked(r).await?;
                bail!("未识别的上传响应");
            }
            Err(_) => {
                failures += 1;
                query = true;
                tokio::time::sleep(Duration::from_secs(1u64 << failures.min(5))).await;
            }
        }
        if failures >= 5 {
            bail!("多次网络错误或进度未前进，已保存会话；可稍后继续");
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Batch {
    id: String,
    channel_id: String,
    changes: Vec<Change>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Change {
    id: String,
    before: Value,
    after: Value,
    part: String,
    status: String,
    message: String,
}
#[derive(Deserialize)]
struct Edit {
    ids: Vec<String>,
    #[serde(default)]
    find: String,
    #[serde(default)]
    replace: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    prefix: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
    privacy: Option<String>,
    group: Option<String>,
}
pub(crate) fn writable(v: &Value, part: &str) -> Value {
    let fields = if part == "snippet" {
        vec![
            "title",
            "description",
            "tags",
            "categoryId",
            "defaultLanguage",
        ]
    } else {
        vec![
            "privacyStatus",
            "embeddable",
            "license",
            "publicStatsViewable",
            "publishAt",
            "selfDeclaredMadeForKids",
            "containsSyntheticMedia",
        ]
    };
    let mut map = serde_json::Map::new();
    for f in fields {
        if let Some(x) = v.get(part).and_then(|p| p.get(f)) {
            map.insert(f.into(), x.clone());
        }
    }
    Value::Object(map)
}
fn edit_video(v: &Value, r: &Edit) -> Result<(Value, String)> {
    let mut out = json!({"id":v["id"]});
    let mut parts = Vec::new();
    if !r.find.is_empty() || !r.prefix.is_empty() || r.description.is_some() || r.tags.is_some() {
        let mut snippet = writable(v, "snippet");
        let title = snippet["title"].as_str().context("远端标题缺失")?;
        let changed = if r.find.is_empty() {
            title.to_owned()
        } else if r.regex {
            regex::Regex::new(&r.find)?
                .replace_all(title, &r.replace)
                .into_owned()
        } else {
            title.replace(&r.find, &r.replace)
        };
        snippet["title"] = json!(format!("{}{}", r.prefix, changed));
        if let Some(d) = &r.description {
            snippet["description"] = json!(d);
        }
        if let Some(t) = &r.tags {
            snippet["tags"] = json!(t);
        }
        let m = Metadata {
            title: snippet["title"].as_str().unwrap().into(),
            description: snippet["description"].as_str().unwrap_or("").into(),
            tags: serde_json::from_value(
                snippet["tags"]
                    .as_array()
                    .map(|a| json!(a))
                    .unwrap_or(json!([])),
            )?,
            privacy: private(),
            category_id: snippet["categoryId"]
                .as_str()
                .context("远端分类缺失")?
                .into(),
            made_for_kids: false,
        };
        validate_metadata(&m)?;
        out["snippet"] = snippet;
        parts.push("snippet");
    }
    if let Some(p) = &r.privacy {
        if !["private", "unlisted", "public"].contains(&p.as_str()) {
            bail!("无效公开状态");
        }
        let mut status = writable(v, "status");
        if status.get("publishAt").is_some() {
            bail!("定时发布视频暂不支持批改公开状态，请先在 Studio 处理排期");
        }
        status["privacyStatus"] = json!(p);
        out["status"] = status;
        parts.push("status");
    }
    if parts.is_empty() && r.group.is_none() {
        bail!("没有要应用的变更");
    }
    Ok((out, parts.join(",")))
}
async fn preview(State(s): State<Arc<AppState>>, Json(r): Json<Edit>) -> HttpResult<Batch> {
    if r.ids.is_empty() || r.ids.len() > 100 {
        return Err(anyhow::anyhow!("每批选择 1–100 个视频").into());
    }
    let channel = credentials(&s)?.channel_id;
    let mut changes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in &r.ids {
        if !seen.insert(id) {
            continue;
        }
        let v =
            s.db.get::<Value>("yt-video", id)?
                .context("请先同步频道视频")?;
        if v["snippet"]["channelId"] != channel {
            return Err(anyhow::anyhow!("视频不属于当前频道").into());
        }
        let (mut after, part) = edit_video(&v, &r)?;
        if let Some(g) = &r.group {
            after["local_group"] = json!(g);
        }
        changes.push(Change {
            id: id.clone(),
            before: v,
            after,
            part,
            status: "pending".into(),
            message: String::new(),
        });
    }
    let b = Batch {
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: channel,
        changes,
    };
    s.db.put("yt-batch", &b.id, &b)?;
    Ok(Json(b))
}
async fn apply(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> HttpResult<Batch> {
    let _op = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("YouTube 操作进行中"))?;
    let mut b = s.db.get::<Batch>("yt-batch", &id)?.context("预览不存在")?;
    if b.channel_id != credentials(&s)?.channel_id {
        return Err(anyhow::anyhow!("当前频道与预览不同").into());
    }
    for i in 0..b.changes.len() {
        if b.changes[i].status == "completed" {
            continue;
        }
        let c = &mut b.changes[i];
        let result = async {
            let response =
                api_get(&s, "videos", &[("part", "snippet,status"), ("id", &c.id)]).await?;
            let current = response["items"]
                .as_array()
                .and_then(|a| a.first())
                .context("远端视频不存在")?;
            for part in c.part.split(',').filter(|p| !p.is_empty()) {
                if writable(current, part) != writable(&c.before, part) {
                    bail!("远端信息已变化，请重新生成预览");
                }
            }
            if !c.part.is_empty() {
                let mut body = c.after.clone();
                body.as_object_mut().unwrap().remove("local_group");
                let mut request = client()?
                    .put(format!("{}/videos", endpoint(&s, API)))
                    .query(&[("part", &c.part)])
                    .bearer_auth(access(&s).await?)
                    .json(&body);
                if let Some(etag) = current["etag"].as_str() {
                    request = request.header("If-Match", etag);
                }
                let updated = checked(request.send().await.map_err(|_| {
                    anyhow::anyhow!("更新响应未知，请同步后重新预览，避免重复提交")
                })?)
                .await?;
                let mut saved = c.before.clone();
                for part in c.part.split(',') {
                    saved[part] = updated[part].clone();
                }
                if let Some(etag) = updated.get("etag") {
                    saved["etag"] = etag.clone();
                }
                s.db.put("yt-video", &c.id, &saved)?;
            }
            if let Some(g) = c.after.get("local_group") {
                s.db.put("yt-group", &c.id, g)?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => {
                c.status = "completed".into();
                c.message = "已应用".into();
            }
            Err(e) => {
                c.status = "failed".into();
                c.message = format!("{e:#}");
            }
        }
        s.db.put("yt-batch", &b.id, &b)?;
    }
    Ok(Json(b))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_preserves_unedited_fields() {
        let v = json!({"id":"v","snippet":{"title":"hello","description":"keep","categoryId":"22","tags":["old"],"defaultLanguage":"zh","thumbnails":{}},"status":{"privacyStatus":"private","license":"creativeCommon"}});
        let r: Edit =
            serde_json::from_value(json!({"ids":["v"],"find":"hello","replace":"世界"})).unwrap();
        let (b, p) = edit_video(&v, &r).unwrap();
        assert_eq!(p, "snippet");
        assert_eq!(b["snippet"]["description"], "keep");
        assert_eq!(b["snippet"]["tags"], json!(["old"]));
        assert!(b.get("status").is_none());
        assert!(b["snippet"].get("thumbnails").is_none());
    }
    #[test]
    fn privacy_preserves_status_and_rejects_scheduled() {
        let mut v = json!({"id":"v","status":{"privacyStatus":"private","license":"creativeCommon","selfDeclaredMadeForKids":true}});
        let r: Edit = serde_json::from_value(json!({"ids":["v"],"privacy":"unlisted"})).unwrap();
        let (b, p) = edit_video(&v, &r).unwrap();
        assert_eq!(p, "status");
        assert_eq!(b["status"]["license"], "creativeCommon");
        assert_eq!(b["status"]["selfDeclaredMadeForKids"], true);
        v["status"]["publishAt"] = json!("future");
        assert!(edit_video(&v, &r).is_err());
    }
    #[test]
    fn rejects_token_exfiltration() {
        assert!(session_url("https://evil.example/upload/a").is_err());
        assert!(session_url("http://www.googleapis.com/upload/a").is_err());
        assert!(
            session_url("https://www.googleapis.com/upload/youtube/v3/videos?upload_id=x").is_ok()
        );
    }
}
#[cfg(test)]
pub(crate) mod protocol_tests {
    use super::*;
    use axum::{
        body::Bytes,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::put,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Mock {
        base: String,
        received: Mutex<Vec<u8>>,
        posts: AtomicUsize,
        chunks: AtomicUsize,
        total: usize,
    }
    async fn initiate(State(m): State<Arc<Mock>>, Json(body): Json<Value>) -> Response {
        assert_eq!(body["status"]["privacyStatus"], "private");
        assert_eq!(body["status"]["selfDeclaredMadeForKids"], false);
        m.posts.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            [("location", format!("{}/session", m.base))],
        )
            .into_response()
    }
    async fn chunk(State(m): State<Arc<Mock>>, headers: HeaderMap, body: Bytes) -> Response {
        assert_eq!(headers["authorization"], "Bearer test-access");
        let range = headers["content-range"].to_str().unwrap();
        let mut data = m.received.lock().await;
        if !range.starts_with("bytes */") {
            let expected = format!(
                "bytes {}-{}/{}",
                data.len(),
                data.len() + body.len() - 1,
                m.total
            );
            assert_eq!(range, expected);
            let n = m.chunks.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                data.extend_from_slice(&body[..262144]);
                return (StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "0")]).into_response();
            }
            if n == 1 {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error":{"message":"simulated quota interruption"}})),
                )
                    .into_response();
            }
            data.extend_from_slice(&body);
        }
        if data.len() == m.total {
            return (StatusCode::CREATED, Json(json!({"id":"uploaded-once"}))).into_response();
        }
        let mut response = StatusCode::PERMANENT_REDIRECT.into_response();
        if !data.is_empty() {
            response.headers_mut().insert(
                "range",
                format!("bytes=0-{}", data.len() - 1).parse().unwrap(),
            );
        }
        response
    }
    pub(crate) fn state(root: &std::path::Path, base: String) -> AppState {
        AppState {
            config: crate::Config {
                library: root.into(),
                data: root.into(),
                output: root.into(),
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
                port: 0,
                bind: std::net::Ipv4Addr::LOCALHOST.into(),
                public_origin: None,
                headless: false,
            },
            db: Db::open(&root.join("test.sqlite")).unwrap(),
            library: tokio::sync::RwLock::new(Library::default()),
            scan_status: Mutex::new(ScanStatus::default()),
            operation: Arc::new(Mutex::new(())),
            cancellations: Mutex::new(Default::default()),
            thumbnails: tokio::sync::Semaphore::new(1),
            token: "local".into(),
            authority: "127.0.0.1:0".into(),
            origin: "http://127.0.0.1:0".into(),
            shutdown: CancellationToken::new(),
            _lock: std::fs::File::create(root.join("lock")).unwrap(),
            youtube: Runtime {
                test: Some(TestEnvironment {
                    base,
                    credentials: std::sync::Mutex::new(Credentials {
                        access_token: "test-access".into(),
                        expires: chrono::Utc::now().timestamp() + 3600,
                        channel_id: "channel".into(),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            },
        }
    }
    #[tokio::test]
    async fn resumable_upload_recovers_server_offset_and_never_reinserts() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = (0..CHUNK + 1048576)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        let path = temp.path().join("video.mp4");
        std::fs::write(&path, &bytes).unwrap();
        let stat = std::fs::metadata(&path).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let m = Arc::new(Mock {
            base: base.clone(),
            received: Mutex::new(Vec::new()),
            posts: AtomicUsize::new(0),
            chunks: AtomicUsize::new(0),
            total: bytes.len(),
        });
        let router = Router::new()
            .route("/upload", post(initiate))
            .route("/session", put(chunk))
            .layer(axum::extract::DefaultBodyLimit::disable())
            .with_state(m.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let s = state(temp.path(), base);
        let mut j = UploadJob {
            id: "job".into(),
            artifact_id: "artifact".into(),
            path: path.to_string_lossy().into(),
            bytes: bytes.len() as u64,
            modified_ms: media::modified_ms(&stat),
            metadata: Metadata {
                title: "测试".into(),
                description: String::new(),
                tags: vec![],
                privacy: private(),
                category_id: category(),
                made_for_kids: false,
            },
            channel_id: "channel".into(),
            status: "queued".into(),
            offset: 0,
            session: None,
            video_id: None,
            message: String::new(),
            created_at: Some(now()),
            updated_at: Some(now()),
        };
        assert!(upload(&s, &mut j)
            .await
            .unwrap_err()
            .to_string()
            .contains("simulated quota"));
        let mut persisted = s.db.get::<UploadJob>("yt-upload", "job").unwrap().unwrap();
        assert_eq!(persisted.offset, 262144);
        assert!(persisted.session.is_some());
        upload(&s, &mut persisted).await.unwrap();
        assert_eq!(persisted.status, "completed");
        assert_eq!(persisted.video_id.as_deref(), Some("uploaded-once"));
        assert_eq!(*m.received.lock().await, bytes);
        assert_eq!(m.posts.load(Ordering::SeqCst), 1);
        // A crash after the last PUT can be recovered from the same session.
        persisted.status = "interrupted".into();
        persisted.video_id = None;
        persisted.offset = 262144;
        upload(&s, &mut persisted).await.unwrap();
        assert_eq!(persisted.video_id.as_deref(), Some("uploaded-once"));
        assert_eq!(m.posts.load(Ordering::SeqCst), 1);
        std::fs::write(&path, b"changed").unwrap();
        assert!(upload(&s, &mut persisted)
            .await
            .unwrap_err()
            .to_string()
            .contains("变化"));
        server.abort();
    }
    #[tokio::test]
    async fn channel_sync_deduplicates_pages_and_preserves_the_previous_list_on_token_cycles() {
        struct Pages {
            repeat: std::sync::atomic::AtomicBool,
            requested: Mutex<Vec<String>>,
        }
        async fn channel() -> Json<Value> {
            Json(
                json!({"items":[{"id":"channel","contentDetails":{"relatedPlaylists":{"uploads":"uploads"}}}]}),
            )
        }
        async fn playlist(
            State(p): State<Arc<Pages>>,
            Query(q): Query<std::collections::HashMap<String, String>>,
        ) -> Json<Value> {
            let page = q.get("pageToken").map(String::as_str).unwrap_or("");
            if page.is_empty() {
                Json(
                    json!({"items":[{"contentDetails":{"videoId":"v1"}},{"contentDetails":{"videoId":"v1"}},{"contentDetails":{"videoId":"v2"}}],"nextPageToken":"next"}),
                )
            } else {
                let mut result = json!({"items":[{"contentDetails":{"videoId":"v2"}},{"contentDetails":{"videoId":"v3"}}]});
                if p.repeat.load(Ordering::SeqCst) {
                    result["nextPageToken"] = json!("next");
                }
                Json(result)
            }
        }
        async fn videos(
            State(p): State<Arc<Pages>>,
            Query(q): Query<std::collections::HashMap<String, String>>,
        ) -> Json<Value> {
            p.requested.lock().await.push(q["id"].clone());
            let mut items = q["id"]
                .split(',')
                .map(|id| json!({"id":id,"snippet":{"channelId":"channel","title":id}}))
                .collect::<Vec<_>>();
            items.push(items[0].clone());
            Json(json!({"items":items}))
        }
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let pages = Arc::new(Pages {
            repeat: std::sync::atomic::AtomicBool::new(false),
            requested: Mutex::new(vec![]),
        });
        let router = Router::new()
            .route("/api/channels", get(channel))
            .route("/api/playlistItems", get(playlist))
            .route("/api/videos", get(videos))
            .with_state(pages.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let s = Arc::new(state(temp.path(), base));
        s.db.put("yt-group", "v1", &json!("本地分组")).unwrap();
        let result = sync(State(s.clone())).await.ok().unwrap().0;
        assert_eq!(result["count"], 3);
        assert_eq!(result["records"], 5);
        assert_eq!(result["duplicates"], 2);
        assert!(result["completed_at"].as_str().is_some());
        assert_eq!(
            s.db.get::<Value>("yt-sync", "channel").unwrap().unwrap(),
            result
        );
        assert_eq!(*pages.requested.lock().await, vec!["v1,v2", "v3"]);
        let saved =
            s.db.get::<Vec<Value>>("yt-video-list", "channel")
                .unwrap()
                .unwrap();
        assert_eq!(
            saved
                .iter()
                .map(|v| v["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["v1", "v2", "v3"]
        );
        assert_eq!(s.db.list::<Value>("yt-video").unwrap().len(), 3);
        assert_eq!(
            s.db.get::<Value>("yt-group", "v1").unwrap().unwrap(),
            "本地分组"
        );
        pages.repeat.store(true, Ordering::SeqCst);
        let baseline = vec![json!({"id":"previous-complete-list"})];
        s.db.put("yt-video-list", "channel", &baseline).unwrap();
        let error = match sync(State(s.clone())).await {
            Ok(_) => panic!("cyclic pagination must fail"),
            Err(e) => e,
        };
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            s.db.get::<Vec<Value>>("yt-video-list", "channel")
                .unwrap()
                .unwrap(),
            baseline
        );
        server.abort();
    }

    #[tokio::test]
    async fn invalid_oauth_state_does_not_consume_pending_authorization() {
        let temp = tempfile::tempdir().unwrap();
        let s = Arc::new(state(temp.path(), String::new()));
        *s.youtube.pending.lock().await = Some(Pending {
            state: "correct".into(),
            verifier: "x".into(),
            redirect: "x".into(),
            created: std::time::Instant::now(),
        });
        let q = std::collections::HashMap::from([
            ("state".into(), "wrong".into()),
            ("code".into(), "code".into()),
        ]);
        assert!(callback(State(s.clone()), Query(q)).await.is_err());
        assert!(s.youtube.pending.lock().await.is_some());
    }

    #[tokio::test]
    async fn batch_rechecks_remote_state_and_preserves_fields_on_update() {
        let temp = tempfile::tempdir().unwrap();
        let original = json!({"id":"v","etag":"etag-1","snippet":{"channelId":"channel","title":"old","description":"retained","categoryId":"22","tags":["keep"],"defaultLanguage":"zh"},"status":{"privacyStatus":"private","license":"youtube"}});
        let remote = Arc::new(Mutex::new((original.clone(), 0usize)));
        async fn list(State(r): State<Arc<Mutex<(Value, usize)>>>) -> Json<Value> {
            Json(json!({"items":[r.lock().await.0.clone()]}))
        }
        async fn update(
            State(r): State<Arc<Mutex<(Value, usize)>>>,
            headers: HeaderMap,
            Json(v): Json<Value>,
        ) -> Json<Value> {
            assert_eq!(headers["if-match"], "etag-1");
            assert!(v.get("status").is_none());
            assert_eq!(v["snippet"]["description"], "retained");
            assert_eq!(v["snippet"]["tags"], json!(["keep"]));
            let mut r = r.lock().await;
            r.1 += 1;
            r.0["snippet"] = v["snippet"].clone();
            Json(v)
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/api/videos", get(list).put(update))
            .with_state(remote.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let s = Arc::new(state(temp.path(), base));
        s.db.put("yt-video", "v", &original).unwrap();
        let request = || {
            serde_json::from_value::<Edit>(json!({"ids":["v"],"find":"old","replace":"new"}))
                .unwrap()
        };
        let b = preview(State(s.clone()), Json(request()))
            .await
            .ok()
            .unwrap()
            .0;
        remote.lock().await.0["snippet"]["title"] = json!("changed elsewhere");
        let failed = apply(State(s.clone()), Path(b.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(failed.changes[0].status, "failed");
        assert_eq!(remote.lock().await.1, 0);
        remote.lock().await.0 = original;
        let completed = apply(State(s.clone()), Path(b.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(completed.changes[0].status, "completed");
        assert_eq!(remote.lock().await.1, 1);
        let _ = apply(State(s), Path(b.id)).await.ok().unwrap();
        assert_eq!(
            remote.lock().await.1,
            1,
            "completed items must not be sent twice"
        );
        server.abort();
    }
}
