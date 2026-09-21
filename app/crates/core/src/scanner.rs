use crate::{media, model::*, AppState};
use anyhow::Result;
use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use quick_xml::{events::Event, Reader};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, LazyLock},
};
use walkdir::WalkDir;

static RECORDING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:录制-)?(\d+)-(\d{8})-(\d{6})-\d+-(.*)$").unwrap());
static ALTERNATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}-\d{2}-\d{2})-\d+\s+(.*)$").unwrap());
static PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)_PART\d+$").unwrap());

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
            Ok(Event::Eof) | Err(_) => return HashMap::new(),
            _ => {}
        }
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
        let stem = path.file_stem().unwrap().to_string_lossy();
        let (mut start, mut title) = filename_info(&stem);
        let mut time_source = if start.is_some() {
            "filename:Asia/Shanghai"
        } else {
            "unknown"
        }
        .to_string();
        let mut warnings = Vec::new();
        let mut sidecars = Vec::new();
        let mut info = HashMap::new();
        for extension in ["xml", "txt"] {
            let mut candidates = vec![path.with_extension(extension)];
            if ext == "mp4" {
                candidates.push(path.with_file_name(format!(
                    "录制-{}.{}",
                    stem.strip_suffix(".flv").unwrap_or(&stem),
                    extension
                )));
            }
            for candidate in candidates {
                if candidate.is_file() {
                    sidecars.push(
                        candidate
                            .strip_prefix(&state.config.library)?
                            .to_string_lossy()
                            .into_owned(),
                    );
                    if extension == "xml" {
                        info = xml_info(&candidate);
                    }
                    break;
                }
            }
        }
        if let Some(xml_time) = info
            .get("start_time")
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        {
            if let Some(file_time) = start
                .as_ref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            {
                if (xml_time - file_time).num_seconds().abs() > 60 {
                    warnings.push("XML 时间与文件名冲突，当前使用 XML".into());
                }
            }
            start = Some(xml_time.to_rfc3339());
            time_source = "xml:start_time".into();
        }
        if let Some(t) = info.get("title").filter(|s| !s.is_empty()) {
            title = PART.replace(t.trim(), "").to_string();
        }
        let room_id = info
            .get("roomid")
            .filter(|s| !s.is_empty() && s.as_str() != "0")
            .cloned()
            .unwrap_or(id.into());
        let room_name = info
            .get("name")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or(rname.into());
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
            role: if ext == "mp4" { "legacy" } else { "source" }.into(),
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
}
