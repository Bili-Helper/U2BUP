#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = u2bup_core::Config::from_args(std::env::args().skip(1))?;
    let running = u2bup_core::start(cfg).await?;
    println!("U2BUP ready: {}", running.launch_url);
    tokio::signal::ctrl_c().await?;
    running.shutdown.cancel();
    running.task.await??;
    Ok(())
}
