use crate::{media, model::*, AppState};
use anyhow::{bail, Context, Result};
use std::{path::Path, process::Stdio, sync::Arc};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

fn save(state: &AppState, job: &mut Job) -> Result<()> {
    job.updated_at = now();
    state.db.put("job", &job.id, job)
}

pub async fn execute(state: Arc<AppState>, plan: Plan, mut job: Job, cancel: CancellationToken) {
    job.status = "running".into();
    job.message = "核对输入文件与输出空间".into();
    let result=async {
        save(&state,&mut job)?;
        // Validate every input before producing any output.
        for o in &plan.outputs {for a in &o.inputs {
            let p=media::resolve_input(&state.config.library,&a.relative_path)?;
            let m=std::fs::metadata(p)?;
            if m.len()!=a.bytes||media::modified_ms(&m)!=a.modified_ms {bail!("源文件已变化，请重新扫描和生成计划：{}",a.name);}
        }}
        let mut needed=0u64;
        for o in &plan.outputs {if state.db.get::<Artifact>("artifact",&o.id)?.is_none(){needed=needed.saturating_add((o.bytes as f64*1.03) as u64);}}
        if fs2::available_space(&state.config.output)?<needed+1024*1024*1024 {bail!("输出盘空间不足：需要额外约 {:.2} GiB，并保留 1 GiB",needed as f64/1073741824.0);}
        for (index,o) in plan.outputs.iter().enumerate() {
            if cancel.is_cancelled()||state.shutdown.is_cancelled(){bail!("任务已取消");}
            let target_dir=state.config.output.join(&o.id);
            std::fs::create_dir_all(&target_dir)?;
            let target=target_dir.join(&o.name);
            if let Some(saved)=state.db.get::<Artifact>("artifact",&o.id)? {
                let meta=std::fs::metadata(&saved.path).context("已登记成品缺失，请检查输出目录")?;
                if Path::new(&saved.path)!=target||meta.len()!=saved.bytes {bail!("已登记成品发生变化，停止自动跳过");}
                let m=media::probe(&state.config.ffprobe,&target).await?;
                if m.duration.map(|d|(d-saved.duration).abs()>1.0).unwrap_or(true){bail!("已登记成品时长不符");}
                job.completed_outputs.push(saved);
                job.progress=(index+1) as f64/plan.outputs.len() as f64;
                job.message=format!("已验证已有成品：{}",o.name);save(&state,&mut job)?;
                continue;
            }
            if target.exists(){bail!("目标存在但没有完成记录，保留文件并停止：{}",target.display());}
            let temporary=target_dir.join(format!(".building-{}.mp4",job.id));
            let list=target_dir.join(format!(".concat-{}.txt",job.id));
            let log_path=target_dir.join(format!("{}.ffmpeg.log",job.id));
            let mut contents=String::new();
            for a in &o.inputs {contents.push_str(&media::concat_line(&media::resolve_input(&state.config.library,&a.relative_path)?)?);}
            std::fs::write(&list,contents)?;
            let log=std::fs::File::create(&log_path)?;
            let mut command=media::command(&state.config.ffmpeg);
            command.args(["-hide_banner","-nostdin","-v","warning"]);
            if let Some(offset)=o.cut_start {
                let input=media::resolve_input(&state.config.library,&o.inputs[0].relative_path)?;
                let audio_count=o.inputs[0].metadata.as_ref().unwrap().streams.iter().filter(|s|s["codec_type"]=="audio").count() as u64;
                let budget=(plan.request.max_bytes as f64*8.0*0.85/o.duration) as u64;
                let video_rate=budget.saturating_sub(audio_count*192_000).min(20_000_000);
                if video_rate<100_000 {bail!("体积预算不足以编码音视频，请提高预算");}
                command.args(["-ss",&format!("{offset:.6}"),"-noautorotate","-i"]).arg(input)
                    .args(["-t",&format!("{:.6}",o.duration),"-map","0:v:0","-map","0:a?","-c:v","libx264","-preset","fast","-crf","20","-maxrate",&video_rate.to_string(),"-bufsize",&(video_rate*2).to_string(),"-pix_fmt","yuv420p","-c:a","aac","-b:a","192k"]);
            } else {
                command.args(["-fflags","+genpts","-f","concat","-safe","0","-i"]).arg(&list).args(["-map","0:v:0","-map","0:a?","-c","copy"]);
            }
            let mut child=command.args(["-avoid_negative_ts","make_zero","-progress","pipe:1","-nostats","-n"]).arg(&temporary)
                .stdout(Stdio::piped()).stderr(Stdio::from(log)).spawn().context("无法启动 FFmpeg")?;
            let mut lines=BufReader::new(child.stdout.take().unwrap()).lines();
            job.message=format!("合并 {}/{}：{}",index+1,plan.outputs.len(),o.name);save(&state,&mut job)?;
            let process_result:Result<()>=async {
                loop {
                    tokio::select! {
                        _=cancel.cancelled()=>{child.kill().await?;let _=child.wait().await;bail!("任务已取消");}
                        _=state.shutdown.cancelled()=>{child.kill().await?;let _=child.wait().await;bail!("服务关闭，任务中断");}
                        line=lines.next_line()=>{
                            match line? {
                                Some(line)=>{if let Some(us)=line.strip_prefix("out_time_us=").and_then(|v|v.parse::<f64>().ok()) {
                                    job.progress=(index as f64+(us/1_000_000.0/o.duration).clamp(0.0,0.98))/plan.outputs.len() as f64;save(&state,&mut job)?;
                                }}
                                None=>break,
                            }
                        }
                    }
                }
                let status=child.wait().await?;
                if !status.success(){bail!("FFmpeg 失败，日志：{}",log_path.display());}
                Ok(())
            }.await;
            let _=std::fs::remove_file(&list);
            if let Err(e)=process_result {let _=std::fs::remove_file(&temporary);return Err(e);}
            job.message=format!("验证 {}/{}：媒体参数与连接处解码",index+1,plan.outputs.len());save(&state,&mut job)?;
            let validation=validate(&state,&temporary,o,&plan.request,&cancel).await;
            let m=match validation {Ok(m)=>m,Err(e)=>{let _=std::fs::remove_file(&temporary);return Err(e);}};
            let bytes=std::fs::metadata(&temporary)?.len();
            // Same-filesystem no-clobber publication: hard_link fails if destination exists.
            std::fs::hard_link(&temporary,&target).context("无法发布成品（目标可能已存在或文件系统不支持硬链接）")?;
            std::fs::remove_file(&temporary)?;
            let artifact=Artifact{output_id:o.id.clone(),path:target.to_string_lossy().into(),bytes,duration:m.duration.unwrap(),validation:"流参数 + 时长/体积 + 连接处抽样解码（非全片解码）".into(),source_ids:o.inputs.iter().map(|a|a.id.clone()).collect()};
            state.db.put("artifact",&o.id,&artifact)?;
            job.completed_outputs.push(artifact);job.progress=(index+1) as f64/plan.outputs.len() as f64;save(&state,&mut job)?;
        }
        Ok::<_,anyhow::Error>(())
    }.await;
    match result {
        Ok(()) => {
            job.status = "completed".into();
            job.progress = 1.0;
            job.message = format!(
                "完成 {} 个成品，原始素材已保留",
                job.completed_outputs.len()
            );
        }
        Err(e) => {
            job.status = if state.shutdown.is_cancelled() {
                "interrupted"
            } else if cancel.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            job.message = format!("{e:#}");
        }
    }
    if let Err(e) = save(&state, &mut job) {
        eprintln!("保存任务状态失败：{e}");
    }
    state.cancellations.lock().await.remove(&job.id);
}

