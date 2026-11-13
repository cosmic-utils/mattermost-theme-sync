// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::cosmic::Mode;

#[derive(Debug, Parser)]
#[command(
    name = "cosmicmost",
    version,
    about = "Sync the COSMIC desktop theme to Mattermost",
    long_about = "Read the active COSMIC theme and publish it as a Mattermost custom theme \
                  for your account. Configuration and state are stored with cosmic-config."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Configure a Mattermost server and store its personal access token in the keychain.
    Init(InitArgs),
    /// Print the generated Mattermost theme without contacting a server.
    Preview(PreviewArgs),
    /// Apply the theme once.
    Apply(ApplyArgs),
    /// Watch COSMIC and keep Mattermost in sync.
    Daemon,
    /// Install and enable the systemd user service.
    InstallService(InstallServiceArgs),
    /// Remove the applied theme from Mattermost.
    Reset(ResetArgs),
    /// Show configuration paths and synchronization state.
    Status,
    /// Print the config, state, and service paths.
    Paths,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Mattermost server URL.
    #[arg(long)]
    pub url: Option<String>,

    /// Only set the global theme; do not write a theme for each team.
    #[arg(long)]
    pub no_sync_all_teams: bool,

    /// Do not validate the token against the server before saving.
    #[arg(long)]
    pub skip_validation: bool,

    /// Replace the existing server list instead of adding or updating a server.
    #[arg(long)]
    pub replace: bool,
}

#[derive(Debug, Args)]
pub struct PreviewArgs {
    /// Build the dark or light theme instead of the active one.
    #[arg(long, value_enum)]
    pub mode: Option<ModeArg>,

    /// Write the theme JSON to a file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// Print what would be sent without changing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Apply the theme even when the generated hash matches the last run.
    #[arg(long)]
    pub force: bool,

    /// Build the dark or light theme instead of the active one.
    #[arg(long, value_enum)]
    pub mode: Option<ModeArg>,
}

#[derive(Debug, Args)]
pub struct ResetArgs {
    /// Also remove per-team theme preferences.
    #[arg(long)]
    pub all_teams: bool,

    /// Forget the stored synchronization state.
    #[arg(long)]
    pub forget: bool,

    /// Print what would be removed without changing anything.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct InstallServiceArgs {
    /// Write the unit file but do not enable it.
    #[arg(long)]
    pub no_enable: bool,

    /// Enable the unit but do not start it now.
    #[arg(long)]
    pub no_start: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ModeArg {
    Dark,
    Light,
}

impl From<ModeArg> for Mode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Dark => Mode::Dark,
            ModeArg::Light => Mode::Light,
        }
    }
}
