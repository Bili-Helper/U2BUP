use crate::model::*;
use anyhow::{bail, Result};
use std::collections::HashSet;

fn start(a: &Asset) -> f64 {
    a.started_at
        .as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis() as f64 / 1000.0)
        .unwrap_or(0.0)
}
pub fn safe_name(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(90)
        .collect();
    clean.trim().trim_end_matches('.').to_string()
}
pub fn build(library: &Library, req: PlanRequest) -> Result<Plan> {
    if req.asset_ids.is_empty() {
        bail!("请先选择素材");
    }
    if req.asset_ids.len() > 5000 {
        bail!("单次计划最多 5000 个素材");
    }
    if !req.max_duration.is_finite()
        || !(60.0..=43200.0).contains(&req.max_duration)
        || req.max_bytes < 1_000_000
        || req.max_bytes > 256_000_000_000
        || !req.max_gap.is_finite()
        || !(0.0..=86400.0).contains(&req.max_gap)
    {
        bail!("无效规则：时长应在 60–43200 秒，体积不得超过 256 GB");
    }
    let ids: HashSet<_> = req.asset_ids.iter().collect();
    let mut assets: Vec<_> = library
        .assets
        .iter()
        .filter(|a| ids.contains(&a.id))
        .cloned()
        .collect();
    if assets.len() != ids.len() {
        bail!("选择中包含不存在的素材，请刷新列表");
    }
    assets.sort_by(|a, b| {
        a.room_id
            .cmp(&b.room_id)
            .then_with(|| start(a).total_cmp(&start(b)))
            .then(a.id.cmp(&b.id))
    });
    let mut blocked = Vec::new();
    let mut cuts = Vec::new();
    let mut groups: Vec<(Vec<Asset>, String)> = Vec::new();
    let mut last_selected: Option<String> = None;
    for a in assets {
        let d = a.metadata.as_ref().and_then(|m| m.duration);
        let reason = if a.role == "legacy" && !req.include_legacy {
            Some("历史 MP4 来源待确认；开启“包含历史成品”后可单独处理")
        } else if d.is_none() {
            Some("时长未知，需进一步检测")
        } else if a.started_at.is_none() {
            Some("录制时间未知，需先补充时间")
        } else if a.warnings.iter().any(|s| s.contains("最近发生变化")) {
            Some("文件可能仍在写入，请稍后重扫")
        } else {
            None
        };
        if let Some(reason) = reason {
            blocked.push(BlockedAsset {
                asset_id: a.id.clone(),
                name: a.name.clone(),
                reason: reason.into(),
            });
            last_selected = None;
            continue;
        }
        if d.unwrap() > req.max_duration - 1.0 || a.bytes as f64 * 1.02 > req.max_bytes as f64 {
            let duration = d.unwrap();
            let seconds = (req.max_duration - 2.0)
                .min(duration * req.max_bytes as f64 / (a.bytes.max(1) as f64 * 1.3));
            if seconds < 1.0 || (duration / seconds).ceil() > 5000.0 {
                blocked.push(BlockedAsset {
                    asset_id: a.id.clone(),
                    name: a.name.clone(),
                    reason: "切割预算过小或片数超过 5000，请提高单片预算".into(),
                });
            } else {
                let mut offset = 0.0;
                while offset < duration - 0.001 {
                    let length = seconds.min(duration - offset);
                    cuts.push((a.clone(), offset, length));
                    offset += length;
                }
            }
            last_selected = None;
            continue;
        }
        let mut why = "首个片段 / 新直播间".to_string();
        let can_join = if let Some((g, _)) = groups.last() {
            let p = g.last().unwrap();
            let gap = start(&a) - start(p) - p.metadata.as_ref().unwrap().duration.unwrap();
            if last_selected.as_ref() != Some(&p.id) || a.room_id != p.room_id {
                false
            } else if a.role != p.role || a.role == "legacy" {
                why = "历史成品独立处理，避免混入原片".into();
                false
            } else if a.title != p.title {
                why = "标题变化".into();
                false
            } else if gap < -1.0 {
                why = "片段存在时间重叠，独立输出供检查".into();
                false
            } else if gap > req.max_gap {
                why = "超出场次间隔".into();
                false
            } else if a.metadata.as_ref().unwrap().signature
                != p.metadata.as_ref().unwrap().signature
            {
                why = "画幅或音视频流参数变化".into();
                false
            }
            // A selection must not silently bridge unselected footage of another configuration.
            else if library.assets.iter().any(|x| {
                x.room_id == a.room_id
                    && x.role == "source"
                    && x.id != p.id
                    && x.id != a.id
                    && start(x) > start(p)
                    && start(x) < start(&a)
            }) {
                why = "中间有未纳入该连续段的素材".into();
                false
            } else if g
                .iter()
                .map(|x| x.metadata.as_ref().unwrap().duration.unwrap())
                .sum::<f64>()
                + d.unwrap()
                > req.max_duration
            {
                why = "达到时长上限".into();
                false
            } else if (g.iter().map(|x| x.bytes).sum::<u64>() + a.bytes) as f64 * 1.02
                > req.max_bytes as f64
            {
                why = "达到体积预算（含 2% 余量）".into();
                false
            } else {
                true
            }
        } else {
            false
        };
        last_selected = Some(a.id.clone());
        if can_join {
            groups.last_mut().unwrap().0.push(a);
        } else {
            groups.push((vec![a], why));
        }
    }
    let mut outputs = Vec::new();
    for (g, reason) in groups {
        let first = &g[0];
        let m = first.metadata.as_ref().unwrap();
        let fingerprint = serde_json::to_vec(&(
            &req.max_duration,
            &req.max_bytes,
            g.iter()
                .map(|a| {
                    (
                        &a.id,
                        a.bytes,
                        a.modified_ms,
                        &a.metadata.as_ref().unwrap().signature,
                    )
                })
                .collect::<Vec<_>>(),
        ))?;
        let id = digest(fingerprint);
        let date = chrono::DateTime::parse_from_rfc3339(first.started_at.as_ref().unwrap())?
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y%m%d_%H%M%S")
            .to_string();
        let name = format!(
            "{}_{}_{}_{}_{}.mp4",
            safe_name(&first.room_name),
            date,
            safe_name(&first.title),
            safe_name(&m.aspect),
            &id[..8]
        );
        outputs.push(PlannedOutput {
            id,
            name,
            room_id: first.room_id.clone(),
            room_name: first.room_name.clone(),
            title: first.title.clone(),
            aspect: m.aspect.clone(),
            duration: g
                .iter()
                .map(|a| a.metadata.as_ref().unwrap().duration.unwrap())
                .sum(),
            bytes: g.iter().map(|a| a.bytes).sum(),
            inputs: g,
            reason,
            cut_start: None,
        });
    }
    for (a, offset, length) in cuts {
        let id = digest(serde_json::to_vec(&(
            "cut-h264-v1",
            &a.id,
            a.modified_ms,
            a.bytes,
            offset,
            length,
            req.max_bytes,
        ))?);
        let name = format!(
            "{}_part_{:06}_{}.mp4",
            safe_name(&a.name),
            offset as u64,
            &id[..8]
        );
        outputs.push(PlannedOutput {
            id,
            name,
            room_id: a.room_id.clone(),
            room_name: a.room_name.clone(),
            title: a.title.clone(),
            aspect: a.metadata.as_ref().unwrap().aspect.clone(),
            duration: length,
            bytes: (a.bytes as f64 * length / a.metadata.as_ref().unwrap().duration.unwrap() * 1.25)
                as u64,
            inputs: vec![a],
            reason: format!("自动精确切割 · 从 {:.3} 秒开始 · H.264/AAC 转码", offset),
            cut_start: Some(offset),
        });
    }
    outputs.sort_by(|a, b| {
        a.room_id.cmp(&b.room_id).then_with(|| {
            (start(&a.inputs[0]) + a.cut_start.unwrap_or(0.0))
                .total_cmp(&(start(&b.inputs[0]) + b.cut_start.unwrap_or(0.0)))
        })
    });
    let id = digest(serde_json::to_vec(&(&req, &outputs, &blocked))?);
    let estimated_bytes = (outputs.iter().map(|o| o.bytes).sum::<u64>() as f64 * 1.02) as u64;
    Ok(Plan {
        id,
        created_at: now(),
        request: req,
        outputs,
        blocked,
        estimated_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn asset(id: &str, start: i64, duration: f64, sig: &str) -> Asset {
        Asset {
            id: id.into(),
            room_id: "1".into(),
            title: "标题".into(),
            role: "source".into(),
            started_at: Some(
                chrono::DateTime::from_timestamp(start, 0)
                    .unwrap()
                    .to_rfc3339(),
            ),
            bytes: 100,
            metadata: Some(MediaInfo {
                duration: Some(duration),
                signature: sig.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn req(assets: &[Asset]) -> PlanRequest {
        PlanRequest {
            asset_ids: assets.iter().map(|a| a.id.clone()).collect(),
            max_duration: 43200.0,
            max_gap: 1800.0,
            max_bytes: 250_000_000_000,
            include_legacy: false,
        }
    }
    #[test]
    fn aspect_switch_preserves_order() {
        let a = vec![
            asset("a", 0, 100.0, "wide"),
            asset("b", 100, 100.0, "tall"),
            asset("c", 200, 100.0, "wide"),
        ];
        let p = build(
            &Library {
                assets: a.clone(),
                ..Default::default()
            },
            req(&a),
        )
        .unwrap();
        assert_eq!(p.outputs.len(), 3);
    }
    #[test]
    fn doesnt_bridge_unselected_footage() {
        let a = vec![
            asset("a", 0, 100.0, "wide"),
            asset("b", 100, 100.0, "tall"),
            asset("c", 200, 100.0, "wide"),
        ];
        let mut r = req(&a);
        r.asset_ids = vec!["a".into(), "c".into()];
        assert_eq!(
            build(
                &Library {
                    assets: a,
                    ..Default::default()
                },
                r
            )
            .unwrap()
            .outputs
            .len(),
            2
        );
    }
    #[test]
    fn splits_at_limit_and_cuts_oversized_file() {
        let a = vec![
            asset("a", 0, 22000.0, "x"),
            asset("b", 22000, 22000.0, "x"),
            asset("c", 44000, 44000.0, "x"),
        ];
        let p = build(
            &Library {
                assets: a.clone(),
                ..Default::default()
            },
            req(&a),
        )
        .unwrap();
        assert_eq!(p.outputs.len(), 4);
        assert_eq!(p.blocked.len(), 0);
        let cuts: Vec<_> = p.outputs.iter().filter(|o| o.cut_start.is_some()).collect();
        assert_eq!(cuts.len(), 2);
        assert!((cuts.iter().map(|o| o.duration).sum::<f64>() - 44000.0).abs() < 0.001);
        assert_eq!(cuts[1].cut_start, Some(cuts[0].duration));
    }
    #[test]
    fn cross_midnight_and_idempotence() {
        let a = vec![asset("a", 86350, 100.0, "x"), asset("b", 86450, 100.0, "x")];
        let l = Library {
            assets: a.clone(),
            ..Default::default()
        };
        let p = build(&l, req(&a)).unwrap();
        assert_eq!(p.outputs.len(), 1);
        assert_eq!(p.id, build(&l, req(&a)).unwrap().id);
    }
    #[test]
    fn overlap_is_not_silently_concatenated() {
        let a = vec![asset("a", 0, 100.0, "x"), asset("b", 90, 100.0, "x")];
        assert_eq!(
            build(
                &Library {
                    assets: a.clone(),
                    ..Default::default()
                },
                req(&a)
            )
            .unwrap()
            .outputs
            .len(),
            2
        );
    }
}
