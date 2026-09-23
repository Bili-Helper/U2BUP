mod db;
mod jobs;
pub mod media;
pub mod model;
pub mod planner;
pub mod scanner;
mod web;
mod workflow;
mod youtube;

use anyhow::{bail, Context, Result};
use db::Db;
use fs2::FileExt;
use model::*;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Config {
    pub library: PathBuf,
    pub data: PathBuf,
    pub output: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub port: u16,
    pub bind: std::net::IpAddr,
    pub public_origin: Option<String>,
    pub headless: bool,
}
impl Config {
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let mut c = Self {
            library: cwd.join("../LiveRec"),
            data: cwd.join(".local"),
            output: cwd.join(".local/exports"),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            port: 4173,
            bind: std::net::Ipv4Addr::LOCALHOST.into(),
            public_origin: None,
            headless: false,
        };
        let mut args = args;
        while let Some(key) = args.next() {
            if key == "--headless" {
                c.headless = true;
                continue;
            }
            let value = args.next().context("参数缺少值")?;
            match key.as_str() {
                "--library" => c.library = value.into(),
                "--data" => c.data = value.into(),
                "--output" => c.output = value.into(),
                "--ffmpeg" => c.ffmpeg = value.into(),
                "--ffprobe" => c.ffprobe = value.into(),
                "--port" => c.port = value.parse()?,
                "--bind" => c.bind = value.parse().context("--bind 必须是 IPv4 或 IPv6 地址")?,
                "--public-origin" => c.public_origin = Some(value),
                _ => bail!("未知参数：{key}"),
            }
        }
        c.validate_network()?;
        Ok(c)
    }

    fn validate_network(&mut self) -> Result<()> {
        if !self.bind.is_loopback() && (!self.headless || self.public_origin.is_none()) {
            bail!("监听非回环地址需要 --headless 和明确的 --public-origin");
        }
        if let Some(value) = &self.public_origin {
            if !self.headless {
                bail!("--public-origin 仅用于 --headless 模式；桌面 OAuth 需要本机回环地址");
            }
            let url = reqwest::Url::parse(value).context("--public-origin 不是有效 URL")?;
            if !["http", "https"].contains(&url.scheme())
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || url.port() == Some(0)
                || matches!(url.host_str(), Some("0.0.0.0" | "[::]"))
            {
                bail!("--public-origin 必须是浏览器访问的 http(s)://主机[:端口]，不能包含路径、凭据、参数或通配地址");
            }
            self.public_origin = Some(url.origin().ascii_serialization());
        }
        Ok(())
    }
}
pub(crate) struct AppState {
    config: Config,
    db: Db,
    library: RwLock<Library>,
    scan_status: Mutex<ScanStatus>,
    operation: Arc<Mutex<()>>,
    cancellations: Mutex<HashMap<String, CancellationToken>>,
    thumbnails: Semaphore,
    token: String,
    authority: String,
    origin: String,
    shutdown: CancellationToken,
    _lock: std::fs::File,
    youtube: youtube::Runtime,
}
pub struct Running {
    pub launch_url: String,
    pub shutdown: CancellationToken,
    pub task: tokio::task::JoinHandle<Result<()>>,
}
pub async fn start(mut config: Config) -> Result<Running> {
    config.validate_network()?;
    config.library = config
        .library
        .canonicalize()
        .context("素材根目录不存在；请使用 --library 指定")?;
    std::fs::create_dir_all(&config.data)?;
    config.data = config.data.canonicalize()?;
    std::fs::create_dir_all(&config.output)?;
    config.output = config.output.canonicalize()?;
    if config.output.starts_with(&config.library) || config.data.starts_with(&config.library) {
        bail!("应用数据和输出目录必须在原始素材库之外");
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config.data.join("instance.lock"))?;
    lock.try_lock_exclusive()
        .context("这个数据目录已有实例正在运行")?;
    let db = Db::open(&config.data.join("u2bup.sqlite3"))?;
    let library = db.get::<Library>("library", "main")?.unwrap_or_default();
    youtube::recover(&db)?;
    workflow::recover(&db)?;
    if !library.root.is_empty() && std::path::Path::new(&library.root) != config.library {
        bail!("当前数据目录属于另一个素材库，请使用独立 --data 目录");
    }
    for mut job in db.list::<Job>("job")? {
        if ["running", "pending"].contains(&job.status.as_str()) {
            job.status = "interrupted".into();
            job.message = "上次服务中断；可重新执行原计划，已验证成品会复用".into();
            job.updated_at = now();
            db.put("job", &job.id, &job)?;
        }
    }
    let listener = tokio::net::TcpListener::bind((config.bind, config.port)).await?;
    let origin = config
        .public_origin
        .clone()
        .unwrap_or_else(|| format!("http://{}", listener.local_addr().expect("bound listener")));
    let authority = origin
        .split_once("://")
        .expect("validated origin")
        .1
        .to_owned();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let launch_url = format!("{origin}/#token={token}");
    let shutdown = CancellationToken::new();
    let state = Arc::new(AppState {
        config: config.clone(),
        db,
        library: RwLock::new(library),
        scan_status: Mutex::new(ScanStatus::default()),
        operation: Arc::new(Mutex::new(())),
        cancellations: Mutex::new(HashMap::new()),
        thumbnails: Semaphore::new(2),
        token,
        authority,
        origin,
        shutdown: shutdown.clone(),
        _lock: lock,
        youtube: youtube::Runtime::default(),
    });
    std::fs::write(
        config.data.join("connection.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"url":launch_url,"pid":std::process::id()}))?,
    )?;
    let router = web::router(state.clone());
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await?;
        state.shutdown.cancel();
        // Drain the active operation so child processes are reaped and the DB is updated.
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(65), state.operation.lock()).await;
        Ok(())
    });
    Ok(Running {
        launch_url,
        shutdown,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_arguments_require_explicit_headless_origin() {
        let parse = |args: &[&str]| Config::from_args(args.iter().map(|value| value.to_string()));
        let defaults = parse(&[]).unwrap();
        assert!(defaults.bind.is_loopback());
        assert!(!defaults.headless);
        assert!(parse(&["--bind", "0.0.0.0"]).is_err());
        assert!(parse(&["--headless", "--bind", "0.0.0.0"]).is_err());
        assert!(parse(&["--public-origin", "http://localhost:4173"]).is_err());
        let config = parse(&[
            "--headless",
            "--bind",
            "0.0.0.0",
            "--public-origin",
            "https://u2bup.example:8443/",
        ])
        .unwrap();
        assert_eq!(
            config.public_origin.as_deref(),
            Some("https://u2bup.example:8443")
        );
        let ipv6 = parse(&["--headless", "--public-origin", "http://[::1]:8080"]).unwrap();
        assert_eq!(ipv6.public_origin.as_deref(), Some("http://[::1]:8080"));
    }

    #[test]
    fn public_origin_rejects_ambiguous_or_credential_bearing_urls() {
        for origin in [
            "ftp://localhost",
            "http://user:secret@localhost",
            "http://localhost/app",
            "http://localhost/?token=value",
            "http://localhost/#token=value",
            "http://0.0.0.0:4173",
            "http://[::]:4173",
            "http://localhost:0",
            "not a url",
        ] {
            assert!(
                Config::from_args(
                    ["--headless", "--public-origin", origin]
                        .map(String::from)
                        .into_iter()
                )
                .is_err(),
                "must reject {origin}"
            );
        }
    }

    #[tokio::test]
    async fn startup_recovers_jobs_and_isolates_sessions() {
        let temporary = tempfile::tempdir().unwrap();
        let library_root = temporary.path().join("library");
        let data_root = temporary.path().join("data");
        std::fs::create_dir_all(&library_root).unwrap();
        std::fs::create_dir_all(&data_root).unwrap();
        {
            let db = Db::open(&data_root.join("u2bup.sqlite3")).unwrap();
            db.put(
                "job",
                "interrupted-job",
                &Job {
                    id: "interrupted-job".into(),
                    plan_id: "test-plan".into(),
                    status: "running".into(),
                    created_at: now(),
                    updated_at: now(),
                    progress: 0.4,
                    message: "working".into(),
                    completed_outputs: vec![],
                },
            )
            .unwrap();
            db.put(
                "library",
                "main",
                &Library {
                    root: library_root
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .into(),
                    assets: vec![Asset {
                        id: "saved-asset".into(),
                        display_title: Some("持久化标题".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let config = Config {
            library: library_root.clone(),
            data: data_root,
            output: temporary.path().join("exports"),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            port: 0,
            bind: std::net::Ipv4Addr::LOCALHOST.into(),
            public_origin: None,
            headless: false,
        };
        let first = start(config.clone()).await.unwrap();
        let url = reqwest::Url::parse(&first.launch_url).unwrap();
        let token = url.fragment().unwrap().strip_prefix("token=").unwrap();
        let base = url.origin().ascii_serialization();
        let client = reqwest::Client::new();
        let snapshot: serde_json::Value = client
            .get(format!("{base}/api/snapshot"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(snapshot["jobs"][0]["status"], "interrupted");
        assert_eq!(
            snapshot["library"]["assets"][0]["display_title"],
            "持久化标题"
        );
        assert!(
            start(config).await.is_err(),
            "same data directory must be locked"
        );
        let cookie1 = client
            .post(format!("{base}/api/session"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        let second = start(Config {
            library: library_root,
            data: temporary.path().join("data2"),
            output: temporary.path().join("exports2"),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            port: 0,
            bind: std::net::Ipv4Addr::LOCALHOST.into(),
            public_origin: None,
            headless: false,
        })
        .await
        .unwrap();
        let u2 = reqwest::Url::parse(&second.launch_url).unwrap();
        let cookie2 = client
            .post(format!("{}/api/session", u2.origin().ascii_serialization()))
            .bearer_auth(u2.fragment().unwrap().strip_prefix("token=").unwrap())
            .send()
            .await
            .unwrap()
            .headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_ne!(cookie1.split('=').next(), cookie2.split('=').next());
        assert!(cookie1.contains("HttpOnly"));
        first.shutdown.cancel();
        second.shutdown.cancel();
        first.task.await.unwrap().unwrap();
        second.task.await.unwrap().unwrap();
    }
}
