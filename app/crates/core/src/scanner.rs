use crate::{media, model::*, AppState};
use anyhow::Result;
use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use quick_xml::{events::Event, Reader};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use walkdir::WalkDir;

static RECORDING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:录制-)?(\d+)-(\d{8})-(\d{6})-\d+-(.*)$").unwrap());
static ALTERNATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}-\d{2}-\d{2})-\d+\s+(.*)$").unwrap());
static PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)_PART\d+$").unwrap());
static MERGED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.*?)_(\d{8})_(\d{6})_(.*?)_\[[^\]]+\](?:_PART\d+)?$").unwrap());

pub fn filename_info(stem: &str) -> (Option<String>, String) {
    let stem = stem.strip_suffix(".flv").unwrap_or(stem);
    let (dt, title) = if let Some(c) = RECORDING.captures(stem) {
        (
            NaiveDateTime::parse_from_str(&format!("{} {}", &c[2], &c[3]), "%Y%m%d %H%M%S").ok(),
            c[4].to_string(),
        )
    } else if let Some(c) = ALTERNATE.captures(stem) {
        (
            NaiveDateTime::parse_from_str(&format!("{} {}", &c[1], &c[2]), "%Y-%m-%d %H-%M-%S")
                .ok(),
            c[3].to_string(),
        )
    } else if let Some(c) = MERGED.captures(stem) {
        (
            NaiveDateTime::parse_from_str(&format!("{} {}", &c[2], &c[3]), "%Y%m%d %H%M%S").ok(),
            c[4].replace('_', " "),
        )
    } else {
        (None, stem.to_string())
    };
    let start = dt
        .and_then(|d| {
            FixedOffset::east_opt(8 * 3600)
                .unwrap()
                .from_local_datetime(&d)
                .single()
        })
        .map(|d| d.to_rfc3339());
    (start, PART.replace(title.trim(), "").to_string())
}

pub fn xml_info(path: &Path) -> HashMap<String, String> {
    let mut bytes = Vec::new();
    let Ok(f) = File::open(path) else {
        return HashMap::new();
    };
    if f.take(128 * 1024).read_to_end(&mut bytes).is_err() {
        return HashMap::new();
    }
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut metadata = HashMap::new();
    let mut in_metadata = false;
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e))
                if e.name().as_ref() == b"BililiveRecorderRecordInfo" =>
            {
                return e
                    .attributes()
                    .filter_map(|a| a.ok())
                    .filter_map(|a| {
                        Some((
                            String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                            a.decode_and_unescape_value(reader.decoder())
                                .ok()?
                                .into_owned(),
                        ))
                    })
                    .collect();
            }
            Ok(Event::Start(e)) if e.name().as_ref() == b"metadata" => {
                in_metadata = true;
            }
            Ok(Event::Start(e)) if in_metadata => {
                let key = match e.name().as_ref() {
                    b"video_start_time" => "video_start_time",
                    b"room_title" => "title",
                    b"user_name" => "name",
                    b"room_id" => "roomid",
                    _ => continue,
                };
                if let Ok(value) = reader.read_text(e.name()) {
                    if let Ok(value) = quick_xml::escape::unescape(&value) {
                        metadata.insert(key.to_string(), value.trim().to_string());
                    }
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"metadata" => {
                if let Some(time) = metadata
                    .get("video_start_time")
                    .and_then(|s| s.parse::<i64>().ok())
                    .and_then(chrono::DateTime::from_timestamp_millis)
                {
                    metadata.insert(
                        "start_time".into(),
                        time.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                            .to_rfc3339(),
                    );
                    metadata.insert("start_time_source".into(), "xml:video_start_time".into());
                }
                return metadata;
            }
            Ok(Event::Eof) | Err(_) => return metadata,
            _ => {}
        }
    }
}

