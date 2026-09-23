//! Persisted, reviewable metadata patches. Graphs are documents, never executable code.
use crate::{db::Db, model::*, web::ApiError, youtube, AppState};
use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

macro_rules! bail {
    ($($arg:tt)*) => { return Err(anyhow::anyhow!($($arg)*).into()) };
}

type HttpResult<T> = std::result::Result<Json<T>, ApiError>;
fn private() -> String {
    "private".into()
}
fn category() -> String {
    "22".into()
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Fields {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "category")]
    pub category_id: String,
    #[serde(default = "private")]
    pub privacy: String,
}
impl Fields {
    fn video(v: &Value) -> Result<Self> {
        Ok(Self {
            title: v["snippet"]["title"]
                .as_str()
                .context("视频标题缺失")?
                .into(),
            description: v["snippet"]["description"].as_str().unwrap_or("").into(),
            tags: serde_json::from_value(v["snippet"].get("tags").cloned().unwrap_or(json!([])))?,
            category_id: v["snippet"]["categoryId"].as_str().unwrap_or("22").into(),
            privacy: v["status"]["privacyStatus"]
                .as_str()
                .unwrap_or("private")
                .into(),
        })
    }
    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty()
            || self.title.chars().count() > 100
            || self.title.contains(['<', '>'])
        {
            bail!("标题应为 1–100 字且不含尖括号");
        }
        if self.description.len() > 5000 || self.description.contains(['<', '>']) {
            bail!("描述超过 5000 字节或包含尖括号");
        }
        if self
            .tags
            .iter()
            .any(|t| t.trim().is_empty() || t.contains(['<', '>']))
            || self
                .tags
                .iter()
                .map(|t| t.chars().count() + if t.contains(' ') { 2 } else { 0 })
                .sum::<usize>()
                + self.tags.len().saturating_sub(1)
                > 500
        {
            bail!("标签为空、含尖括号或总长度超过 500");
        }
        if !["private", "unlisted", "public"].contains(&self.privacy.as_str()) {
            bail!("无效公开状态");
        }
        if self.category_id.is_empty() || !self.category_id.chars().all(|c| c.is_ascii_digit()) {
            bail!("无效 YouTube 分类 ID");
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistIntent {
    action: String,
    #[serde(default)]
    playlist_id: Option<String>,
    #[serde(default)]
    identity_key: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct ThumbnailCandidate {
    asset_id: String,
    time_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data_url: Option<String>,
    #[serde(default)]
    image_id: String,
    #[serde(default)]
    modified_ms: u64,
    #[serde(default)]
    remote_before: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct Item {
    id: String,
    kind: String,
    before: Fields,
    after: Fields,
    #[serde(default = "object")]
    metadata: Value,
    #[serde(default)]
    playlist: Option<PlaylistIntent>,
    #[serde(default)]
    thumbnail: Option<ThumbnailCandidate>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    actions: BTreeMap<String, String>,
    #[serde(default)]
    playlist_before: Option<Value>,
    #[serde(default)]
    playlist_auto: bool,
    #[serde(default)]
    metadata_before: Option<Value>,
}
fn object() -> Value {
    json!({})
}
#[derive(Clone, Serialize, Deserialize)]
struct Run {
    id: String,
    name: String,
    graph: Value,
    channel_id: String,
    created_at: String,
    updated_at: String,
    status: String,
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Preview {
    name: String,
    graph: Value,
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Draft {
    #[serde(default)]
    id: Option<String>,
    name: String,
    graph: Value,
}

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/workflows", get(snapshot))
        .route("/api/workflows/drafts", post(save_draft))
        .route("/api/workflows/preview", post(preview))
        .route("/api/workflows/runs/{id}/apply", post(apply))
        .route("/api/workflows/playlists/sync", post(sync_playlists))
        .route("/api/workflows/thumbnails/{id}", get(thumbnail_image))
        .route(
            "/api/workflows/modules",
            get(list_modules).post(save_modules),
        )
        .route("/api/workflows/modules/install", post(install_module))
        .route("/api/workflows/modules/{id}/enable", post(enable_module))
        .route(
            "/api/workflows/modules/{id}/uninstall",
            post(uninstall_module),
        )
        .route(
            "/api/workflows/identity-ledger",
            get(list_identity).post(save_identity),
        )
}

pub(crate) fn recover(db: &Db) -> Result<()> {
    for mut run in db.list::<Run>("workflow-run")? {
        if run.status == "running" {
            run.status = "interrupted".into();
            run.updated_at = now();
            for item in &mut run.items {
                if item.status == "running" {
                    item.status = "interrupted".into();
                    item.message = "服务中断；继续时先核对远端，已完成动作不重放".into();
                }
            }
            db.put("workflow-run", &run.id, &run)?;
        }
    }
    Ok(())
}
async fn snapshot(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let channel = if s.config.headless {
        String::new()
    } else {
        youtube::workflow_channel(&s).unwrap_or_default()
    };
    Ok(Json(json!({
        "drafts": s.db.list::<Value>("workflow-draft")?,
        "runs": s.db.list::<Run>("workflow-run")?,
        "metadata": s.db.list::<Value>("workflow-metadata")?,
        "playlists": s.db.get::<Value>("workflow-playlists", &channel)?,
        "modules": module_state(&s.db)?,
        "identityLedger": s.db.get::<Value>("workflow-identity-ledger", "default")?.unwrap_or_else(|| json!({"items":[]})),
        "knownIntentKinds": KNOWN_INTENT_KINDS,
    })))
}

const KNOWN_INTENT_KINDS: &[&str] = &[
    "metadata.patch",
    "playlist.add",
    "playlist.create",
    "playlist.review",
    "playlist.updateSnippet",
    "thumbnail.setFromLocalFrame",
    "local.metadata.save",
    "identity.upsert",
    "agent.task",
    "publish.settings",
];

fn module_state(db: &Db) -> Result<Value> {
    Ok(db
        .get::<Value>("workflow-modules", "registry")?
        .unwrap_or_else(|| json!({"modules":[],"disabledBuiltinIds":[]})))
}

#[derive(Deserialize)]
struct ModuleStateBody {
    modules: Value,
    #[serde(default, rename = "disabledBuiltinIds")]
    disabled_builtin_ids: Value,
}

async fn list_modules(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    Ok(Json(module_state(&s.db)?))
}

async fn save_modules(
    State(s): State<Arc<AppState>>,
    Json(body): Json<ModuleStateBody>,
) -> HttpResult<Value> {
    let modules = body.modules.as_array().context("modules 必须为数组")?;
    if modules.len() > 200 {
        bail!("本地模块过多");
    }
    for module in modules {
        let id = module["id"].as_str().unwrap_or("");
        let kind = module["kind"].as_str().unwrap_or("");
        let api = module["apiVersion"].as_u64().unwrap_or(0);
        if id.is_empty() || id.len() > 160 || kind != "declarative" || api != 1 {
            bail!("仅接受 apiVersion=1 的 declarative 本地模块");
        }
        if id.starts_with("com.u2bup.builtin.") {
            bail!("不能覆盖内置模块");
        }
    }
    let state = json!({
        "modules": body.modules,
        "disabledBuiltinIds": body.disabled_builtin_ids,
    });
    s.db.put("workflow-modules", "registry", &state)?;
    Ok(Json(state))
}

#[derive(Deserialize)]
struct InstallBody {
    package: Value,
    #[serde(default)]
    trusted: bool,
}

async fn install_module(
    State(s): State<Arc<AppState>>,
    Json(body): Json<InstallBody>,
) -> HttpResult<Value> {
    let package = &body.package;
    if package["apiVersion"] != 1 || package["kind"] != "declarative" {
        bail!("仅支持 apiVersion=1 的声明式模块包");
    }
    let id = package["id"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 160)
        .context("无效模块 ID")?
        .to_string();
    if id.starts_with("com.u2bup.builtin.") {
        bail!("不能安装与内置模块冲突的 ID");
    }
    if package.get("manifest").is_none() || package.get("rules").is_none() {
        bail!("模块包需要 manifest 与 rules");
    }
    let mut state = module_state(&s.db)?;
    let mut modules = state["modules"].as_array().cloned().unwrap_or_default();
    modules.retain(|m| m["id"] != id);
    let installed = json!({
        "id": id,
        "version": package["version"].as_str().unwrap_or("1.0.0"),
        "apiVersion": 1,
        "kind": "declarative",
        "alias": package["manifest"]["alias"].as_str().unwrap_or(&id),
        "enabled": true,
        "source": "local",
        "trusted": body.trusted,
        "installedAt": now(),
        "manifest": package["manifest"],
        "rules": package["rules"],
    });
    modules.push(installed.clone());
    state["modules"] = Value::Array(modules);
    s.db.put("workflow-modules", "registry", &state)?;
    Ok(Json(json!({"installed": installed, "state": state})))
}

#[derive(Deserialize)]
struct EnableBody {
    enabled: bool,
    #[serde(default)]
    builtin: bool,
}

async fn enable_module(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<EnableBody>,
) -> HttpResult<Value> {
    if id.is_empty() || id.len() > 160 {
        bail!("无效模块 ID");
    }
    let mut state = module_state(&s.db)?;
    if body.builtin || id.starts_with("com.u2bup.builtin.") {
        let mut disabled = state["disabledBuiltinIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        disabled.retain(|v| v.as_str() != Some(id.as_str()));
        if !body.enabled {
            disabled.push(json!(id));
        }
        state["disabledBuiltinIds"] = Value::Array(disabled);
    } else {
        let modules = state["modules"].as_array_mut().context("模块注册表损坏")?;
        let module = modules
            .iter_mut()
            .find(|m| m["id"] == id)
            .context("未找到本地模块")?;
        module["enabled"] = json!(body.enabled);
    }
    s.db.put("workflow-modules", "registry", &state)?;
    Ok(Json(state))
}

async fn uninstall_module(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HttpResult<Value> {
    if id.starts_with("com.u2bup.builtin.") {
        bail!("不能卸载内置模块");
    }
    let mut state = module_state(&s.db)?;
    let modules = state["modules"].as_array_mut().context("模块注册表损坏")?;
    let before = modules.len();
    modules.retain(|m| m["id"] != id);
    if modules.len() == before {
        bail!("未找到本地模块");
    }
    s.db.put("workflow-modules", "registry", &state)?;
    Ok(Json(state))
}

#[derive(Deserialize)]
struct IdentityLedgerBody {
    items: Vec<Value>,
}

async fn list_identity(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    Ok(Json(
        s.db.get::<Value>("workflow-identity-ledger", "default")?
            .unwrap_or_else(|| json!({"items":[]})),
    ))
}

async fn save_identity(
    State(s): State<Arc<AppState>>,
    Json(body): Json<IdentityLedgerBody>,
) -> HttpResult<Value> {
    if body.items.len() > 5000 {
        bail!("身份台账条目过多");
    }
    for item in &body.items {
        let platform = item["platform"].as_str().unwrap_or("");
        let room = item["roomId"].as_str().unwrap_or("");
        let creator = item["creator"].as_str().unwrap_or("");
        if platform.is_empty()
            || room.is_empty()
            || creator.is_empty()
            || platform.len() > 40
            || room.len() > 80
            || creator.chars().count() > 120
        {
            bail!("身份台账需要有效的 platform、roomId、creator");
        }
    }
    let saved = json!({"items": body.items, "updatedAt": now()});
    s.db.put("workflow-identity-ledger", "default", &saved)?;
    Ok(Json(saved))
}
fn validate_graph(name: &str, graph: &Value) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 160 {
        bail!("管线名称应为 1–160 字");
    }
    if !graph.is_object()
        || graph["version"] != 1
        || graph["nodes"].as_array().is_none_or(|v| v.len() > 100)
        || graph["edges"].as_array().is_none_or(|v| v.len() > 300)
    {
        bail!("无效管线图；只接受 version=1、最多 100 个节点和 300 条连线");
    }
    Ok(())
}
async fn save_draft(State(s): State<Arc<AppState>>, Json(d): Json<Draft>) -> HttpResult<Value> {
    validate_graph(&d.name, &d.graph)?;
    let id = d.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if id.is_empty() || id.len() > 160 {
        bail!("无效管线 ID");
    }
    let v = json!({"id":id,"name":d.name,"graph":d.graph,"updated_at":now()});
    s.db.put("workflow-draft", &id, &v)?;
    Ok(Json(v))
}

fn metadata_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}
async fn local_fields(s: &AppState, id: &str) -> Result<Fields> {
    let library = s.library.read().await;
    let asset = library
        .assets
        .iter()
        .find(|a| a.id == id)
        .context("素材已不存在")?;
    let mut fields = if let Some(saved) =
        s.db.get::<Value>("workflow-metadata", &metadata_key("local", id))?
    {
        serde_json::from_value(saved["fields"].clone())?
    } else {
        Fields {
            title: String::new(),
            description: String::new(),
            tags: vec![],
            category_id: category(),
            privacy: private(),
        }
    };
    fields.title = asset.display_title.as_ref().unwrap_or(&asset.title).clone();
    Ok(fields)
}
// Compare only fields that the user approved changing, then preserve fresh untouched fields.
fn patched_fields(
    current: &Fields,
    before: &Fields,
    after: &Fields,
    allow_applied: bool,
) -> Result<Fields> {
    let mut result = serde_json::to_value(current)?;
    let base = serde_json::to_value(before)?;
    let target = serde_json::to_value(after)?;
    for field in ["title", "description", "tags", "categoryId", "privacy"] {
        if base[field] != target[field] {
            if result[field] != base[field] && !(allow_applied && result[field] == target[field]) {
                bail!("{field} 已在预览后变化，请重新预览");
            }
            result[field] = target[field].clone();
        }
    }
    Ok(serde_json::from_value(result)?)
}
fn managed_description(original: &str, addition: &str, identity: &str) -> Result<String> {
    let start = "[U2BUP 管理信息]";
    let end = "[/U2BUP 管理信息]";
    let mut retained = original.to_owned();
    if let Some(a) = retained.find(start) {
        let b = retained[a..]
            .find(end)
            .context("播放列表管理信息区块不完整，请人工检查")?
            + a
            + end.len();
        retained.replace_range(a..b, "");
    }
    let result = format!(
        "{}\n\n{start}\n{}\nU2BUP identity: {identity}\n{end}",
        retained.trim(),
        addition.trim()
    )
    .trim()
    .to_owned();
    if result.len() > 5000 {
        bail!("播放列表说明合并后超过 5000 字节");
    }
    Ok(result)
}
fn playlist_identity(item: &Item) -> Result<String> {
    let platform = item.metadata["platform"].as_str().unwrap_or("");
    let room = item.metadata["roomId"].as_str().unwrap_or("");
    let valid_room = match platform.to_ascii_lowercase().as_str() {
        "bilibili" => room.chars().all(|c| c.is_ascii_digit()),
        "twitch" => room.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    };
    if room.is_empty() || !valid_room {
        bail!("自动播放列表绑定需要明确来源和完整房间号；否则请选定播放列表 ID");
    }
    Ok(format!("{platform}:{room}"))
}
fn matches_identity(playlist: &Value, identity: &str, room: &str) -> bool {
    let description = playlist["snippet"]["description"].as_str().unwrap_or("");
    let markers: Vec<_> = description
        .lines()
        .filter_map(|l| l.strip_prefix("U2BUP identity: "))
        .collect();
    if !markers.is_empty() {
        return markers.len() == 1 && markers[0] == identity;
    }
    let text = format!(
        "{}\n{}",
        playlist["snippet"]["title"].as_str().unwrap_or(""),
        description
    );
    let lower = text.to_lowercase();
    let platform = identity
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let platform_matches = match platform.as_str() {
        "bilibili" => {
            lower.contains("bilibili") || lower.contains("哔哩哔哩") || lower.contains("b站")
        }
        "twitch" => lower.contains("twitch"),
        _ => false,
    };
    platform_matches
        && lower
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|part| !part.is_empty() && part == room.to_lowercase())
}
fn bind_playlist(item: &mut Item, cache: &Value, channel: &str) -> Result<()> {
    let intent = item.playlist.as_ref().unwrap();
    if !["add", "create"].contains(&intent.action.as_str()) {
        bail!("播放列表待人工确认，不能直接执行");
    }
    if intent.title.trim().is_empty()
        || intent.title.chars().count() > 150
        || intent.title.contains(['<', '>'])
        || intent.description.contains(['<', '>'])
    {
        bail!("播放列表名称或说明不合法");
    }
    let playlists = cache["items"]
        .as_array()
        .context("请先同步播放列表后重新预览")?;
    if cache["channel_id"] != channel {
        bail!("播放列表缓存属于其他频道，请重新同步");
    }
    let matches: Vec<_> = if let Some(id) = &intent.playlist_id {
        playlists.iter().filter(|p| p["id"] == *id).collect()
    } else {
        let identity = playlist_identity(item)?;
        if intent.identity_key != identity {
            bail!("播放列表身份与素材来源不一致");
        }
        let room = item.metadata["roomId"].as_str().unwrap();
        playlists
            .iter()
            .filter(|p| matches_identity(p, &identity, room))
            .collect()
    };
    match matches.as_slice() {
        [found] => {
            if found["snippet"]["channelId"] != channel {
                bail!("播放列表不属于当前频道");
            }
            item.playlist_before = Some((*found).clone());
            let intent = item.playlist.as_mut().unwrap();
            intent.playlist_id = Some(found["id"].as_str().context("播放列表 ID 缺失")?.into());
            intent.description = managed_description(
                found["snippet"]["description"].as_str().unwrap_or(""),
                &intent.description,
                &intent.identity_key,
            )?;
        }
        [] if intent.playlist_id.is_none() && intent.action == "create" => {
            playlist_identity(item)?;
            let intent = item.playlist.as_mut().unwrap();
            intent.description =
                managed_description("", &intent.description, &intent.identity_key)?;
        }
        [] => bail!("找不到唯一对应的播放列表；默认不会创建"),
        _ => bail!("同一房间存在多个播放列表，请明确选择 ID"),
    }
    Ok(())
}
async fn preview(State(s): State<Arc<AppState>>, Json(mut p): Json<Preview>) -> HttpResult<Run> {
    validate_graph(&p.name, &p.graph)?;
    if p.items.is_empty() || p.items.len() > 100 {
        bail!("每次预览选择 1–100 条记录");
    }
    let channel = if p.items.iter().any(|i| i.kind == "youtube") {
        youtube::workflow_channel(&s)?
    } else {
        String::new()
    };
    let cache =
        s.db.get::<Value>("workflow-playlists", &channel)?
            .unwrap_or(Value::Null);
    let mut seen = HashSet::new();
    for item in &mut p.items {
        if !["youtube", "local"].contains(&item.kind.as_str())
            || item.id.is_empty()
            || item.id.len() > 160
            || !seen.insert(metadata_key(&item.kind, &item.id))
        {
            bail!("记录类型、ID 无效或重复");
        }
        if !item.metadata.is_object() || serde_json::to_vec(&item.metadata)?.len() > 64 * 1024 {
            bail!("本地分类元信息无效或过大");
        }
        item.after.validate()?;
        item.actions.clear();
        item.playlist_before = None;
        item.playlist_auto = item
            .playlist
            .as_ref()
            .is_some_and(|p| p.playlist_id.is_none());
        item.metadata_before =
            s.db.get::<Value>("workflow-metadata", &metadata_key(&item.kind, &item.id))?;
        let current = if item.kind == "youtube" {
            let video =
                s.db.get::<Value>("yt-video", &item.id)?
                    .context("请先同步频道视频")?;
            if video["snippet"]["channelId"] != channel {
                bail!("视频不属于当前频道");
            }
            if item.before.privacy != item.after.privacy
                && video["status"].get("publishAt").is_some()
            {
                bail!("定时发布视频请先在 Studio 处理排期");
            }
            Fields::video(&video)?
        } else {
            local_fields(&s, &item.id).await?
        };
        patched_fields(&current, &item.before, &item.after, false)?;
        if item.kind == "youtube" && item.playlist.is_some() {
            bind_playlist(item, &cache, &channel)?;
        }
        if let Some(candidate) = &mut item.thumbnail {
            if !candidate.time_seconds.is_finite() || candidate.time_seconds < 0.0 {
                bail!("封面时间无效");
            }
            let library = s.library.read().await;
            let asset = library
                .assets
                .iter()
                .find(|a| a.id == candidate.asset_id)
                .context("封面素材不存在")?;
            if item.kind == "youtube" && item.metadata["localAssetId"] != candidate.asset_id {
                bail!("上传封面前需要明确绑定对应本地素材 localAssetId");
            }
            if item.kind == "local" && item.id != candidate.asset_id {
                bail!("本地封面必须来自当前素材");
            }
            if asset
                .metadata
                .as_ref()
                .and_then(|m| m.duration)
                .is_some_and(|d| candidate.time_seconds >= d)
            {
                bail!("所选帧超过素材时长");
            }
            let path = crate::media::resolve_input(&s.config.library, &asset.relative_path)?;
            candidate.modified_ms = crate::media::modified_ms(&std::fs::metadata(path)?);
            let url = candidate
                .data_url
                .take()
                .context("请选择当前帧并提供 JPEG data_url")?;
            let bytes = STANDARD.decode(
                url.strip_prefix("data:image/jpeg;base64,")
                    .context("封面必须是 JPEG data URL")?,
            )?;
            validate_jpeg(&bytes)?;
            candidate.image_id = digest(&bytes);
            let dir = s.config.data.join("workflow-thumbnails");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(format!("{}.jpg", candidate.image_id)), bytes)?;
            candidate.remote_before = if item.kind == "youtube" {
                s.db.get::<Value>("yt-video", &item.id)?
                    .context("视频缓存缺失")?["snippet"]["thumbnails"]
                    .clone()
            } else {
                Value::Null
            };
        }
        item.status = "pending".into();
        item.message.clear();
    }
    let run = Run {
        id: uuid::Uuid::new_v4().to_string(),
        name: p.name,
        graph: p.graph,
        channel_id: channel,
        created_at: now(),
        updated_at: now(),
        status: "preview".into(),
        items: p.items,
    };
    s.db.put("workflow-run", &run.id, &run)?;
    Ok(Json(run))
}

async fn all_pages(s: &AppState, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
    let mut token = String::new();
    let mut tokens = HashSet::new();
    let mut ids = HashSet::new();
    let mut items = Vec::new();
    loop {
        if !tokens.insert(token.clone()) {
            bail!("YouTube 返回重复分页标记，保留上次完整缓存");
        }
        let mut q = query.to_vec();
        q.extend([("maxResults", "50"), ("pageToken", token.as_str())]);
        let response = youtube::api_get(s, path, &q).await?;
        for item in response["items"].as_array().context("无效播放列表响应")? {
            if ids.insert(item["id"].as_str().context("播放列表项缺少 ID")?.to_owned()) {
                items.push(item.clone());
            }
        }
        token = response["nextPageToken"].as_str().unwrap_or("").into();
        if token.is_empty() {
            break;
        }
    }
    Ok(items)
}
async fn playlist_list(s: &AppState) -> Result<Vec<Value>> {
    youtube::workflow_channel(s)?;
    all_pages(
        s,
        "playlists",
        &[("part", "snippet,status,contentDetails"), ("mine", "true")],
    )
    .await
}
async fn member_ids(s: &AppState, id: &str) -> Result<Vec<String>> {
    Ok(all_pages(
        s,
        "playlistItems",
        &[("part", "contentDetails"), ("playlistId", id)],
    )
    .await?
    .iter()
    .filter_map(|v| v["contentDetails"]["videoId"].as_str().map(str::to_owned))
    .collect())
}
async fn sync_playlists(State(s): State<Arc<AppState>>) -> HttpResult<Value> {
    let channel = youtube::workflow_channel(&s)?;
    let _op = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("YouTube 操作进行中"))?;
    let mut items = playlist_list(&s).await?;
    for item in &mut items {
        if item["snippet"]["channelId"] != channel {
            bail!("同步收到其他频道播放列表");
        }
        item["video_ids"] =
            json!(member_ids(&s, item["id"].as_str().context("播放列表 ID 缺失")?).await?);
    }
    let cache = json!({"channel_id":channel,"synced_at":now(),"items":items});
    s.db.put("workflow-playlists", &channel, &cache)?;
    Ok(Json(cache))
}

fn checkpoint(s: &AppState, run: &mut Run, index: usize, action: &str, status: &str) -> Result<()> {
    run.items[index]
        .actions
        .insert(action.into(), status.into());
    run.updated_at = now();
    s.db.put("workflow-run", &run.id, run)
}
fn done(run: &Run, index: usize, action: &str) -> bool {
    run.items[index]
        .actions
        .get(action)
        .is_some_and(|s| s == "completed")
}
async fn apply_video(s: &AppState, run: &mut Run, index: usize) -> Result<()> {
    if done(run, index, "video") {
        return Ok(());
    }
    let item = &run.items[index];
    let response =
        youtube::api_get(s, "videos", &[("part", "snippet,status"), ("id", &item.id)]).await?;
    let current = response["items"]
        .as_array()
        .and_then(|v| v.first())
        .context("远端视频已不存在")?;
    if current["snippet"]["channelId"] != run.channel_id {
        bail!("远端视频不属于预览频道");
    }
    let fields = Fields::video(current)?;
    let patched = patched_fields(
        &fields,
        &item.before,
        &item.after,
        item.actions.contains_key("video"),
    )?;
    let mut saved =
        s.db.get::<Value>("yt-video", &item.id)?
            .unwrap_or_else(|| current.clone());
    saved["snippet"] = current["snippet"].clone();
    saved["status"] = current["status"].clone();
    if let Some(etag) = current.get("etag") {
        saved["etag"] = etag.clone();
    }
    if fields != patched {
        let mut body = json!({"id":item.id});
        let mut parts = Vec::new();
        if fields.title != patched.title
            || fields.description != patched.description
            || fields.tags != patched.tags
            || fields.category_id != patched.category_id
        {
            let mut snippet = youtube::writable(current, "snippet");
            snippet["title"] = json!(patched.title);
            snippet["description"] = json!(patched.description);
            snippet["tags"] = json!(patched.tags);
            snippet["categoryId"] = json!(patched.category_id);
            body["snippet"] = snippet;
            parts.push("snippet");
        }
        if fields.privacy != patched.privacy {
            if current["status"].get("publishAt").is_some() {
                bail!("视频已加入定时发布，请重新检查排期");
            }
            let mut status = youtube::writable(current, "status");
            status["privacyStatus"] = json!(patched.privacy);
            body["status"] = status;
            parts.push("status");
        }
        let etag = current["etag"]
            .as_str()
            .context("远端缺少 ETag，不能安全更新")?;
        let id = item.id.clone();
        checkpoint(s, run, index, "video", "running")?;
        let updated = youtube::workflow_write(
            s,
            reqwest::Method::PUT,
            "videos",
            &parts.join(","),
            &body,
            Some(etag),
        )
        .await?;
        for part in parts {
            saved[part] = updated
                .get(part)
                .cloned()
                .unwrap_or_else(|| body[part].clone());
        }
        if let Some(etag) = updated.get("etag") {
            saved["etag"] = etag.clone();
        }
        s.db.put("yt-video", &id, &saved)?;
    }
    s.db.put("yt-video", &run.items[index].id, &saved)?;
    checkpoint(s, run, index, "video", "completed")
}
async fn apply_metadata(s: &AppState, run: &mut Run, index: usize) -> Result<()> {
    if done(run, index, "metadata") {
        return Ok(());
    }
    let item = &run.items[index];
    let key = metadata_key(&item.kind, &item.id);
    let existing = s.db.get::<Value>("workflow-metadata", &key)?;
    if existing != item.metadata_before && existing.as_ref().is_none_or(|v| v["run_id"] != run.id) {
        bail!("本地分类元信息已在预览后变化，请重新预览");
    }
    let mut fields = item.after.clone();
    if item.kind == "youtube" {
        fields = Fields::video(
            &s.db
                .get::<Value>("yt-video", &item.id)?
                .context("视频缓存缺失")?,
        )?;
    }
    if item.kind == "local" {
        let current = local_fields(s, &item.id).await?;
        fields = patched_fields(
            &current,
            &item.before,
            &item.after,
            item.actions.contains_key("metadata"),
        )?;
        checkpoint(s, run, index, "metadata", "running")?;
        let mut library = s.library.write().await;
        library
            .assets
            .iter_mut()
            .find(|a| a.id == run.items[index].id)
            .context("素材已不存在")?
            .display_title = Some(fields.title.clone());
        s.db.put("library", "main", &*library)?;
    }
    let item = &run.items[index];
    let saved = json!({"id":item.id,"kind":item.kind,"fields":fields,"metadata":item.metadata,
        "playlist":item.playlist,"thumbnail":item.thumbnail,"updated_at":now(),"run_id":run.id});
    s.db.put("workflow-metadata", &key, &saved)?;
    checkpoint(s, run, index, "metadata", "completed")
}

async fn apply_playlist(s: &AppState, run: &mut Run, index: usize) -> Result<()> {
    if done(run, index, "playlist") || run.items[index].playlist.is_none() {
        return Ok(());
    }
    let intent = run.items[index].playlist.clone().unwrap();
    let mut id = intent.playlist_id.clone();
    if id.is_none() {
        let key = format!("{}:{}", run.channel_id, intent.identity_key);
        let receipt = s.db.get::<Value>("workflow-playlist-create", &key)?;
        let marker = format!("U2BUP identity: {}", intent.identity_key);
        let all = playlist_list(s).await?;
        let found: Vec<_> = all
            .iter()
            .filter(|p| {
                p["snippet"]["description"]
                    .as_str()
                    .unwrap_or("")
                    .lines()
                    .any(|l| l == marker)
            })
            .collect();
        if found.len() > 1 {
            bail!("播放列表身份对应多个结果，请同步并明确绑定 ID");
        }
        if let Some(existing) = found.first() {
            id = existing["id"].as_str().map(str::to_owned);
        } else if receipt.as_ref().is_some_and(|r| r["status"] != "failed") {
            bail!("曾发出创建请求但尚未找到确认结果；不会盲目重建，请在 Studio 核对后绑定 ID");
        } else {
            if intent.action != "create" {
                bail!("此预览未授权新建播放列表");
            }
            let room = run.items[index].metadata["roomId"].as_str().unwrap_or("");
            if all
                .iter()
                .any(|p| matches_identity(p, &intent.identity_key, room))
            {
                bail!("预览后出现对应房间的播放列表，请重新同步预览，避免重复创建");
            }
            s.db.put(
                "workflow-playlist-create",
                &key,
                &json!({"status":"unknown","run_id":run.id,"created_at":now()}),
            )?;
            checkpoint(s, run, index, "playlist_create", "running")?;
            let created = youtube::workflow_write(s, reqwest::Method::POST, "playlists", "snippet,status", &json!({
                "snippet":{"title":intent.title,"description":intent.description},"status":{"privacyStatus":"private"}
            }), None).await;
            let created = match created {
                Ok(value) => value,
                Err(error) => {
                    if youtube::workflow_definitive_failure(&error) {
                        s.db.put(
                            "workflow-playlist-create",
                            &key,
                            &json!({"status":"failed","run_id":run.id}),
                        )?;
                        checkpoint(s, run, index, "playlist_create", "failed")?;
                    }
                    return Err(error);
                }
            };
            id = Some(
                created["id"]
                    .as_str()
                    .context("创建响应缺少播放列表 ID，请同步后核对")?
                    .into(),
            );
        }
        let playlist_id = id.as_deref().context("播放列表 ID 缺失")?;
        s.db.put(
            "workflow-playlist-create",
            &key,
            &json!({"status":"completed","playlist_id":playlist_id,"run_id":run.id}),
        )?;
        run.items[index].playlist.as_mut().unwrap().playlist_id = id.clone();
        checkpoint(s, run, index, "playlist_create", "completed")?;
    }
    let id = id.context("播放列表 ID 缺失")?;
    if run.items[index].playlist_auto && run.items[index].playlist_before.is_some() {
        let room = run.items[index].metadata["roomId"].as_str().unwrap_or("");
        let all = playlist_list(s).await?;
        let matches: Vec<_> = all
            .iter()
            .filter(|p| matches_identity(p, &intent.identity_key, room))
            .collect();
        if matches.len() != 1 || matches[0]["id"] != id {
            bail!("播放列表房间绑定已变化或出现多个候选，请重新同步并明确 ID");
        }
    }
    let response =
        youtube::api_get(s, "playlists", &[("part", "snippet,status"), ("id", &id)]).await?;
    let current = response["items"]
        .as_array()
        .and_then(|v| v.first())
        .context("播放列表已删除，请重新预览")?;
    if current["snippet"]["channelId"] != run.channel_id {
        bail!("播放列表不属于当前频道");
    }
    if !done(run, index, "playlist_rename") {
        let already = current["snippet"]["title"] == intent.title
            && current["snippet"]["description"] == intent.description;
        if let Some(before) = &run.items[index].playlist_before {
            for field in ["title", "description"] {
                if current["snippet"][field] != before["snippet"][field] && !already {
                    bail!("播放列表已在预览后变化，请重新预览");
                }
            }
        } else if !already {
            bail!("发现的播放列表内容与新建预览不同，请重新同步预览以保留人工说明");
        }
        if !already {
            let mut snippet = json!({"title":intent.title,"description":intent.description});
            if let Some(language) = current["snippet"].get("defaultLanguage") {
                snippet["defaultLanguage"] = language.clone();
            }
            let etag = current["etag"]
                .as_str()
                .context("播放列表缺少 ETag，不能安全更新")?;
            checkpoint(s, run, index, "playlist_rename", "running")?;
            youtube::workflow_write(
                s,
                reqwest::Method::PUT,
                "playlists",
                "snippet",
                &json!({"id":id,"snippet":snippet}),
                Some(etag),
            )
            .await?;
        }
        checkpoint(s, run, index, "playlist_rename", "completed")?;
    }
    if !done(run, index, "playlist_add") {
        let video_id = run.items[index].id.clone();
        let members = member_ids(s, &id).await?;
        if !members.contains(&video_id) {
            if run.items[index]
                .actions
                .get("playlist_add")
                .is_some_and(|v| v == "running")
            {
                bail!("上次添加响应未知且尚未查到成员；不会重复插入，请同步并在 Studio 核对");
            }
            checkpoint(s, run, index, "playlist_add", "running")?;
            let result = youtube::workflow_write(
                s,
                reqwest::Method::POST,
                "playlistItems",
                "snippet",
                &json!({"snippet":{
                    "playlistId":id,"resourceId":{"kind":"youtube#video","videoId":video_id}
                }}),
                None,
            )
            .await;
            if let Err(error) = result {
                if youtube::workflow_definitive_failure(&error) {
                    checkpoint(s, run, index, "playlist_add", "failed")?;
                }
                return Err(error);
            }
        }
        checkpoint(s, run, index, "playlist_add", "completed")?;
    }
    checkpoint(s, run, index, "playlist", "completed")
}

fn validate_jpeg(bytes: &[u8]) -> Result<()> {
    // The existing 2 MiB JSON boundary also bounds the base64 request.
    if bytes.len() < 4
        || bytes.len() > 1024 * 1024
        || !bytes.starts_with(&[0xff, 0xd8])
        || !bytes.ends_with(&[0xff, 0xd9])
    {
        bail!("封面必须是完整 JPEG，应用内大小上限为 1 MiB");
    }
    let mut offset = 2;
    let mut dimensions = false;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            bail!("JPEG 结构不完整");
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes.get(offset).context("JPEG 标记不完整")?;
        offset += 1;
        if marker == 0xda {
            if dimensions {
                return Ok(());
            }
            bail!("JPEG 缺少画面尺寸");
        }
        if matches!(marker, 0xd8 | 0xd9 | 0x00) {
            bail!("JPEG 标记无效");
        }
        let size = u16::from_be_bytes([
            *bytes.get(offset).context("JPEG 段缺失")?,
            *bytes.get(offset + 1).context("JPEG 段缺失")?,
        ]) as usize;
        if size < 2 || offset + size > bytes.len() {
            bail!("JPEG 数据被截断");
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if size < 8
                || bytes[offset + 3..offset + 5] == [0, 0]
                || bytes[offset + 5..offset + 7] == [0, 0]
            {
                bail!("JPEG 尺寸无效");
            }
            dimensions = true;
        }
        offset += size;
    }
    bail!("JPEG 缺少画面数据");
}
async fn thumbnail_image(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> std::result::Result<Response, ApiError> {
    if id.len() != 20 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("无效封面 ID");
    }
    let bytes = tokio::fs::read(
        s.config
            .data
            .join("workflow-thumbnails")
            .join(format!("{id}.jpg")),
    )
    .await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response())
}
async fn apply_thumbnail(s: &AppState, run: &mut Run, index: usize) -> Result<()> {
    let Some(candidate) = run.items[index].thumbnail.clone() else {
        return Ok(());
    };
    if done(run, index, "thumbnail") {
        return Ok(());
    }
    if run.items[index].kind == "local" {
        return checkpoint(s, run, index, "thumbnail", "candidate_saved");
    }
    if run.items[index]
        .actions
        .get("thumbnail")
        .is_some_and(|v| v == "running")
    {
        bail!("上次封面上传响应未知，不能自动重放；请在 Studio 核对后重新预览");
    }
    let library = s.library.read().await;
    let asset = library
        .assets
        .iter()
        .find(|a| a.id == candidate.asset_id)
        .context("封面素材已不存在")?;
    let path = crate::media::resolve_input(&s.config.library, &asset.relative_path)?;
    if crate::media::modified_ms(&std::fs::metadata(path)?) != candidate.modified_ms {
        bail!("封面来源素材已变化，请重新选帧");
    }
    drop(library);
    let bytes = tokio::fs::read(
        s.config
            .data
            .join("workflow-thumbnails")
            .join(format!("{}.jpg", candidate.image_id)),
    )
    .await?;
    validate_jpeg(&bytes)?;
    if digest(&bytes) != candidate.image_id {
        bail!("封面文件已变化，请重新选帧");
    }
    let id = run.items[index].id.clone();
    let response =
        youtube::api_get(s, "videos", &[("part", "snippet,status"), ("id", &id)]).await?;
    let current = response["items"]
        .as_array()
        .and_then(|v| v.first())
        .context("远端视频已不存在")?;
    if current["snippet"]["channelId"] != run.channel_id {
        bail!("封面目标不属于当前频道");
    }
    if current["snippet"]["thumbnails"] != candidate.remote_before {
        bail!("远端封面在预览后变化，请重新检查");
    }
    checkpoint(s, run, index, "thumbnail", "running")?;
    let result = match youtube::workflow_thumbnail(s, &id, bytes).await {
        Ok(value) => value,
        Err(error) => {
            if youtube::workflow_definitive_failure(&error) {
                checkpoint(s, run, index, "thumbnail", "failed")?;
            }
            return Err(error);
        }
    };
    if let Some(thumbnails) = result["items"].as_array().and_then(|v| v.first()) {
        if let Some(mut saved) = s.db.get::<Value>("yt-video", &id)? {
            saved["snippet"]["thumbnails"] = thumbnails.clone();
            s.db.put("yt-video", &id, &saved)?;
        }
    }
    checkpoint(s, run, index, "thumbnail", "completed")
}

