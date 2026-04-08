//! fasm-engine binary — delegates entirely to the library.

use fasm_engine::config;
use fasm_engine::engine;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "fasm-engine", about = "FASM Function-as-a-Service engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the FaaS engine with the given config file.
    Serve {
        /// Path to `engine.toml`.
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { config } => {
            let config_path = config.canonicalize().expect("config path not found");
            let config_dir  = config_path.parent().unwrap().to_path_buf();

            let cfg = match config::load(&config_path) {
                Ok(c) => c,
                Err(e) => { eprintln!("Error loading config: {}", e); std::process::exit(1); }
            };

            tracing::info!(config = ?config_path, "loaded engine config");

            if let Err(e) = engine::run(cfg, config_dir).await {
                eprintln!("Engine error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
