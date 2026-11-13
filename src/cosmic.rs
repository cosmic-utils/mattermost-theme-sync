// SPDX-License-Identifier: GPL-3.0-only

// Read COSMIC theme/mode and set up watchers for the daemon.

use anyhow::{Result, anyhow};
use cosmic_config::CosmicConfigEntry;
use cosmic_theme::{Theme, ThemeBuilder, ThemeMode};
use notify::RecommendedWatcher;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;

use crate::config::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// Read the theme COSMIC currently considers active.
pub fn read_active() -> Result<Theme> {
    Theme::get_active().map_err(|(errors, _)| {
        anyhow!(
            "cannot read the active COSMIC theme: {}",
            format_errors(&errors)
        )
    })
}

/// Read a specific mode, or the active theme when `mode` is `None`.
pub fn read(mode: Option<Mode>) -> Result<Theme> {
    match mode {
        None => read_active(),
        Some(Mode::Dark) => read_config(Theme::dark_config()?),
        Some(Mode::Light) => read_config(Theme::light_config()?),
    }
}

fn read_config(config: cosmic_config::Config) -> Result<Theme> {
    Theme::get_entry(&config).map_err(|(errors, _)| {
        anyhow!(
            "cannot read the COSMIC theme at {}: {}",
            config
                .user_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_owned()),
            format_errors(&errors)
        )
    })
}

fn format_errors(errors: &[cosmic_config::Error]) -> String {
    if errors.is_empty() {
        "unknown error".to_owned()
    } else {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Watch the COSMIC mode, the dark and light themes, and cosmicmost's own
/// configuration. Sends a message on the channel whenever anything changes.
///
/// The returned watchers must be kept alive for as long as the daemon runs.
pub fn watch_all(tx: UnboundedSender<()>) -> Result<Vec<RecommendedWatcher>> {
    let mut watchers = Vec::new();

    add_watcher(&mut watchers, &tx, ThemeMode::config()?, "theme mode")?;
    add_watcher(&mut watchers, &tx, Theme::dark_config()?, "dark theme")?;
    add_watcher(&mut watchers, &tx, Theme::light_config()?, "light theme")?;
    add_watcher(
        &mut watchers,
        &tx,
        ThemeBuilder::dark_config()?,
        "dark theme builder",
    )?;
    add_watcher(
        &mut watchers,
        &tx,
        ThemeBuilder::light_config()?,
        "light theme builder",
    )?;
    add_watcher(&mut watchers, &tx, Settings::config()?, "cosmicmost config")?;

    Ok(watchers)
}

fn add_watcher(
    watchers: &mut Vec<RecommendedWatcher>,
    tx: &UnboundedSender<()>,
    config: cosmic_config::Config,
    name: &'static str,
) -> Result<()> {
    let sender = tx.clone();
    let watcher = config.watch(move |_config, keys| {
        debug!("{name} changed: {keys:?}");
        let _ = sender.send(());
    })?;
    watchers.push(watcher);
    Ok(())
}
