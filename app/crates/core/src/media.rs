use crate::model::MediaInfo;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

pub fn command(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    c.kill_on_drop(true).stdin(Stdio::null());
    #[cfg(windows)]
    c.creation_flags(0x08000000);
    c
}

pub async fn probe(bin: &Path, file: &Path) -> Result<MediaInfo> {
    let result = tokio::time::timeout(Duration::from_secs(30),command(bin).args([
        "-v","error","-show_data_hash","sha256","-show_entries",
        "format=duration:stream=codec_type,codec_name,profile,width,height,pix_fmt,sample_aspect_ratio,display_aspect_ratio,r_frame_rate,time_base,sample_rate,channels,channel_layout,extradata_hash:stream_side_data=rotation",
        "-of","json"
    ]).arg(file).output()).await.context("媒体探测超时")??;
    if !result.status.success() {
        bail!(
            "{}",
            String::from_utf8_lossy(&result.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        );
    }
    parse_probe(&serde_json::from_slice(&result.stdout)?)
}

pub fn parse_probe(v: &Value) -> Result<MediaInfo> {
    let streams = v["streams"]
        .as_array()
        .ok_or_else(|| anyhow!("没有媒体流"))?
        .clone();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| anyhow!("没有视频流"))?;
    let duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);
    let width = video["width"].as_u64().unwrap_or(0) as u32;
    let height = video["height"].as_u64().unwrap_or(0) as u32;
    let ratio = video["display_aspect_ratio"].as_str().unwrap_or("");
    let aspect = if !ratio.is_empty() && ratio != "0:1" {
        ratio.to_string()
    } else if width > height {
        "横屏".into()
    } else {
        "竖屏".into()
    };
    Ok(MediaInfo {
        duration,
        width,
        height,
        aspect,
        codec: video["codec_name"].as_str().unwrap_or("unknown").into(),
        signature: crate::model::digest(serde_json::to_vec(&streams)?),
        streams,
    })
}

pub fn resolve_input(root: &Path, relative: &str) -> Result<PathBuf> {
    let p = root.join(relative).canonicalize().context("源文件不存在")?;
    if !p.starts_with(root) {
        bail!("路径超出素材库");
    }
    Ok(p)
}

pub fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn concat_line(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow!("路径不能编码为 UTF-8"))?;
    if text.contains(['\n', '\r', '\0']) {
        bail!("路径包含不能用于 concat 的控制字符");
    }
    let text = text
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
        .replace('\'', "'\\''");
    Ok(format!("file '{text}'\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_duration_stays_unknown() {
        let m=parse_probe(&serde_json::json!({"format":{"duration":"N/A"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080}]})).unwrap();
        assert_eq!(m.duration, None);
    }
    #[test]
    fn concat_escapes_quotes_and_rejects_lines() {
        assert_eq!(
            concat_line(Path::new("C:/主播/it's.flv")).unwrap(),
            "file 'C:/主播/it'\\''s.flv'\n"
        );
        assert!(concat_line(Path::new("a\nb.flv")).is_err());
    }
}
