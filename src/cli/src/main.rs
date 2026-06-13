mod commands;
mod extensions;
mod symbols;
mod writer;

use clap::{Parser, Subcommand};
#[cfg(debug_assertions)]
use commands::playground;
use commands::up;

use indicatif::MultiProgress;
use knot_terminal::TaskEngine;
use std::path::PathBuf;
use symbols::LOGO;
use writer::MultiProgressWriter;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
#[command(name = "knot", version, author, about = "Local process orchestrator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true, default_value = ".")]
    dir: PathBuf,

    #[arg(long, global = true)]
    debug: bool,
}

#[derive(Subcommand)]
enum Commands {
    Up(up::UpArgs),
    #[cfg(debug_assertions)]
    Playground(playground::PgArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let logo = console::style(LOGO).cyan().to_string();
    println!("{}\n", logo);

    let cli = Cli::parse();
    let multi_progress = MultiProgress::new();
    let engine = TaskEngine::with_multi(multi_progress.clone());

    let engine = if cli.debug {
        engine.space();
        let writer = MultiProgressWriter::new(multi_progress);
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .compact()
                    .with_target(false)
                    .with_file(false),
            )
            .init();
        engine
    } else {
        engine
    };

    let directory = std::fs::canonicalize(&cli.dir)?;

    match cli.command {
        Commands::Up(args) => up::execute(args, &directory, &engine).await?,
        #[cfg(debug_assertions)]
        Commands::Playground(args) => playground::execute(args, &directory, &engine).await?,
    }

    Ok(())
}
