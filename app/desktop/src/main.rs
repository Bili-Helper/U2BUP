#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

fn main() {
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("创建后台运行时失败"));
    let server: Arc<Mutex<Option<u2bup_core::Running>>> = Arc::new(Mutex::new(None));
    let server_for_setup = server.clone();
    let runtime_for_setup = runtime.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let mut cfg = u2bup_core::Config::from_args(std::env::args().skip(1))?;
            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;
            let root_file = app_data.join("library-root.txt");
            if !cfg.library.exists() {
                if let Ok(root) = std::fs::read_to_string(&root_file) {
                    cfg.library = root.trim().into();
                }
            }
            if !cfg.library.exists() {
                let Some(folder) = app
                    .dialog()
                    .file()
                    .set_title("选择录播素材根目录（例如 LiveRec）")
                    .blocking_pick_folder()
                else {
                    return Err(std::io::Error::other("未选择素材目录").into());
                };
                cfg.library = folder.into_path()?;
            }
            std::fs::write(&root_file, cfg.library.to_string_lossy().as_bytes())?;
            cfg.data = app_data.join("data");
            cfg.output = app_data.join("exports");
            cfg.port = 0;
            let resources = app.path().resource_dir()?.join("resources");
            let extension = if cfg!(windows) { ".exe" } else { "" };
            let ffmpeg = resources.join(format!("ffmpeg{extension}"));
            let ffprobe = resources.join(format!("ffprobe{extension}"));
            if ffmpeg.exists() && ffprobe.exists() {
                cfg.ffmpeg = ffmpeg;
                cfg.ffprobe = ffprobe;
            }
            #[cfg(not(debug_assertions))]
            if !cfg.ffmpeg.is_absolute() {
                return Err(std::io::Error::other(
                    "发行包缺少内置 FFmpeg/FFprobe，请使用完整安装包",
                )
                .into());
            }
            let running = runtime_for_setup.block_on(u2bup_core::start(cfg))?;
            let url = running.launch_url.parse()?;
            *server_for_setup.lock().unwrap() = Some(running);
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title("U2BUP · 录播工作台")
                .inner_size(1440.0, 940.0)
                .min_inner_size(840.0, 620.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("U2BUP 启动失败");
    if let Some(running) = server.lock().unwrap().take() {
        running.shutdown.cancel();
        let _ = runtime.block_on(running.task);
    };
}