async fn validate(
    state: &AppState,
    path: &Path,
    o: &PlannedOutput,
    limits: &PlanRequest,
    cancel: &CancellationToken,
) -> Result<MediaInfo> {
    let m = media::probe(&state.config.ffprobe, path).await?;
    let d = m.duration.context("输出时长未知")?;
    let expected = o.inputs[0].metadata.as_ref().unwrap();
    if d > limits.max_duration || d > 43200.0 || std::fs::metadata(path)?.len() > limits.max_bytes {
        bail!("输出超过时长或体积限制，未登记成品");
    }
    if (d - o.duration).abs() > 2.0f64.max(o.duration * 0.005) {
        bail!("输出时长与输入总时长不符");
    }
    if m.width != expected.width
        || m.height != expected.height
        || m.codec
            != if o.cut_start.is_some() {
                "h264"
            } else {
                &expected.codec
            }
    {
        bail!("输出视频参数不符");
    }
    let expected_audio = expected
        .streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .count();
    let actual_audio = m
        .streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .count();
    if actual_audio != expected_audio {
        bail!("输出音轨数量不符");
    }
    let mut boundaries = vec![0.0, (d - 2.0).max(0.0)];
    let mut t = 0.0;
    for a in &o.inputs {
        t += a.metadata.as_ref().unwrap().duration.unwrap();
        if t < d {
            boundaries.push((t - 1.0).max(0.0));
        }
    }
    for offset in boundaries {
        if cancel.is_cancelled() || state.shutdown.is_cancelled() {
            bail!("验证已中断");
        }
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            media::command(&state.config.ffmpeg)
                .args([
                    "-hide_banner",
                    "-nostdin",
                    "-v",
                    "error",
                    "-xerror",
                    "-ss",
                    &format!("{offset:.3}"),
                    "-i",
                ])
                .arg(path)
                .args(["-t", "2", "-f", "null", "-"])
                .output(),
        )
        .await
        .context("抽样解码超时")??;
        if !output.status.success() {
            bail!(
                "连接处抽样解码失败：{}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(300)
                    .collect::<String>()
            );
        }
    }
    Ok(m)
}
