//! Compatibility view used by the existing media tools for registered libraries.
//! Asset IDs stay bound to database records; callers never supply a source root.
use crate::{db::Db, media, model::*, AppState};
use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

fn relative_source(root: &LibraryRoot, source: &str) -> Result<String> {
    let root_path = Path::new(&root.path);
    let source_path = Path::new(source);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.into());
    let relative = source_path
        .strip_prefix(&canonical_root)
        .or_else(|_| source_path.strip_prefix(root_path))
        .context("素材记录不属于已登记的素材库")?;
    if relative
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("素材记录路径不合法");
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

pub(crate) fn project_asset(root: &LibraryRoot, record: &AssetRecord) -> Result<Asset> {
    let a = &record.asset;
    let relative_path = relative_source(root, &record.source_path)?;
    let directory = relative_path.split('/').next().unwrap_or(&root.name);
    let room_name = a
        .room_name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if relative_path.contains('/') {
                directory.into()
            } else {
                root.name.clone()
            }
        });
    let room_id = a
        .room_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("folder:{}", digest(format!("{}:{room_name}", root.id))));
    let scanner = a.custom_meta.as_ref().map(|v| &v["scanner"]);
    let metadata: Option<MediaInfo> =
        scanner.and_then(|v| serde_json::from_value(v["metadata"].clone()).ok());
    let role = scanner
        .and_then(|v| v["role"].as_str())
        .filter(|r| ["source", "legacy"].contains(r))
        .unwrap_or(if a.extension == "mp4" {
            "legacy"
        } else {
            "source"
        });
    let mut warnings = a.warnings.clone();
    if a.file_status != "ok" {
        warnings.push(format!("文件状态：{}；请重新扫描素材库", a.file_status));
    }
    Ok(Asset {
        id: record.id.clone(),
        name: Path::new(&record.source_path)
            .file_name()
            .context("素材文件名缺失")?
            .to_string_lossy()
            .into_owned(),
        relative_path,
        room_id,
        room_name,
        title: a.title.clone(),
        display_title: a.display_title.clone(),
        bytes: record.file_size,
        modified_ms: record.modified_ms,
        extension: a.extension.clone(),
        role: role.into(),
        started_at: a.started_at.clone(),
        time_source: a.time_source.clone().unwrap_or_else(|| "unknown".into()),
        metadata,
        sidecars: a.sidecars.clone(),
        warnings,
    })
}

pub(crate) fn project_library(db: &Db, root: &LibraryRoot) -> Result<Library> {
    let count = db.asset_count_for_library(&root.id)?;
    let records = db.asset_list_for_library(&root.id, count, 0)?;
    let mut library = Library {
        root: root.path.clone(),
        scanned_at: root.last_scanned_at.clone(),
        errors: root.scan_error.clone().into_iter().collect(),
        ..Default::default()
    };
    let mut rooms: BTreeMap<String, Room> = BTreeMap::new();
    let mut sidecars = HashSet::new();
    for record in records {
        let asset = project_asset(root, &record)?;
        sidecars.extend(asset.sidecars.iter().cloned());
        let directory = asset
            .relative_path
            .rsplit_once('/')
            .map(|(d, _)| d)
            .unwrap_or("");
        let room = rooms.entry(asset.room_id.clone()).or_insert_with(|| Room {
            id: asset.room_id.clone(),
            name: asset.room_name.clone(),
            historical_title: Some(asset.title.clone()),
            ..Default::default()
        });
        if !room.directories.iter().any(|d| d == directory) {
            room.directories.push(directory.into());
        }
        if room.name != asset.room_name && !room.aliases.contains(&asset.room_name) {
            room.aliases.push(asset.room_name.clone());
        }
        library.assets.push(asset);
    }
    for room in rooms.values_mut() {
        if let Some(saved) = db.get::<Room>("room-info", &room.id)? {
            room.online = saved.online;
            room.refreshed_at = saved.refreshed_at;
            room.refresh_error = saved.refresh_error;
        }
    }
    library.rooms = rooms.into_values().collect();
    library.sidecar_count = sidecars.len();
    Ok(library)
}

pub(crate) async fn asset(s: &AppState, id: &str) -> Result<Asset> {
    if let Some(record) = s.db.asset_get(id)? {
        let root =
            s.db.library_get::<LibraryRoot>(&record.library_id)?
                .context("素材库已移除")?;
        return project_asset(&root, &record);
    }
    s.library
        .read()
        .await
        .assets
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .context("素材不存在")
}

pub(crate) async fn selection(s: &AppState, ids: &[String]) -> Result<Library> {
    // The planner needs unselected clips too, so that a selected segment cannot
    // silently bridge footage recorded with another stream configuration.
    let legacy = s.library.read().await.clone();
    let mut library_ids = HashSet::new();
    let mut has_legacy = false;
    for id in ids {
        if let Some(record) = s.db.asset_get(id)? {
            library_ids.insert(record.library_id);
        } else if legacy.assets.iter().any(|a| &a.id == id) {
            has_legacy = true;
        } else {
            bail!("素材不存在，请刷新列表");
        }
    }
    if library_ids.len() > 1 || (has_legacy && !library_ids.is_empty()) {
        bail!("一个处理计划只能选择同一个素材库的素材，请按库分别生成计划");
    }
    if let Some(id) = library_ids.into_iter().next() {
        let root =
            s.db.library_get::<LibraryRoot>(&id)?
                .context("素材库已移除")?;
        project_library(&s.db, &root)
    } else {
        Ok(legacy)
    }
}

