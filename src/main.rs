use clap::{Parser, Subcommand};
use hireferry::{AppState, config::Config};
#[derive(Parser)]
#[command(version, about = "HireFerry · 聘渡 — one small Rust service")]
struct Cli {
    #[arg(long, global = true, default_value = "config.toml")]
    config: std::path::PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Serve,
    AdminBootstrap,
    CheckConfig,
    Backup { path: std::path::PathBuf },
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    if matches!(cli.command, Some(Command::CheckConfig)) {
        println!("Configuration is valid.");
        return Ok(());
    }
    let state = AppState::new(cfg)?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::AdminBootstrap => {
            let password = hireferry::auth::generate_bootstrap(&*state.conn()?)?;
            println!("{password}");
        }
        Command::Backup { path } => {
            state.conn()?.backup("main", &path, None)?;
            println!("Backup written to {}", path.display());
        }
        Command::CheckConfig => unreachable!(),
        Command::Serve => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "hireferry=info".into()),
                )
                .init();
            let listener = tokio::net::TcpListener::bind(&state.cfg.bind).await?;
            let webhook_worker = hireferry::webhooks::spawn_worker(state.clone());
            tracing::info!(address=%state.cfg.bind,"HireFerry is ready");
            axum::serve(listener, hireferry::app(state))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
            webhook_worker.abort();
        }
    }
    Ok(())
}
