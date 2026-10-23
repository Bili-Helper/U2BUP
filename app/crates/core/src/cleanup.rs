//! Explicit, persisted manual cleanup. No arbitrary paths accepted from clients.
use crate::{catalog, model::LibraryRoot, web::ApiError, AppState};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, path::PathBuf, sync::Arc};

#[derive(Clone, Serialize, Deserialize)]
struct Item {
    asset_id: String,
    path: PathBuf,
    root: PathBuf,
    bytes: u64,
    stamp: String,
    status: String,
    message: String,
    #[serde(default)]
    remote: Option<RemoteEvidence>,
}
#[derive(Clone, Serialize, Deserialize)]
struct RemoteEvidence {
    video_id: String,
    channel_id: String,
    checked_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Plan {
    id: String,
    created_at: String,
    platform: String,
    note: String,
    method: String,
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Preview {
    asset_ids: Vec<String>,
    excluded_ids: Vec<String>,
    platform: String,
    note: String,
    confirmed_uploaded: bool,
    #[serde(default = "manual_method")]
    method: String,
}
fn manual_method() -> String {
    "user_confirmed".into()
}
#[derive(Deserialize)]
struct Apply {
    confirm_permanent_delete: bool,
}
type Http<T> = std::result::Result<Json<T>, ApiError>;

pub(crate) struct UploadGuard {
    _file: locked::Locked,
    pub stamp: String,
}
pub(crate) fn guard_upload(path: &std::path::Path) -> Result<UploadGuard> {
    let file = locked::open_upload(path.parent().context("上传路径缺少目录")?, path)?;
    Ok(UploadGuard {
        stamp: file.stamp.clone(),
        _file: file,
    })
}
fn validate_remote(v: &Value, e: &RemoteEvidence) -> Result<()> {
    let video = v["items"]
        .as_array()
        .and_then(|items| items.iter().find(|v| v["id"] == e.video_id))
        .context("远端视频不存在或不可访问")?;
    if video["snippet"]["channelId"] != e.channel_id {
        bail!("远端视频不属于本次确认频道");
    }
    if video["status"]["uploadStatus"] != "processed"
        || video["processingDetails"]["processingStatus"] != "succeeded"
    {
        bail!("YouTube 尚未确认处理成功，保留本地视频");
    }
    if video["status"]["failureReason"].is_string()
        || video["status"]["rejectionReason"].is_string()
    {
        bail!("远端视频存在失败或拒绝状态");
    }
    Ok(())
}
async fn verify_remote(s: &AppState, e: &RemoteEvidence) -> Result<()> {
    if crate::youtube::workflow_channel(s)? != e.channel_id {
        bail!("当前频道与冻结计划不同");
    }
    let v = crate::youtube::api_get(
        s,
        "videos",
        &[
            ("part", "snippet,status,processingDetails"),
            ("id", &e.video_id),
        ],
    )
    .await?;
    validate_remote(&v, e)
}
async fn upload_evidence(s: &AppState, item: &Item) -> Result<RemoteEvidence> {
    let channel = crate::youtube::workflow_channel(s)?;
    let jobs = s.db.list::<Value>("yt-upload")?;
    let j = jobs
        .iter()
        .find(|j| {
            j["status"] == "completed"
                && j["channel_id"] == channel
                && j["file_stamp"] == item.stamp
                && j["path"]
                    .as_str()
                    .is_some_and(|p| PathBuf::from(p) == item.path)
        })
        .context("无同一文件的完整上传凭据；历史上传或切片来源请人工核对完整覆盖")?;
    let e = RemoteEvidence {
        video_id: j["video_id"].as_str().context("上传记录无视频 ID")?.into(),
        channel_id: channel,
        checked_at: crate::model::now(),
    };
    verify_remote(s, &e).await?;
    Ok(e)
}

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/workflows/cleanup", get(history).post(preview))
        .route("/api/workflows/cleanup/{id}/apply", post(apply))
}
async fn history(State(s): State<Arc<AppState>>) -> Http<Value> {
    Ok(Json(
        json!({"plans":s.db.list::<Plan>("cleanup-plan")?, "supported":cfg!(windows), "assets":s.db.list::<Value>("cleanup-asset")?}),
    ))
}
pub(crate) fn restore_metadata(
    db: &crate::db::Db,
    library: &mut crate::model::Library,
) -> Result<()> {
    for plan in db.list::<Plan>("cleanup-plan")? {
        for item in plan
            .items
            .iter()
            .filter(|i| i.status == "deleted" && i.root == PathBuf::from(&library.root))
        {
            if item.path.exists() {
                continue;
            }
            if let Some(a) = library.assets.iter_mut().find(|a| a.id == item.asset_id) {
                if !a
                    .warnings
                    .iter()
                    .any(|w| w == "本地视频已清理；弹幕和元数据保留")
                {
                    a.warnings.push("本地视频已清理；弹幕和元数据保留".into());
                }
                continue;
            }
            if let Some(saved) = db.get::<Value>("cleanup-asset", &item.asset_id)? {
                let mut a: crate::model::Asset = serde_json::from_value(saved["asset"].clone())?;
                a.warnings.push("本地视频已清理；弹幕和元数据保留".into());
                library.assets.push(a);
            }
        }
    }
    Ok(())
}
pub(crate) fn recover(db: &crate::db::Db) -> Result<()> {
    for mut p in db.list::<Plan>("cleanup-plan")? {
        let mut changed = false;
        for item in &mut p.items {
            if item.status == "deleting" {
                item.status = "needs_review".into();
                item.message = "删除时服务中断，请人工核对；不会自动重试".into();
                changed = true;
            }
        }
        if changed {
            db.put("cleanup-plan", &p.id, &p)?;
        }
    }
    Ok(())
}
pub(crate) fn task_summaries(db: &crate::db::Db) -> Result<Vec<Value>> {
    Ok(db.list::<Plan>("cleanup-plan")?.iter().map(|p| {
        let deleted=p.items.iter().filter(|i|i.status=="deleted").count();
        let waiting=p.items.iter().any(|i|i.status=="planned");
        let problem=p.items.iter().any(|i|matches!(i.status.as_str(),"failed"|"conflict"|"deleting"|"needs_review"|"blocked"));
        json!({"id":format!("cleanup:{}",p.id),"source_id":p.id,"kind":"cleanup","title":"上传后清理本地视频","status":if problem {"interrupted"} else if waiting {"pending"} else {"completed"},"created_at":p.created_at,"updated_at":p.created_at,"progress":null,"message":format!("已删除 {deleted} 项；在工作流清理面板查看逐项记录"),"capabilities":{"cancel":false,"retry":false,"pause":false,"resume":false}})
    }).collect())
}
async fn binding(s: &AppState, id: &str) -> Result<(PathBuf, PathBuf)> {
    if let Some(a) = s.db.asset_get(id)? {
        let root =
            s.db.library_get::<LibraryRoot>(&a.library_id)?
                .context("素材库不存在")?;
        if root.readonly || !root.enabled {
            bail!("素材库只读或已禁用");
        }
        Ok((root.path.into(), a.source_path.into()))
    } else {
        let a = catalog::asset(s, id).await?;
        Ok((
            s.config.library.clone(),
            s.config.library.join(a.relative_path),
        ))
    }
}
fn enabled(s: &AppState) -> Result<()> {
    if !cfg!(windows) {
        bail!("此预览版仅在 Windows 支持安全删除");
    }
    let state =
        s.db.get::<Value>("workflow-modules", "registry")?
            .unwrap_or_default();
    if state["disabledBuiltinIds"].as_array().is_some_and(|a| {
        a.iter()
            .any(|v| v == "com.u2bup.builtin.cleanup-local-video")
    }) {
        bail!("清理模块已禁用");
    }
    Ok(())
}
fn idle(s: &AppState) -> Result<()> {
    for kind in ["job", "yt-upload"] {
        for job in s.db.list::<Value>(kind)? {
            if !matches!(
                job["status"].as_str(),
                Some("completed" | "cancelled" | "failed")
            ) {
                bail!("存在未结束的媒体或上传任务，请先完成或取消后再清理");
            }
        }
    }
    Ok(())
}
async fn preview(State(s): State<Arc<AppState>>, Json(b): Json<Preview>) -> Http<Plan> {
    enabled(&s)?;
    if !["user_confirmed", "platform_verified"].contains(&b.method.as_str())
        || (b.method == "user_confirmed"
            && (!b.confirmed_uploaded || b.platform.trim().is_empty() || b.note.trim().is_empty()))
        || b.asset_ids.is_empty()
        || b.asset_ids.len() > 500
    {
        return Err(anyhow::anyhow!(
            "请选择 1–500 个素材，填写平台与核对说明，并确认所选视频完整内容已在线保存"
        )
        .into());
    }
    let _op = s.operation.try_lock().context("媒体任务忙")?;
    let _yt = s.youtube.operation.try_lock().context("上传任务忙")?;
    idle(&s)?;
    let mut plan = Plan {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: crate::model::now(),
        platform: if b.method == "platform_verified" {
            "YouTube".into()
        } else {
            b.platform
        },
        note: b.note,
        method: b.method,
        items: vec![],
    };
    let mut seen = HashSet::new();
    for id in b.asset_ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let (root, path) = binding(&s, &id).await?;
        let mut item = Item {
            asset_id: id.clone(),
            path,
            root,
            bytes: 0,
            stamp: String::new(),
            status: "planned".into(),
            message: String::new(),
            remote: None,
        };
        if b.excluded_ids.contains(&id) {
            item.status = "excluded".into();
            item.message = "用户保留".into();
        } else {
            match locked::open(&item.root, &item.path) {
                Ok(file) => {
                    item.bytes = file.bytes;
                    item.stamp = file.stamp.clone();
                }
                Err(e) => {
                    item.status = "blocked".into();
                    item.message = e.to_string();
                }
            }
        }
        if item.status == "planned" && plan.method == "platform_verified" {
            match upload_evidence(&s, &item).await {
                Ok(e) => item.remote = Some(e),
                Err(e) => {
                    item.status = "blocked".into();
                    item.message = e.to_string();
                }
            }
        }
        plan.items.push(item);
    }
    s.db.put("cleanup-plan", &plan.id, &plan)?;
    Ok(Json(plan))
}
async fn apply(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(b): Json<Apply>,
) -> Http<Plan> {
    enabled(&s)?;
    if !b.confirm_permanent_delete {
        return Err(anyhow::anyhow!("需要明确确认永久删除冻结清单中的视频").into());
    }
    let _op = s.operation.try_lock().context("媒体任务忙")?;
    let _yt = s.youtube.operation.try_lock().context("上传任务忙")?;
    idle(&s)?;
    let mut plan =
        s.db.get::<Plan>("cleanup-plan", &id)?
            .context("清理计划不存在")?;
    for i in 0..plan.items.len() {
        if plan.items[i].status == "deleting" {
            plan.items[i].status = "needs_review".into();
            plan.items[i].message = "上次执行中断，结果需人工核对；不会重放删除".into();
            s.db.put("cleanup-plan", &id, &plan)?;
            continue;
        }
        if plan.items[i].status != "planned" {
            continue;
        }
        let item = plan.items[i].clone();
        let checked = async {
            if plan.method == "platform_verified" {
                verify_remote(&s, item.remote.as_ref().context("计划缺少平台核验凭据")?).await?;
            } else if plan.method != "user_confirmed" {
                bail!("未知确认方式");
            }
            let (root, path) = binding(&s, &item.asset_id).await?;
            if root != item.root || path != item.path {
                bail!("素材库或路径已变化，请重新预览");
            }
            let file = locked::open(&root, &path)?;
            if file.stamp != item.stamp {
                bail!("文件已变化，请重新预览");
            }
            Ok::<_, anyhow::Error>(file)
        }
        .await;
        match checked {
            Err(e) => {
                plan.items[i].status = "conflict".into();
                plan.items[i].message = e.to_string();
            }
            Ok(file) => {
                // Preserve the original metadata independently of subsequent rescans.
                let asset = catalog::asset(&s, &item.asset_id).await?;
                s.db.put("cleanup-asset", &item.asset_id, &json!({"asset":asset,"plan_id":id,"platform":plan.platform,"note":plan.note,"path":item.path,"remote":item.remote,"method":plan.method}))?;
                plan.items[i].status = "deleting".into();
                s.db.put("cleanup-plan", &id, &plan)?;
                match file.delete() {
                    Ok(()) => {
                        plan.items[i].status = "deleted".into();
                        plan.items[i].message = format!("本地视频已清理 · {}", crate::model::now());
                        if let Some(mut a) = s.db.asset_get(&item.asset_id)? {
                            a.asset.file_status = "local_deleted".into();
                            s.db.asset_put(&a)?;
                        }
                    }
                    Err(e) => {
                        plan.items[i].status = "failed".into();
                        plan.items[i].message = e.to_string();
                    }
                }
            }
        }
        s.db.put("cleanup-plan", &id, &plan)?;
    }
    Ok(Json(plan))
}

