// SPDX-License-Identifier: GPL-3.0-only

mod cli;
mod commands;
mod config;
mod cosmic;
mod mapping;
mod mattermost;
mod sync;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,cosmicmost=info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Command::Init(args) => commands::init(args).await,
        Command::Preview(args) => commands::preview(args),
        Command::Apply(args) => commands::apply(args).await,
        Command::Daemon => commands::daemon().await,
        Command::InstallService(args) => commands::install_service(args),
        Command::Reset(args) => commands::reset(args).await,
        Command::Status => commands::status(),
        Command::Paths => commands::paths(),
    }
}
