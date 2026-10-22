use clap::Parser;
use std::path::PathBuf;
use telex_watcher::{management::Command, Runtime, RuntimeConfig};

#[derive(Parser)]
#[command(version, about = "Telex Watcher v2 runtime host")]
struct Args {
    #[arg(long, env = "TELEX_WATCHER_REGISTRY")]
    registry: PathBuf,
    #[arg(long, env = "TELEX_INSTALL_ROOT")]
    install_root: PathBuf,
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut config = RuntimeConfig::installed(args.registry, args.install_root);
    config.concurrency = args.concurrency;
    let (runtime, management) = Runtime::open(config)?;
    let signal = tokio::spawn(async move {
        shutdown_signal().await?;
        management.execute(Command::Shutdown).await?;
        Ok::<_, anyhow::Error>(())
    });
    let result = runtime.run().await;
    signal.abort();
    match signal.await {
        Ok(result) => result?,
        Err(error) if error.is_cancelled() => {}
        Err(error) => return Err(error.into()),
    }
    result
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