// Lock every ancestor against renaming and open the leaf without sharing writes
// or deletes. Delete that same Windows handle, never re-resolve a pathname.
#[cfg(windows)]
mod locked {
    use anyhow::{bail, Result};
    use std::{
        fs::{File, OpenOptions},
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Component, Path},
    };
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        attrs: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        high: u32,
        low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(h: *mut std::ffi::c_void, info: *mut Info) -> i32;
        fn SetFileInformationByHandle(
            h: *mut std::ffi::c_void,
            class: u32,
            data: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    pub(crate) struct Locked {
        file: File,
        _parents: Vec<File>,
        pub bytes: u64,
        pub stamp: String,
    }
    impl Locked {
        pub fn delete(self) -> Result<()> {
            let yes: i32 = 1;
            if unsafe {
                SetFileInformationByHandle(
                    self.file.as_raw_handle(),
                    4,
                    (&yes as *const i32).cast(),
                    4,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        }
    }
    pub(super) fn open(root: &Path, path: &Path) -> Result<Locked> {
        open_access(root, path, true)
    }
    pub(super) fn open_upload(root: &Path, path: &Path) -> Result<Locked> {
        open_access(root, path, false)
    }
    fn open_access(root: &Path, path: &Path, delete: bool) -> Result<Locked> {
        if !path.is_absolute()
            || !root.is_absolute()
            || !path.starts_with(root)
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            bail!("路径超出素材库");
        }
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["mp4", "mov", "mkv", "flv", "webm", "ts"].contains(&ext.as_str()) {
            bail!("不是支持的视频文件，保留");
        }
        let mut parents = vec![];
        let mut dirs = path.ancestors().skip(1).collect::<Vec<_>>();
        dirs.reverse();
        for dir in dirs {
            let f = OpenOptions::new()
                .access_mode(0x80)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(dir)?;
            if f.metadata()?.file_attributes() & 0x400 != 0 {
                bail!("拒绝链接或目录连接");
            }
            parents.push(f);
        }
        let file = OpenOptions::new()
            .access_mode(if delete { 0x80010000 } else { 0x80000000 })
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            bail!("拒绝链接和非普通文件");
        }
        let mut info = Info::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if info.links != 1 {
            bail!("共享硬链接文件需另行核对，保留");
        }
        let stamp = format!(
            "{}:{}:{}:{}:{}",
            info.volume,
            info.index_high,
            info.index_low,
            meta.file_size(),
            meta.last_write_time()
        );
        Ok(Locked {
            file,
            _parents: parents,
            bytes: meta.file_size(),
            stamp,
        })
    }
}
#[cfg(not(windows))]
mod locked {
    use anyhow::{bail, Result};
    use std::path::Path;
    pub(crate) struct Locked {
        pub bytes: u64,
        pub stamp: String,
    }
    pub(super) fn open(_: &Path, _: &Path) -> Result<Locked> {
        bail!("此版本尚不支持此平台的安全删除")
    }
    pub(super) fn open_upload(root: &Path, path: &Path) -> Result<Locked> {
        open(root, path)
    }
    impl Locked {
        pub fn delete(self) -> Result<()> {
            bail!("此版本尚不支持此平台的安全删除")
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    async fn sample(count: usize) -> (tempfile::TempDir, Arc<AppState>, Vec<String>) {
        let d = tempfile::tempdir().unwrap();
        let s = Arc::new(crate::youtube::protocol_tests::state(
            d.path(),
            String::new(),
        ));
        let mut l = crate::model::Library {
            root: d.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut ids = vec![];
        for n in 0..count {
            let id = format!("录像 {n}.flv");
            std::fs::write(d.path().join(&id), b"video").unwrap();
            l.assets.push(crate::model::Asset {
                id: id.clone(),
                relative_path: id.clone(),
                name: id.clone(),
                bytes: 5,
                extension: "flv".into(),
                ..Default::default()
            });
            ids.push(id);
        }
        *s.library.write().await = l;
        (d, s, ids)
    }
    fn request(ids: Vec<String>) -> Preview {
        Preview {
            asset_ids: ids,
            excluded_ids: vec![],
            platform: "外部平台".into(),
            note: "已逐项核对完整内容".into(),
            confirmed_uploaded: true,
            method: manual_method(),
        }
    }
    #[tokio::test]
    async fn large_batch_keeps_excluded_and_all_sidecars() {
        let (d, s, ids) = sample(121).await;
        let mut b = request(ids.clone());
        b.excluded_ids = vec![ids[120].clone()];
        for name in [
            "聊天.xml",
            "直播间.txt",
            "字幕.ass",
            "封面.jpg",
            "未知.bin",
            "脚本.ps1",
        ] {
            std::fs::write(d.path().join(name), name.as_bytes()).unwrap();
        }
        let p = preview(State(s.clone()), Json(b)).await.unwrap().0;
        std::fs::write(d.path().join("新增.mp4"), b"new").unwrap();
        let p = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(
            p.items.iter().filter(|i| i.status == "deleted").count(),
            120
        );
        assert!(d.path().join(&ids[120]).exists());
        assert!(d.path().join("新增.mp4").exists());
        for name in [
            "聊天.xml",
            "直播间.txt",
            "字幕.ass",
            "封面.jpg",
            "未知.bin",
            "脚本.ps1",
        ] {
            assert_eq!(std::fs::read(d.path().join(name)).unwrap(), name.as_bytes());
        }
    }
    #[tokio::test]
    async fn disabled_active_changed_and_interrupted_items_are_not_deleted() {
        let (d, s, ids) = sample(1).await;
        s.db.put(
            "workflow-modules",
            "registry",
            &json!({"disabledBuiltinIds":["com.u2bup.builtin.cleanup-local-video"]}),
        )
        .unwrap();
        assert!(preview(State(s.clone()), Json(request(ids.clone())))
            .await
            .is_err());
        s.db.delete("workflow-modules", "registry").unwrap();
        s.db.put("yt-upload", "busy", &json!({"status":"paused"}))
            .unwrap();
        assert!(preview(State(s.clone()), Json(request(ids.clone())))
            .await
            .is_err());
        s.db.delete("yt-upload", "busy").unwrap();
        let mut p = preview(State(s.clone()), Json(request(ids.clone())))
            .await
            .unwrap()
            .0;
        p.items[0].status = "deleting".into();
        s.db.put("cleanup-plan", &p.id, &p).unwrap();
        recover(&s.db).unwrap();
        let p = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p.items[0].status, "needs_review");
        assert!(d.path().join(&ids[0]).exists());
        let p = preview(State(s.clone()), Json(request(ids.clone())))
            .await
            .unwrap()
            .0;
        std::fs::write(d.path().join(&ids[0]), b"replacement").unwrap();
        let p = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p.items[0].status, "conflict");
        assert!(d.path().join(&ids[0]).exists());
    }
    #[test]
    fn platform_status_requires_owner_and_processing_success() {
        let e = RemoteEvidence {
            video_id: "video".into(),
            channel_id: "channel".into(),
            checked_at: String::new(),
        };
        let good = json!({"items":[{"id":"video","snippet":{"channelId":"channel"},"status":{"uploadStatus":"processed","privacyStatus":"private"},"processingDetails":{"processingStatus":"succeeded"}}]});
        assert!(validate_remote(&good, &e).is_ok());
        for status in ["processing", "failed", "terminated", ""] {
            let mut v = good.clone();
            v["items"][0]["processingDetails"]["processingStatus"] = json!(status);
            assert!(validate_remote(&v, &e).is_err());
        }
        let mut v = good.clone();
        v["items"][0]["snippet"]["channelId"] = json!("other");
        assert!(validate_remote(&v, &e).is_err());
        assert!(validate_remote(&json!({"items":[]}), &e).is_err());
    }
    #[tokio::test]
    async fn platform_mode_rechecks_remote_before_delete_and_rejects_legacy_evidence() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let ready = Arc::new(AtomicBool::new(true));
        let server_ready = ready.clone();
        let app=Router::new().route("/api/videos",get(move ||{let r=server_ready.clone();async move {Json(json!({"items":[{"id":"video","snippet":{"channelId":"channel"},"status":{"uploadStatus":"processed"},"processingDetails":{"processingStatus":if r.load(Ordering::SeqCst){"succeeded"}else{"processing"}}}]}))}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (d, source, ids) = sample(1).await;
        let s = Arc::new(crate::youtube::protocol_tests::state(
            d.path(),
            format!("http://{addr}"),
        ));
        *s.library.write().await = source.library.read().await.clone();
        let p = d.path().join(&ids[0]);
        let stamp = locked::open(d.path(), &p).unwrap().stamp.clone();
        s.db.put("yt-upload","upload",&json!({"status":"completed","path":p,"channel_id":"channel","file_stamp":stamp,"video_id":"video"})).unwrap();
        let mut b = request(ids.clone());
        b.method = "platform_verified".into();
        b.confirmed_uploaded = false;
        let p = preview(State(s.clone()), Json(b)).await.unwrap().0;
        assert_eq!(p.items[0].status, "planned");
        ready.store(false, Ordering::SeqCst);
        let p = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p.items[0].status, "conflict");
        assert!(d.path().join(&ids[0]).exists());
        s.db.put("yt-upload","upload",&json!({"status":"completed","path":d.path().join(&ids[0]),"channel_id":"channel","video_id":"video"})).unwrap();
        let mut b = request(ids);
        b.method = "platform_verified".into();
        let p = preview(State(s), Json(b)).await.unwrap().0;
        assert_eq!(p.items[0].status, "blocked");
        server.abort();
    }
    #[tokio::test]
    async fn frozen_manual_plan_excludes_and_never_replays_deleted_files() {
        let d = tempfile::tempdir().unwrap();
        let s = Arc::new(crate::youtube::protocol_tests::state(
            d.path(),
            String::new(),
        ));
        let mut library = crate::model::Library {
            root: d.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        for name in ["delete.flv", "keep.mp4"] {
            std::fs::write(d.path().join(name), b"video").unwrap();
            let a=serde_json::from_value(json!({"id":name,"name":name,"relative_path":name,"room_id":"room","room_name":"room","title":name,"bytes":5,"modified_ms":0,"extension":name.rsplit('.').next().unwrap(),"role":"source","time_source":"unknown","sidecars":[],"warnings":[]})).unwrap();
            library.assets.push(a);
        }
        *s.library.write().await = library;
        std::fs::write(d.path().join("notes.xml"), b"danmaku").unwrap();
        let body = || Preview {
            asset_ids: vec!["delete.flv".into(), "keep.mp4".into()],
            excluded_ids: vec!["keep.mp4".into()],
            platform: "external".into(),
            note: "完整核对".into(),
            confirmed_uploaded: true,
            method: manual_method(),
        };
        let mut unconfirmed = body();
        unconfirmed.confirmed_uploaded = false;
        assert!(preview(State(s.clone()), Json(unconfirmed)).await.is_err());
        let p = preview(State(s.clone()), Json(body())).await.unwrap().0;
        assert!(d.path().join("delete.flv").exists());
        assert!(apply(
            State(s.clone()),
            Path(p.id.clone()),
            Json(Apply {
                confirm_permanent_delete: false
            })
        )
        .await
        .is_err());
        let p = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(p.items[0].status, "deleted");
        assert_eq!(p.items[1].status, "excluded");
        assert!(d.path().join("keep.mp4").exists());
        assert_eq!(
            std::fs::read(d.path().join("notes.xml")).unwrap(),
            b"danmaku"
        );
        std::fs::write(d.path().join("delete.flv"), b"new file").unwrap();
        let _ = apply(
            State(s.clone()),
            Path(p.id),
            Json(Apply {
                confirm_permanent_delete: true,
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(d.path().join("delete.flv")).unwrap(),
            b"new file"
        );
        let mut rescan = crate::model::Library {
            root: d.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        restore_metadata(&s.db, &mut rescan).unwrap();
        assert_eq!(
            rescan.assets.len(),
            0,
            "new local file must not be marked deleted"
        );
        std::fs::remove_file(d.path().join("delete.flv")).unwrap();
        restore_metadata(&s.db, &mut rescan).unwrap();
        assert_eq!(rescan.assets.len(), 1);
    }
    #[test]
    fn deletes_only_locked_video_preserves_sidecars() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("中文.mp4");
        let xml = d.path().join("中文.xml");
        std::fs::write(&p, b"video").unwrap();
        std::fs::write(&xml, b"danmaku").unwrap();
        assert!(locked::open(d.path(), &xml).is_err());
        let f = locked::open(d.path(), &p).unwrap();
        assert!(std::fs::write(&p, b"replacement").is_err());
        assert!(std::fs::rename(&p, d.path().join("new.mp4")).is_err());
        f.delete().unwrap();
        assert!(!p.exists());
        assert_eq!(std::fs::read(xml).unwrap(), b"danmaku");
    }
    #[test]
    fn rejects_outside_paths_and_detects_changes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let p = d.path().join("outside.mp4");
        std::fs::write(&p, b"a").unwrap();
        assert!(locked::open(&root, &p).is_err());
        let p = root.join("file.mp4");
        std::fs::write(&p, b"a").unwrap();
        let stamp = locked::open(&root, &p).unwrap().stamp.clone();
        std::fs::write(&p, b"changed").unwrap();
        assert_ne!(locked::open(&root, &p).unwrap().stamp, stamp);
    }
}