/// Recorder metadata shared by the legacy and multi-library scanners.
/// A merged recording may retain its original XML one directory above `merged`.
/// Only the matching first segment is associated here; this does not imply that
/// its danmaku covers the entire merged video.
pub struct RecordingInfo {
    pub title: String,
    pub started_at: Option<String>,
    pub time_source: String,
    pub room_id: Option<String>,
    pub room_name: Option<String>,
    pub sidecars: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub role: &'static str,
}

pub fn recording_info(root: &Path, path: &Path) -> RecordingInfo {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let (mut started_at, mut title) = filename_info(&stem);
    let merged = MERGED.captures(&stem);
    let mut room_name = merged.as_ref().map(|c| c[1].to_string());
    let mut room_id = RECORDING.captures(&stem).map(|c| c[1].to_string());
    let mut time_source = if started_at.is_some() {
        "filename:Asia/Shanghai".to_string()
    } else {
        "unknown".to_string()
    };
    let mut warnings = Vec::new();
    let mut sidecars = Vec::new();
    let mut info = HashMap::new();
    for extension in ["xml", "txt"] {
        let mut candidates = vec![path.with_extension(extension)];
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        {
            candidates.push(path.with_file_name(format!(
                "录制-{}.{}",
                stem.strip_suffix(".flv").unwrap_or(&stem),
                extension
            )));
        }
        for candidate in candidates {
            if std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_file()) {
                if extension == "xml" {
                    info = xml_info(&candidate);
                }
                sidecars.push(candidate);
                break;
            }
        }
    }
    if info.is_empty() && merged.is_some() {
        if let Some(original_dir) = path
            .parent()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("merged"))
            })
            .and_then(Path::parent)
            .filter(|p| p.starts_with(root))
        {
            // Compare filename timestamps rather than XML timestamps: recorder
            // startup can add a few seconds between the two.
            let mut candidates: Vec<_> = std::fs::read_dir(original_dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")))
                .filter(|p| {
                    filename_info(&p.file_stem().unwrap_or_default().to_string_lossy()).0
                        == started_at
                })
                .collect();
            candidates.sort();
            if candidates.len() == 1 {
                let candidate = candidates.remove(0);
                info = xml_info(&candidate);
                sidecars.push(candidate);
                warnings.push("合并文件关联起始分段 XML，弹幕尚未合并".into());
            } else if candidates.len() > 1 {
                warnings.push("存在多个同时间 XML，未自动关联".into());
            }
        }
    }
    if let Some(xml_time) = info
        .get("start_time")
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
    {
        if started_at
            .as_ref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| {
                // Filenames contain a local wall clock without an offset.
                // A whole-hour (or quarter-hour) difference can reflect the
                // recorder's host timezone rather than conflicting metadata.
                let seconds = (xml_time - t).num_seconds();
                let timezone_shift = (seconds as f64 / 900.0).round() as i64 * 900;
                !(-6 * 3600..=20 * 3600).contains(&timezone_shift)
                    || (seconds - timezone_shift).abs() > 60
            })
        {
            warnings.push("XML 时间与文件名冲突，当前使用 XML".into());
        }
        started_at = Some(xml_time.to_rfc3339());
        time_source = info
            .get("start_time_source")
            .cloned()
            .unwrap_or_else(|| "xml:start_time".into());
    }
    if let Some(value) = info.get("title").filter(|s| !s.trim().is_empty()) {
        title = PART.replace(value.trim(), "").to_string();
    }
    if let Some(value) = info
        .get("roomid")
        .filter(|s| !s.is_empty() && s.as_str() != "0")
    {
        room_id = Some(value.clone());
    }
    if let Some(value) = info.get("name").filter(|s| !s.is_empty()) {
        room_name = Some(value.clone());
    }
    if room_id.is_none() && merged.is_some() {
        if let (Some(name), Some(original_dir)) = (
            room_name.as_ref(),
            path.parent()
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("merged"))
                })
                .and_then(Path::parent)
                .filter(|p| p.starts_with(root)),
        ) {
            // A new merged file can arrive before its corresponding XML. Reuse
            // only an unambiguous room identity from this same recorder folder;
            // never borrow another session's title, time, or danmaku sidecar.
            let mut identities = std::collections::BTreeSet::new();
            for entry in std::fs::read_dir(original_dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_ok_and(|t| t.is_file())
                    || !entry
                        .path()
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
                {
                    continue;
                }
                let identity = xml_info(&entry.path());
                if identity.get("name") == Some(name) {
                    if let Some(id) = identity
                        .get("roomid")
                        .filter(|id| !id.is_empty() && id.as_str() != "0")
                    {
                        identities.insert(id.clone());
                    }
                }
            }
            if identities.len() == 1 {
                room_id = identities.into_iter().next();
            } else if identities.len() > 1 {
                warnings.push("同主播目录存在多个房间号，未自动推断房间身份".into());
            }
        }
    }
    if merged.is_some() && info.is_empty() {
        warnings.push("缺少起始分段 XML，录制时间按文件名解释，时区待确认".into());
    }
    let role = recording_role(path, started_at.is_some());
    RecordingInfo {
        title,
        started_at,
        time_source,
        room_id,
        room_name,
        sidecars,
        warnings,
        role,
    }
}