/// Recheck the registered library binding when consuming a preview or saved plan.
pub(crate) async fn resolve_input(s: &AppState, a: &Asset) -> Result<PathBuf> {
    if let Some(record) = s.db.asset_get(&a.id)? {
        let root =
            s.db.library_get::<LibraryRoot>(&record.library_id)?
                .context("素材库已移除，请重新生成计划")?;
        let relative = relative_source(&root, &record.source_path)?;
        if relative != a.relative_path {
            bail!("素材路径在计划生成后变化，请重新生成计划");
        }
        let canonical_root = Path::new(&root.path)
            .canonicalize()
            .context("素材库当前离线")?;
        return media::resolve_input(&canonical_root, &relative);
    }
    // A registered-library ID cannot become a legacy path after removal.
    if !s
        .library
        .read()
        .await
        .assets
        .iter()
        .any(|stored| stored.id == a.id && stored.relative_path == a.relative_path)
    {
        bail!("素材已不在索引中，请重新扫描和生成计划");
    }
    media::resolve_input(&s.config.library, &a.relative_path)
}

/// Publishing fields a local asset contributes to YouTube: the explicit
/// publishing title wins, then the library display title, then the scanned title.
pub(crate) struct Publish {
    pub title: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

pub(crate) async fn publish(s: &AppState, id: &str) -> Result<Publish> {
    if let Some(record) = s.db.asset_get(id)? {
        let a = record.asset;
        return Ok(Publish {
            title: [a.pub_title, a.display_title]
                .into_iter()
                .flatten()
                .find(|t| !t.trim().is_empty())
                .unwrap_or(a.title),
            description: a.pub_description.filter(|d| !d.trim().is_empty()),
            tags: a.pub_tags,
        });
    }
    let a = asset(s, id).await?;
    Ok(Publish {
        title: a
            .display_title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(a.title),
        description: None,
        tags: vec![],
    })
}

pub(crate) async fn set_display_title(s: &AppState, id: &str, title: String) -> Result<()> {
    if let Some(mut record) = s.db.asset_get(id)? {
        s.db.library_get::<LibraryRoot>(&record.library_id)?
            .context("素材库已移除")?;
        record.asset.display_title = Some(title);
        return s.db.asset_put(&record);
    }
    let mut library = s.library.write().await;
    library
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .context("素材不存在")?
        .display_title = Some(title);
    s.db.put("library", "main", &*library)
}

pub(crate) async fn room(s: &AppState, id: &str) -> Result<Room> {
    if let Some(room) = s
        .library
        .read()
        .await
        .rooms
        .iter()
        .find(|r| r.id == id)
        .cloned()
    {
        return Ok(room);
    }
    for root in s.db.library_list::<LibraryRoot>()? {
        if let Some(room) = project_library(&s.db, &root)?
            .rooms
            .into_iter()
            .find(|r| r.id == id)
        {
            return Ok(room);
        }
    }
    bail!("直播间不存在")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn library(path: &Path) -> LibraryRoot {
        LibraryRoot {
            id: "library-2".into(),
            name: "录播".into(),
            kind: LibraryKind::Folder,
            path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
            display_tz: "Asia/Shanghai".into(),
            readonly: false,
            enabled: true,
            scan_exclude: vec![],
            created_at: now(),
            last_scanned_at: Some(now()),
            scan_status: "idle".into(),
            scan_error: None,
            asset_count: 1,
        }
    }
    fn record(root: &LibraryRoot, path: &Path) -> AssetRecord {
        let asset = AssetV2 {
            id: "asset-2".into(),
            library_id: root.id.clone(),
            source_path: path.to_string_lossy().into_owned(),
            title: "直播标题".into(),
            room_id: Some("123456".into()),
            room_name: Some("主播".into()),
            started_at: Some("2026-09-25T09:53:02+08:00".into()),
            time_source: Some("filename".into()),
            extension: "mp4".into(),
            file_status: "ok".into(),
            sidecars: vec!["主播/source.xml".into()],
            custom_meta: Some(json!({"scanner":{"version":1,"role":"source","metadata":{
                "duration":3600.0,"width":1080,"height":1920,"codec":"h264","aspect":"9:16",
                "signature":"sig","streams":[{"codec_type":"video","codec_name":"h264"}]
            }}})),
            ..Default::default()
        };
        AssetRecord {
            id: asset.id.clone(),
            library_id: root.id.clone(),
            source_path: asset.source_path.clone(),
            content_hash: None,
            pub_title: None,
            custom_tags_json: None,
            modified_ms: 123,
            file_size: 456,
            asset,
        }
    }
    #[test]
    fn projection_preserves_recording_identity_and_complete_probe_for_planning() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("主播/merged/source.mp4");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "video").unwrap();
        let root = library(temp.path());
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        db.library_put(&root.id, &root).unwrap();
        db.asset_put(&record(&root, &source.canonicalize().unwrap()))
            .unwrap();
        let projected = project_library(&db, &root).unwrap();
        assert_eq!(projected.assets.len(), 1);
        assert_eq!(projected.assets[0].relative_path, "主播/merged/source.mp4");
        assert_eq!(projected.assets[0].role, "source");
        assert_eq!(
            projected.assets[0].metadata.as_ref().unwrap().signature,
            "sig"
        );
        assert_eq!(projected.rooms[0].id, "123456");
        assert_eq!(projected.sidecar_count, 1);
        let plan = crate::planner::build(
            &projected,
            serde_json::from_value(json!({"asset_ids":["asset-2"]})).unwrap(),
        )
        .unwrap();
        assert_eq!(plan.outputs.len(), 1);
        assert!(plan.blocked.is_empty());
    }
    #[test]
    fn projection_rejects_a_record_outside_its_registered_root() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = library(temp.path());
        assert!(project_asset(&root, &record(&root, &outside.path().join("source.mp4"))).is_err());
        assert!(project_asset(&root, &record(&root, &temp.path().join("../source.mp4"))).is_err());
    }
}
