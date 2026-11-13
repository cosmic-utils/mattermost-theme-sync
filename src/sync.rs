// SPDX-License-Identifier: GPL-3.0-only

// Rendering + applying the theme, with per-server state and change detection.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::config::{Auth, Server, ServerState, Settings, State};
use crate::cosmic::{self, Mode};
use crate::mapping;
use crate::mattermost::{MattermostClient, Preference};

pub struct RenderedTheme {
    pub theme: cosmic_theme::Theme,
    pub value: String,
    pub hash: String,
}

#[derive(Clone, Debug)]
pub struct ServerOutcome {
    pub url: String,
    pub applied: bool,
    pub skipped: bool,
    pub teams: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ApplyOptions {
    pub dry_run: bool,
    pub force: bool,
    pub mode: Option<Mode>,
}

pub fn render(settings: &Settings, mode: Option<Mode>) -> Result<RenderedTheme> {
    let theme = cosmic::read(mode)?;
    let map = mapping::build(&theme, &settings.theme, &settings.overrides);
    let value = serde_json::to_string(&map).context("cannot serialize the Mattermost theme")?;
    let hash = hash_theme(&value);
    Ok(RenderedTheme { theme, value, hash })
}

pub fn hash_theme(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

pub async fn apply(
    settings: &Settings,
    state: &mut State,
    options: ApplyOptions,
) -> Result<Vec<ServerOutcome>> {
    let rendered = render(settings, options.mode)?;
    let mut outcomes = Vec::with_capacity(settings.servers.len());

    for server in &settings.servers {
        let outcome = match apply_server(server, &rendered, state, options).await {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!("{}: {error:#}", display_server(server));
                if !options.dry_run {
                    record_error(state, server, &error);
                }
                ServerOutcome {
                    url: server.url.clone(),
                    applied: false,
                    skipped: false,
                    teams: Vec::new(),
                    error: Some(format!("{error:#}")),
                }
            }
        };
        outcomes.push(outcome);
    }

    if !options.dry_run {
        state.save()?;
    }
    Ok(outcomes)
}

async fn apply_server(
    server: &Server,
    rendered: &RenderedTheme,
    state: &mut State,
    options: ApplyOptions,
) -> Result<ServerOutcome> {
    let url = server.normalized_url()?;
    let previous = state.get(&url).cloned().unwrap_or_default();

    if options.dry_run {
        print_dry_run(&url, server.sync_all_teams, &previous, rendered);
        return Ok(ServerOutcome {
            url,
            applied: false,
            skipped: false,
            teams: previous.teams,
            error: None,
        });
    }

    if server.auth == Auth::Unset {
        anyhow::bail!("no authentication configured; run `cosmicmost init`");
    }

    let client = MattermostClient::from_server(server).await?;
    let user = client.me().await.context("cannot authenticate")?;

    let teams = if server.sync_all_teams {
        client
            .teams()
            .await
            .context("cannot list teams")?
            .into_iter()
            .map(|team| team.id)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let previous = state.get(&url).cloned().unwrap_or_default();
    let unchanged = !options.force
        && previous.url == url
        && previous.user_id == user.id
        && previous.last_theme_hash == rendered.hash
        && previous.teams == teams;

    if unchanged {
        info!("{url}: theme unchanged, skipping");
        return Ok(ServerOutcome {
            url,
            applied: false,
            skipped: true,
            teams,
            error: None,
        });
    }

    let preferences = theme_preferences(&user.id, &teams, &rendered.value);
    client
        .save_preferences(&preferences)
        .await
        .context("cannot save the theme preference")?;

    delete_removed_team_preferences(&client, &user.id, &previous.teams, &teams).await;

    let now = unix_now();
    state.upsert(ServerState {
        url: url.clone(),
        user_id: user.id,
        last_theme_hash: rendered.hash.clone(),
        last_applied_at: now,
        teams: teams.clone(),
        last_error: None,
    });

    info!(
        "{url}: applied theme ({} team{})",
        teams.len(),
        if teams.len() == 1 { "" } else { "s" }
    );

    Ok(ServerOutcome {
        url,
        applied: true,
        skipped: false,
        teams,
        error: None,
    })
}

fn theme_preferences(user_id: &str, teams: &[String], value: &str) -> Vec<Preference> {
    let mut preferences = Vec::with_capacity(teams.len() + 1);
    preferences.push(Preference::theme(user_id, "", value.to_owned()));
    for team in teams {
        preferences.push(Preference::theme(user_id, team, value.to_owned()));
    }
    preferences
}

async fn delete_removed_team_preferences(
    client: &MattermostClient,
    user_id: &str,
    previous: &[String],
    current: &[String],
) {
    let removed = previous
        .iter()
        .filter(|team| !current.contains(team))
        .map(|team| Preference::theme(user_id, team, String::new()))
        .collect::<Vec<_>>();

    if removed.is_empty() {
        return;
    }

    if let Err(error) = client.delete_preferences(&removed).await {
        warn!("cannot remove stale team theme preferences: {error:#}");
    }
}

fn record_error(state: &mut State, server: &Server, error: &anyhow::Error) {
    let url = server
        .normalized_url()
        .unwrap_or_else(|_| server.url.clone());
    let mut entry = state.get(&url).cloned().unwrap_or_default();
    entry.url = url;
    entry.last_error = Some(format!("{error:#}"));
    state.upsert(entry);
}

fn print_dry_run(
    url: &str,
    sync_all_teams: bool,
    previous: &ServerState,
    rendered: &RenderedTheme,
) {
    println!("server: {url}");
    println!(
        "mode: {}",
        if rendered.theme.is_dark {
            "dark"
        } else {
            "light"
        }
    );
    println!(
        "theme changed since last apply: {}",
        previous.last_theme_hash != rendered.hash
    );
    println!("preferences:");
    println!("  theme/\"\" (global)");
    if sync_all_teams {
        println!("  theme/<team id> (one per team)");
    }
    println!("theme:");
    println!("{}", pretty_theme(rendered));
}

pub fn pretty_theme(rendered: &RenderedTheme) -> String {
    serde_json::from_str::<serde_json::Value>(&rendered.value)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| rendered.value.clone())
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn display_server(server: &Server) -> &str {
    if server.url.is_empty() {
        "<unnamed server>"
    } else {
        &server.url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_prefixed() {
        let a = hash_theme("{\"a\":1}");
        let b = hash_theme("{\"a\":1}");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_ne!(a, hash_theme("{\"a\":2}"));
    }

    #[test]
    fn builds_global_and_team_preferences() {
        let teams = vec!["t1".to_owned(), "t2".to_owned()];
        let preferences = theme_preferences("u1", &teams, "{}");
        assert_eq!(preferences.len(), 3);
        assert_eq!(preferences[0].name, "");
        assert_eq!(preferences[1].name, "t1");
        assert_eq!(preferences[2].name, "t2");
        assert!(preferences.iter().all(Preference::is_theme));
    }
}