/// MP4 is a recorder output format too; only processed outputs and unknown MP4
/// files retain the legacy classification.
pub fn recording_role(path: &Path, has_recording_metadata: bool) -> &'static str {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    if path
        .components()
        .any(|p| p.as_os_str().eq_ignore_ascii_case("merged"))
        || MERGED.is_match(&stem)
    {
        "legacy"
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        && !has_recording_metadata
    {
        "legacy"
    } else {
        "source"
    }
}

pub(crate) async fn scan(state: Arc<AppState>) -> Result<()> {
    let old = state.library.read().await.clone();
    let cached: HashMap<_, _> = old
        .assets
        .iter()
        .map(|a| (a.relative_path.clone(), a))
        .collect();
    let mut library = Library {
        root: state.config.library.to_string_lossy().into(),
        ..Default::default()
    };
    let mut rooms: BTreeMap<String, Room> = BTreeMap::new();
    let mut videos = Vec::new();
    // Do not follow symlinks/reparse points outside the authorized library.
    for entry in WalkDir::new(&state.config.library).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                library.errors.push(e.to_string());
                continue;
            }
        };
        let path = entry.path();
        if entry.depth() == 1 && entry.file_type().is_dir() {
            let dir = entry.file_name().to_string_lossy().into_owned();
            let (id, name) = dir.split_once('-').unwrap_or((&dir, &dir));
            let r = rooms.entry(id.into()).or_insert_with(|| Room {
                id: id.into(),
                name: name.into(),
                ..Default::default()
            });
            if !r.aliases.contains(&name.to_string()) {
                r.aliases.push(name.into());
            }
            r.directories.push(dir);
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ["flv", "ts", "mkv", "mp4"].contains(&ext.as_str()) {
            videos.push(path.to_path_buf());
        } else if ["xml", "txt"].contains(&ext.as_str()) {
            library.sidecar_count += 1;
        }
    }
    videos.sort();
    {
        let mut s = state.scan_status.lock().await;
        s.total = videos.len();
        s.message = "探测媒体与读取 XML 历史信息".into();
    }
    for path in videos {
        if state.shutdown.is_cancelled() {
            anyhow::bail!("服务关闭，扫描中断");
        }
        let relative = path
            .strip_prefix(&state.config.library)?
            .to_string_lossy()
            .into_owned();
        let meta = std::fs::metadata(&path)?;
        let modified = media::modified_ms(&meta);
        let first = Path::new(&relative)
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        let (id, rname) = first.split_once('-').unwrap_or((&first, &first));
        let ext = path.extension().unwrap().to_string_lossy().to_lowercase();
        let recording = recording_info(&state.config.library, &path);
        let start = recording.started_at;
        let title = recording.title;
        let time_source = recording.time_source;
        let mut warnings = recording.warnings;
        let sidecars = recording
            .sidecars
            .iter()
            .filter_map(|p| p.strip_prefix(&state.config.library).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let room_id = recording.room_id.unwrap_or(id.into());
        let room_name = recording.room_name.unwrap_or(rname.into());
        let room = rooms.entry(room_id.clone()).or_insert_with(|| Room {
            id: room_id.clone(),
            name: room_name.clone(),
            ..Default::default()
        });
        if !room.aliases.contains(&room_name) {
            room.aliases.push(room_name.clone());
        }
        if !room.directories.contains(&first) {
            room.directories.push(first.clone());
        }
        room.historical_title = Some(title.clone());
        let prev = cached.get(&relative).copied();
        let metadata = if let Some(p) = prev
            .filter(|p| p.bytes == meta.len() && p.modified_ms == modified && p.metadata.is_some())
        {
            p.metadata.clone()
        } else {
            match media::probe(&state.config.ffprobe, &path).await {
                Ok(m) => Some(m),
                Err(e) => {
                    warnings.push(format!("探测失败：{e}"));
                    None
                }
            }
        };
        if metadata.as_ref().and_then(|m| m.duration).is_none() {
            warnings.push("时长未知，暂不能自动合并".into());
        }
        if start.is_none() {
            warnings.push("录制时间未知，暂不能自动归场".into());
        }
        let age = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        if age.saturating_sub(modified) < 60_000 {
            warnings.push("文件最近发生变化，请录制结束后重扫".into());
        }
        library.assets.push(Asset {
            id: digest(&relative),
            relative_path: relative,
            name: path.file_name().unwrap().to_string_lossy().into(),
            room_id,
            room_name,
            title,
            display_title: prev.and_then(|p| p.display_title.clone()),
            bytes: meta.len(),
            modified_ms: modified,
            extension: ext.clone(),
            role: recording.role.into(),
            started_at: start,
            time_source,
            metadata,
            sidecars,
            warnings,
        });
        state.scan_status.lock().await.completed += 1;
    }
    for r in rooms.values_mut() {
        if let Some(p) = old.rooms.iter().find(|p| p.id == r.id) {
            r.online = p.online.clone();
            r.refreshed_at = p.refreshed_at.clone();
            r.refresh_error = p.refresh_error.clone();
        }
    }
    library.rooms = rooms.into_values().collect();
    library.scanned_at = Some(now());
    state.db.put("library", "main", &library)?;
    *state.library.write().await = library;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_legacy_and_reconnect_filenames() {
        let (t, title) = filename_info("录制-123-20230620-235958-100-测试_PART000");
        assert_eq!(t.as_deref(), Some("2023-06-20T23:59:58+08:00"));
        assert_eq!(title, "测试");
        assert_eq!(filename_info("123-20230620-235958-100-测试.flv").1, "测试");
        assert!(filename_info("Yommyko_摸一摸").0.is_none());
    }

    #[test]
    fn parses_bili_live_tools_merged_filename_and_recording_role() {
        let path = Path::new(
            "主播甲/merged/主播甲_20261005_085403_直播_标题_[9x16_1080x1920_h264_High].mp4",
        );
        let info = recording_info(Path::new("."), path);
        assert_eq!(
            info.started_at.as_deref(),
            Some("2026-10-05T08:54:03+08:00")
        );
        assert_eq!(info.title, "直播 标题");
        assert_eq!(info.room_name.as_deref(), Some("主播甲"));
        assert_eq!(info.role, "legacy");
        let direct = Path::new("2026-10-05 08-54-03-075 直播标题.mp4");
        assert_eq!(recording_info(Path::new("."), direct).role, "source");
        assert_eq!(
            recording_info(Path::new("."), Path::new("ordinary.mp4")).role,
            "legacy"
        );
    }

    #[test]
    fn reads_bili_live_tools_xml_and_matches_merged_anchor() {
        let tmp = tempfile::tempdir().unwrap();
        let room = tmp.path().join("主播甲");
        std::fs::create_dir_all(room.join("merged")).unwrap();
        let xml = room.join("2026-10-05 08-54-03-075 直播标题.xml");
        std::fs::write(&xml, r#"<?xml version="1.0"?><i><metadata>
          <video_start_time>1791204846479</video_start_time>
          <live_start_time>1791204818000</live_start_time>
          <room_title>直播 &amp; 标题</room_title><user_name>主播甲</user_name><room_id>22551964</room_id>
          </metadata><RecorderXmlStyle><title>不要读取这个标题</title></RecorderXmlStyle></i>"#).unwrap();
        let video = room.join("merged/主播甲_20261005_085403_直播标题_[9x16].mp4");
        std::fs::write(&video, b"fake video").unwrap();
        let info = recording_info(tmp.path(), &video);
        assert_eq!(info.title, "直播 & 标题");
        assert_eq!(info.room_id.as_deref(), Some("22551964"));
        assert_eq!(info.room_name.as_deref(), Some("主播甲"));
        assert_eq!(
            info.started_at.as_deref(),
            Some("2026-10-05T20:54:06.479+08:00")
        );
        assert_eq!(info.time_source, "xml:video_start_time");
        assert_eq!(info.sidecars, vec![xml]);
        assert!(info.warnings.iter().any(|w| w.contains("弹幕尚未合并")));
        assert!(!info.warnings.iter().any(|w| w.contains("时间与文件名冲突")));
    }

    #[test]
    fn recorder_xml_keeps_legacy_attributes_and_rejects_invalid_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let xml = tmp.path().join("legacy.xml");
        std::fs::write(&xml, r#"<i><BililiveRecorderRecordInfo roomid="123" name="主播" title="A &amp; B" start_time="2023-06-20T23:59:58+08:00" /></i>"#).unwrap();
        let info = xml_info(&xml);
        assert_eq!(info.get("title").map(String::as_str), Some("A & B"));
        assert_eq!(info.get("roomid").map(String::as_str), Some("123"));
        std::fs::write(&xml, "<i><metadata><video_start_time>invalid</video_start_time><room_title>标题</room_title></metadata></i>").unwrap();
        let info = xml_info(&xml);
        assert!(!info.contains_key("start_time"));
        assert_eq!(info.get("title").map(String::as_str), Some("标题"));
    }

    #[test]
    fn missing_merged_xml_only_infers_a_consistent_room_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let room = tmp.path().join("主播甲");
        std::fs::create_dir_all(room.join("merged")).unwrap();
        let old_xml = room.join("2026-10-05 08-54-03-075 旧标题.xml");
        std::fs::write(&old_xml, "<i><metadata><video_start_time>1791204846479</video_start_time><room_title>旧标题</room_title><user_name>主播甲</user_name><room_id>123</room_id></metadata></i>").unwrap();
        let video = room.join("merged/主播甲_20261006_094931_新标题_[9x16].mp4");
        let info = recording_info(tmp.path(), &video);
        assert_eq!(info.room_id.as_deref(), Some("123"));
        assert_eq!(info.title, "新标题");
        assert_eq!(
            info.started_at.as_deref(),
            Some("2026-10-06T09:49:31+08:00")
        );
        assert!(info.sidecars.is_empty());
        assert!(info.warnings.iter().any(|w| w.contains("时区待确认")));
        std::fs::write(
            room.join("other.xml"),
            "<i><metadata><user_name>主播甲</user_name><room_id>456</room_id></metadata></i>",
        )
        .unwrap();
        assert!(recording_info(tmp.path(), &video).room_id.is_none());
    }
}
