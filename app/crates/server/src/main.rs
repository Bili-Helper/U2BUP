#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = u2bup_core::Config::from_args(std::env::args().skip(1))?;
    let headless = cfg.headless;
    let mut running = u2bup_core::start(cfg).await?;
    println!("U2BUP ready: {}", running.launch_url);
    if headless {
        println!("Headless preview: 本地媒体处理可用；YouTube 授权和上传暂不支持。登录链接含本次运行的访问令牌，请勿公开日志。");
    }
    tokio::select! {
        result = shutdown_signal() => result?,
        result = &mut running.task => { result??; return Ok(()); }
    }
    running.shutdown.cancel();
    running.task.await??;
    Ok(())
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