async fn apply(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> HttpResult<Run> {
    let _local = s
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("本地任务进行中，请稍后应用管线"))?;
    let _youtube = s
        .youtube
        .operation
        .try_lock()
        .map_err(|_| anyhow::anyhow!("YouTube 操作进行中"))?;
    let mut run =
        s.db.get::<Run>("workflow-run", &id)?
            .context("管线预览不存在")?;
    if run.items.iter().any(|i| i.kind == "youtube")
        && youtube::workflow_channel(&s)? != run.channel_id
    {
        bail!("当前频道与预览不同");
    }
    run.status = "running".into();
    s.db.put("workflow-run", &run.id, &run)?;
    for index in 0..run.items.len() {
        if run.items[index].status == "completed" {
            continue;
        }
        if s.shutdown.is_cancelled() {
            break;
        }
        run.items[index].status = "running".into();
        s.db.put("workflow-run", &run.id, &run)?;
        let result = async {
            if run.items[index].kind == "youtube" {
                apply_video(&s, &mut run, index).await?;
            }
            apply_metadata(&s, &mut run, index).await?;
            if run.items[index].kind == "youtube" {
                apply_playlist(&s, &mut run, index).await?;
            }
            apply_thumbnail(&s, &mut run, index).await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let item = &mut run.items[index];
        match result {
            Ok(()) => {
                item.status = "completed".into();
                item.message = if item.kind == "local" {
                    "已应用到素材元信息；封面保存在本地"
                } else {
                    "已应用"
                }
                .into();
            }
            Err(e) => {
                item.status = "failed".into();
                item.message = format!("{e:#}");
            }
        }
        run.updated_at = now();
        s.db.put("workflow-run", &run.id, &run)?;
    }
    run.status = if run.items.iter().all(|i| i.status == "completed") {
        "completed"
    } else {
        "partial"
    }
    .into();
    run.updated_at = now();
    s.db.put("workflow-run", &run.id, &run)?;
    Ok(Json(run))
}

// Upload callers opt in explicitly; mixed-source exports require identical prepared metadata.
pub(crate) fn prepared_upload(db: &Db, source_ids: &[String]) -> Result<Option<Fields>> {
    let mut prepared: Option<Fields> = None;
    for id in source_ids {
        let Some(saved) = db.get::<Value>("workflow-metadata", &metadata_key("local", id))? else {
            bail!("成品包含尚未应用管线元信息的素材，请先完成素材预处理");
        };
        let fields: Fields = serde_json::from_value(saved["fields"].clone())?;
        if prepared.as_ref().is_some_and(|v| *v != fields) {
            bail!("合并成品的素材元信息不同，请在上传前统一或关闭管线元信息选项");
        }
        prepared = Some(fields);
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Query,
        http::{HeaderMap, StatusCode},
    };
    use tokio::sync::Mutex;

    fn graph() -> Value {
        json!({"version":1,"nodes":[],"edges":[]})
    }
    fn fields(title: &str) -> Fields {
        serde_json::from_value(
            json!({"title":title,"description":"","tags":[],"categoryId":"22","privacy":"private"}),
        )
        .unwrap()
    }
    fn request(kind: &str, id: &str) -> Preview {
        serde_json::from_value(json!({"name":"测试管线","graph":graph(),"items":[{
            "id":id,"kind":kind,"before":fields("old"),"after":fields("new"),
            "metadata":{"platform":"Bilibili","roomId":"123","creator":"Creator"}
        }]}))
        .unwrap()
    }
    #[test]
    fn patch_preserves_fresh_untouched_fields_and_rejects_stale_changes() {
        let before = fields("old");
        let after = fields("new");
        let mut fresh = before.clone();
        fresh.description = "fresh human note".into();
        let patched = patched_fields(&fresh, &before, &after, false).unwrap();
        assert_eq!(patched.title, "new");
        assert_eq!(patched.description, "fresh human note");
        fresh.title = "someone else's title".into();
        assert!(patched_fields(&fresh, &before, &after, false).is_err());
        fresh.title = "new".into();
        assert!(patched_fields(&fresh, &before, &after, true).is_ok());
        let mut too_long = before;
        too_long.description = "界".repeat(1667);
        assert!(too_long.validate().is_err());
    }
    #[test]
    fn room_identity_is_exact_and_does_not_cross_platform_markers() {
        assert!(matches_identity(
            &json!({"snippet":{"title":"Bilibili Creator 123"}}),
            "Bilibili:123",
            "123"
        ));
        assert!(!matches_identity(
            &json!({"snippet":{"title":"Creator 1234"}}),
            "Bilibili:123",
            "123"
        ));
        assert!(!matches_identity(
            &json!({"snippet":{"title":"Twitch Creator 123"}}),
            "bilibili:123",
            "123"
        ));
        assert!(matches_identity(
            &json!({"snippet":{"description":"https://twitch.tv/example_live"}}),
            "twitch:example_live",
            "example_live"
        ));
        let mut twitch = request("youtube", "video").items.remove(0);
        twitch.metadata = json!({"platform":"twitch","roomId":"example_live"});
        assert_eq!(playlist_identity(&twitch).unwrap(), "twitch:example_live");
        assert!(!matches_identity(
            &json!({"snippet":{"title":"Creator 123","description":"U2BUP identity: Twitch:123"}}),
            "Bilibili:123",
            "123"
        ));
        assert!(!matches_identity(
            &json!({"snippet":{"description":"U2BUP identity: Bilibili:123\nU2BUP identity: Twitch:123"}}),
            "Bilibili:123",
            "123"
        ));
        let merged = managed_description("Human note", "房间号：123", "Bilibili:123").unwrap();
        assert!(merged.starts_with("Human note"));
        assert_eq!(
            managed_description(&merged, "房间号：123", "Bilibili:123").unwrap(),
            merged
        );
    }
    #[tokio::test]
    async fn local_run_is_durable_in_headless_and_upload_inherits_it() {
        let temp = tempfile::tempdir().unwrap();
        let mut raw = youtube::protocol_tests::state(temp.path(), String::new());
        raw.config.headless = true;
        raw.library = tokio::sync::RwLock::new(Library {
            assets: vec![Asset {
                id: "asset".into(),
                title: "old".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let s = Arc::new(raw);
        let run = preview(State(s.clone()), Json(request("local", "asset")))
            .await
            .ok()
            .unwrap()
            .0;
        let result = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(result.status, "completed");
        assert_eq!(
            s.library.read().await.assets[0].display_title.as_deref(),
            Some("new")
        );
        assert_eq!(
            prepared_upload(&s.db, &["asset".into()])
                .unwrap()
                .unwrap()
                .title,
            "new"
        );
        assert!(prepared_upload(&s.db, &["unknown".into()]).is_err());
        let repeated = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(repeated.items[0].actions["metadata"], "completed");
        assert_eq!(
            s.db.get::<Run>("workflow-run", &run.id)
                .unwrap()
                .unwrap()
                .status,
            "completed"
        );
        assert!(preview(State(s.clone()), Json(request("youtube", "video")))
            .await
            .is_err());
        let mut malicious = run;
        malicious.items[0].kind = "youtube".into();
        malicious.items[0].status = "pending".into();
        s.db.put("workflow-run", &malicious.id, &malicious).unwrap();
        assert!(
            apply(State(s), Path(malicious.id)).await.is_err(),
            "direct workflow apply must not bypass headless gate"
        );
    }
    #[tokio::test]
    async fn local_preview_rejects_concurrent_rename_without_partial_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let s = Arc::new(youtube::protocol_tests::state(temp.path(), String::new()));
        s.library.write().await.assets.push(Asset {
            id: "asset".into(),
            title: "old".into(),
            ..Default::default()
        });
        let run = preview(State(s.clone()), Json(request("local", "asset")))
            .await
            .ok()
            .unwrap()
            .0;
        s.library.write().await.assets[0].display_title = Some("manual".into());
        let result = apply(State(s.clone()), Path(run.id)).await.ok().unwrap().0;
        assert_eq!(result.items[0].status, "failed");
        assert!(s.db.list::<Value>("workflow-metadata").unwrap().is_empty());
    }

    struct Remote {
        video: Value,
        playlist: Value,
        members: Vec<String>,
        video_writes: usize,
        playlist_writes: usize,
        inserts: usize,
    }
    async fn videos(State(r): State<Arc<Mutex<Remote>>>) -> Json<Value> {
        Json(json!({"items":[r.lock().await.video.clone()]}))
    }
    async fn video_update(
        State(r): State<Arc<Mutex<Remote>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_eq!(headers["if-match"], "video-etag");
        assert_eq!(body["snippet"]["defaultLanguage"], "en");
        assert!(body.get("status").is_none());
        let mut remote = r.lock().await;
        for (key, value) in body["snippet"].as_object().unwrap() {
            remote.video["snippet"][key] = value.clone();
        }
        remote.video_writes += 1;
        Json(remote.video.clone())
    }
    async fn playlists(State(r): State<Arc<Mutex<Remote>>>) -> Json<Value> {
        Json(json!({"items":[r.lock().await.playlist.clone()]}))
    }
    async fn playlist_update(
        State(r): State<Arc<Mutex<Remote>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_eq!(headers["if-match"], "playlist-etag");
        assert!(
            body.get("status").is_none(),
            "existing privacy must be preserved"
        );
        assert!(body["snippet"]["description"]
            .as_str()
            .unwrap()
            .contains("Human note"));
        let mut remote = r.lock().await;
        for (key, value) in body["snippet"].as_object().unwrap() {
            remote.playlist["snippet"][key] = value.clone();
        }
        remote.playlist_writes += 1;
        Json(remote.playlist.clone())
    }
    async fn members(State(r): State<Arc<Mutex<Remote>>>) -> Json<Value> {
        Json(
            json!({"items":r.lock().await.members.iter().enumerate().map(|(i,id)| json!({"id":format!("m{i}"),"contentDetails":{"videoId":id}})).collect::<Vec<_>>()}),
        )
    }
    async fn insert_member(
        State(r): State<Arc<Mutex<Remote>>>,
        Json(body): Json<Value>,
    ) -> Response {
        let mut remote = r.lock().await;
        remote.members.push(
            body["snippet"]["resourceId"]["videoId"]
                .as_str()
                .unwrap()
                .into(),
        );
        remote.inserts += 1;
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":{"message":"response lost after write"}})),
        )
            .into_response()
    }
    #[tokio::test]
    async fn remote_actions_preserve_fields_and_reconcile_partial_playlist_without_replay() {
        let temp = tempfile::tempdir().unwrap();
        let original = json!({"id":"video","etag":"video-etag","snippet":{"channelId":"channel","title":"old","description":"","tags":[],"categoryId":"22","defaultLanguage":"zh"},"status":{"privacyStatus":"private","license":"youtube"}});
        let playlist = json!({"id":"playlist","etag":"playlist-etag","snippet":{"channelId":"channel","title":"Creator 123","description":"Human note\nhttps://live.bilibili.com/123","defaultLanguage":"zh"},"status":{"privacyStatus":"private"}});
        let remote = Arc::new(Mutex::new(Remote {
            video: original.clone(),
            playlist: playlist.clone(),
            members: vec![],
            video_writes: 0,
            playlist_writes: 0,
            inserts: 0,
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/api/videos", get(videos).put(video_update))
            .route("/api/playlists", get(playlists).put(playlist_update))
            .route("/api/playlistItems", get(members).post(insert_member))
            .with_state(remote.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let s = Arc::new(youtube::protocol_tests::state(temp.path(), base));
        s.db.put("yt-video", "video", &original).unwrap();
        s.db.put(
            "workflow-playlists",
            "channel",
            &json!({"channel_id":"channel","items":[playlist]}),
        )
        .unwrap();
        let mut p = request("youtube", "video");
        p.items[0].playlist = Some(PlaylistIntent {
            action: "add".into(),
            playlist_id: None,
            identity_key: "Bilibili:123".into(),
            title: "Creator".into(),
            description: "房间号：123".into(),
            reason: "test".into(),
        });
        let run = preview(State(s.clone()), Json(p)).await.ok().unwrap().0;
        remote.lock().await.video["snippet"]["title"] = json!("manual edit");
        let conflict = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(conflict.items[0].status, "failed");
        assert_eq!(remote.lock().await.video_writes, 0);
        remote.lock().await.video["snippet"]["title"] = json!("old");
        remote.lock().await.video["snippet"]["defaultLanguage"] = json!("en");
        let partial = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(partial.status, "partial");
        assert_eq!(partial.items[0].actions["video"], "completed");
        assert_eq!(partial.items[0].actions["playlist_rename"], "completed");
        assert_eq!(partial.items[0].actions["playlist_add"], "running");
        let completed = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(completed.status, "completed");
        let _ = apply(State(s.clone()), Path(run.id)).await.ok().unwrap();
        let remote = remote.lock().await;
        assert_eq!(
            (remote.video_writes, remote.playlist_writes, remote.inserts),
            (1, 1, 1)
        );
        assert_eq!(remote.video["status"]["license"], "youtube");
        assert_eq!(remote.playlist["status"]["privacyStatus"], "private");
        assert_eq!(remote.members, vec!["video"]);
        server.abort();
    }
    #[tokio::test]
    async fn failed_playlist_pagination_keeps_last_complete_cache() {
        async fn page(Query(q): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
            assert_eq!(q["maxResults"], "50");
            Json(json!({"items":[],"nextPageToken":"repeated"}))
        }
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/api/playlists", get(page)))
                .await
                .unwrap()
        });
        let s = Arc::new(youtube::protocol_tests::state(temp.path(), base));
        let previous = json!({"channel_id":"channel","items":[{"id":"retained"}]});
        s.db.put("workflow-playlists", "channel", &previous)
            .unwrap();
        assert!(sync_playlists(State(s.clone())).await.is_err());
        assert_eq!(
            s.db.get::<Value>("workflow-playlists", "channel")
                .unwrap()
                .unwrap(),
            previous
        );
        server.abort();
    }
    #[tokio::test]
    async fn known_create_rejection_is_retryable_and_discovered_human_notes_are_preserved() {
        type Creation = Arc<Mutex<(Option<Value>, usize)>>;
        async fn list(State(remote): State<Creation>) -> Json<Value> {
            Json(json!({"items":remote.lock().await.0.iter().cloned().collect::<Vec<_>>()}))
        }
        async fn create(State(remote): State<Creation>, Json(body): Json<Value>) -> Response {
            let mut remote = remote.lock().await;
            remote.1 += 1;
            if remote.1 == 1 {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error":{"message":"quotaExceeded"}})),
                )
                    .into_response();
            }
            assert_eq!(body["status"]["privacyStatus"], "private");
            let mut created = body;
            created["id"] = json!("created");
            created["etag"] = json!("playlist-etag");
            created["snippet"]["channelId"] = json!("channel");
            remote.0 = Some(created.clone());
            Json(created).into_response()
        }
        let temp = tempfile::tempdir().unwrap();
        let remote: Creation = Arc::new(Mutex::new((None, 0)));
        let original = json!({"id":"video","etag":"video-etag","snippet":{"channelId":"channel","title":"old","description":"","tags":[],"categoryId":"22"},"status":{"privacyStatus":"private"}});
        let served = original.clone();
        let router = Router::new()
            .route("/api/playlists", get(list).post(create))
            .route(
                "/api/videos",
                get(move || {
                    let v = served.clone();
                    async move { Json(json!({"items":[v]})) }
                }),
            )
            .route(
                "/api/playlistItems",
                get(|| async {
                    Json(json!({"items":[{"id":"member","contentDetails":{"videoId":"video"}}]}))
                }),
            )
            .with_state(remote.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let s = Arc::new(youtube::protocol_tests::state(temp.path(), base));
        s.db.put("yt-video", "video", &original).unwrap();
        s.db.put(
            "workflow-playlists",
            "channel",
            &json!({"channel_id":"channel","items":[]}),
        )
        .unwrap();
        let make = |room: &str| {
            let mut p = request("youtube", "video");
            p.items[0].after = p.items[0].before.clone();
            p.items[0].metadata["roomId"] = json!(room);
            p.items[0].playlist = Some(PlaylistIntent {
                action: "create".into(),
                playlist_id: None,
                identity_key: format!("Bilibili:{room}"),
                title: "Creator".into(),
                description: format!("房间号：{room}"),
                reason: String::new(),
            });
            p
        };
        let run = preview(State(s.clone()), Json(make("123")))
            .await
            .ok()
            .unwrap()
            .0;
        let denied = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(denied.items[0].actions["playlist_create"], "failed");
        assert_eq!(
            s.db.get::<Value>("workflow-playlist-create", "channel:Bilibili:123")
                .unwrap()
                .unwrap()["status"],
            "failed"
        );
        let completed = apply(State(s.clone()), Path(run.id)).await.ok().unwrap().0;
        assert_eq!(completed.status, "completed");
        assert_eq!(remote.lock().await.1, 2);
        let next = preview(State(s.clone()), Json(make("456")))
            .await
            .ok()
            .unwrap()
            .0;
        remote.lock().await.0 = Some(
            json!({"id":"human-created","etag":"fresh","snippet":{"channelId":"channel","title":"Human title","description":"Human note\nU2BUP identity: Bilibili:456"},"status":{"privacyStatus":"private"}}),
        );
        let conflict = apply(State(s), Path(next.id)).await.ok().unwrap().0;
        assert_eq!(conflict.status, "partial");
        assert!(conflict.items[0].message.contains("人工说明"));
        assert_eq!(
            remote.lock().await.0.as_ref().unwrap()["snippet"]["title"],
            "Human title"
        );
        assert_eq!(
            remote.lock().await.1,
            2,
            "discovery cannot recreate or overwrite a playlist"
        );
        server.abort();
    }
    #[tokio::test]
    async fn selected_thumbnail_is_persisted_bound_and_uploaded_only_once() {
        // A small JPEG fixture, transported unchanged; no remote channel is contacted.
        let jpeg = STANDARD.decode("/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAX/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABBQJ//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAwEBPwF//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAgEBPwF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQAGPwJ//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPyF//9oADAMBAAIAAwAAABD/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAEDAQE/EH//xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAECAQE/EH//xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAE/EH//2Q==").unwrap();
        validate_jpeg(&jpeg).unwrap();
        let writes = Arc::new(Mutex::new(0usize));
        let received = writes.clone();
        let expected = jpeg.clone();
        let original = json!({"id":"video","etag":"video-etag","snippet":{"channelId":"channel","title":"old","description":"","tags":[],"categoryId":"22","thumbnails":{"default":{"url":"old"}}},"status":{"privacyStatus":"private"}});
        let served = original.clone();
        let router = Router::new()
            .route(
                "/api/videos",
                get(move || {
                    let v = served.clone();
                    async move { Json(json!({"items":[v]})) }
                }),
            )
            .route(
                "/upload/thumbnails/set",
                post(
                    move |Query(q): Query<std::collections::HashMap<String, String>>,
                          headers: HeaderMap,
                          body: axum::body::Bytes| {
                        let count = received.clone();
                        let bytes = expected.clone();
                        async move {
                            assert_eq!(q["videoId"], "video");
                            assert_eq!(headers["content-type"], "image/jpeg");
                            assert_eq!(body.as_ref(), bytes);
                            let mut n = count.lock().await;
                            *n += 1;
                            if *n == 1 {
                                return (
                                    StatusCode::FORBIDDEN,
                                    Json(json!({"error":{"message":"permission denied"}})),
                                )
                                    .into_response();
                            }
                            Json(json!({"items":[{"default":{"url":"new"}}]})).into_response()
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("source.mp4"),
            b"synthetic-media-placeholder",
        )
        .unwrap();
        // App startup canonicalizes media roots; match that boundary on Windows too.
        let root = temp.path().canonicalize().unwrap();
        let s = Arc::new(youtube::protocol_tests::state(&root, base));
        s.library.write().await.assets.push(Asset {
            id: "asset".into(),
            relative_path: "source.mp4".into(),
            title: "old".into(),
            ..Default::default()
        });
        s.db.put("yt-video", "video", &original).unwrap();
        let mut p = request("youtube", "video");
        p.items[0].after = p.items[0].before.clone();
        p.items[0].metadata["localAssetId"] = json!("asset");
        p.items[0].thumbnail = Some(ThumbnailCandidate {
            asset_id: "asset".into(),
            time_seconds: 1.0,
            data_url: Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(&jpeg))),
            image_id: String::new(),
            modified_ms: 0,
            remote_before: Value::Null,
        });
        let run = preview(State(s.clone()), Json(p)).await.ok().unwrap().0;
        let candidate = run.items[0].thumbnail.as_ref().unwrap();
        assert!(
            candidate.data_url.is_none(),
            "base64 payload must not bloat stored runs"
        );
        assert_eq!(
            std::fs::read(
                temp.path()
                    .join("workflow-thumbnails")
                    .join(format!("{}.jpg", candidate.image_id))
            )
            .unwrap(),
            jpeg
        );
        let failed = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(
            failed.items[0].actions["thumbnail"], "failed",
            "explicit rejection is retryable"
        );
        let complete = apply(State(s.clone()), Path(run.id.clone()))
            .await
            .ok()
            .unwrap()
            .0;
        assert_eq!(complete.items[0].actions["thumbnail"], "completed");
        let _ = apply(State(s.clone()), Path(run.id)).await.ok().unwrap();
        assert_eq!(
            *writes.lock().await,
            2,
            "completed uploads must never replay"
        );
        server.abort();
    }
    #[test]
    fn malformed_thumbnail_payloads_are_rejected() {
        assert!(validate_jpeg(b"not a jpeg").is_err());
        assert!(validate_jpeg(&[0xff, 0xd8, 0xff, 0xd9]).is_err());
        assert!(validate_jpeg(&[0xff, 0xd8, 0xff, 0xc0, 0, 17, 0xff, 0xd9]).is_err());
    }
}
