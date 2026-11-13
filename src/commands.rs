// SPDX-License-Identifier: GPL-3.0-only

// Command handlers. Most of the "app logic" lives here and in sync.rs.

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use cosmic_config::Config;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::cli::{ApplyArgs, InitArgs, InstallServiceArgs, PreviewArgs, ResetArgs};
use crate::config::{self, Auth, Server, Settings, State, normalize_url};
use crate::mattermost::MattermostClient;
use crate::sync::{self, ApplyOptions};

pub fn preview(args: PreviewArgs) -> Result<()> {
    let settings = Settings::load_or_default();
    let rendered = sync::render(&settings, args.mode.map(Into::into))?;
    let json = sync::pretty_theme(&rendered);

    match args.output {
        Some(path) => {
            std::fs::write(&path, format!("{json}\n"))
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("wrote {}", path.display());
        }
        None => println!("{json}"),
    }

    eprintln!(
        "COSMIC mode: {}; {}",
        if rendered.theme.is_dark {
            "dark"
        } else {
            "light"
        },
        rendered.hash
    );

    Ok(())
}

pub async fn init(args: InitArgs) -> Result<()> {
    let config = Settings::config()?;
    let mut settings = Settings::load(&config);

    let url = match args.url.as_deref() {
        Some(url) => normalize_url(url)?,
        None => {
            let default = settings
                .servers
                .first()
                .map(|server| server.url.as_str())
                .unwrap_or("https://chat.example.com");
            normalize_url(&prompt("Mattermost URL", Some(default))?)?
        }
    };

    let token = rpassword::prompt_password("Personal access token: ")
        .context("couldn't read the token from the terminal")?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        bail!("token can't be empty");
    }

    let sync_all_teams = if args.no_sync_all_teams {
        false
    } else if args.url.is_some() {
        // non-interactive: default to syncing all teams
        true
    } else {
        prompt_bool("Sync the theme to every team?", true)?
    };

    if !args.skip_validation {
        let client = MattermostClient::with_token(&url, &token)?;
        let user = client
            .me()
            .await
            .context("couldn't validate the server URL/token")?;
        println!("Authenticated as {} on {url}", user.username);
    }

    let entry =
        keyring::Entry::new(config::APP_ID, &url).context("couldn't access the system keychain")?;
    entry
        .set_password(&token)
        .context("couldn't store the token in the keychain")?;

    let server = Server {
        url: url.clone(),
        auth: Auth::Keyring,
        sync_all_teams,
    };

    if args.replace {
        settings.servers.clear();
    }

    let mut replaced = false;
    for existing in &mut settings.servers {
        if normalize_url(&existing.url).ok().as_deref() == Some(url.as_str()) {
            *existing = server.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        settings.servers.push(server);
    }

    settings.save(&config)?;

    println!("Saved config to {}", display_path(&config));
    println!(
        "Tip: in Mattermost Desktop (v6+) you can enable \"Synchronize Desktop App theme with server\" \
         under Settings > Advanced if you want the window chrome to follow."
    );

    Ok(())
}

pub async fn apply(args: ApplyArgs) -> Result<()> {
    let config = Settings::config()?;
    let settings = Settings::load(&config);
    settings.validate()?;

    let mut state = State::load_or_default();
    let outcomes = sync::apply(
        &settings,
        &mut state,
        ApplyOptions {
            dry_run: args.dry_run,
            force: args.force,
            mode: args.mode.map(Into::into),
        },
    )
    .await?;

    let mut failures = 0;
    for outcome in &outcomes {
        if let Some(error) = &outcome.error {
            failures += 1;
            eprintln!("{}: {error}", outcome.url);
        } else if outcome.skipped {
            println!("{}: unchanged", outcome.url);
        } else if outcome.applied {
            println!(
                "{}: applied ({} team{})",
                outcome.url,
                outcome.teams.len(),
                if outcome.teams.len() == 1 { "" } else { "s" }
            );
        } else if args.dry_run {
            println!("{}: dry run", outcome.url);
        }
    }

    if failures > 0 {
        bail!("{failures} server(s) failed");
    }

    Ok(())
}

pub async fn daemon() -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _watchers = crate::cosmic::watch_all(tx)?;

    let mut state = State::load_or_default();
    let mut failures = attempt(&mut state, 0).await;
    let mut retry_at = if failures == 0 {
        None
    } else {
        Some(Instant::now() + backoff(failures))
    };

    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    let mut sigterm = signal(SignalKind::terminate()).context("cannot install SIGTERM handler")?;
    let mut sighup = signal(SignalKind::hangup()).context("cannot install SIGHUP handler")?;

    info!("watching COSMIC theme and cosmicmost configuration");

    loop {
        let deadline = retry_at.unwrap_or_else(|| Instant::now() + Duration::from_secs(86_400));

        tokio::select! {
            _ = &mut ctrl_c => {
                info!("shutting down");
                break;
            }
            _ = sigterm.recv() => {
                info!("shutting down");
                break;
            }
            _ = sighup.recv() => {
                info!("SIGHUP received; resyncing");
                failures = attempt(&mut state, 0).await;
                retry_at = retry_deadline(failures);
            }
            Some(_) = rx.recv() => {
                // COSMIC writes several files per change; coalesce them.
                tokio::time::sleep(Duration::from_millis(400)).await;
                while rx.try_recv().is_ok() {}
                info!("COSMIC theme or configuration changed; syncing");
                failures = attempt(&mut state, 0).await;
                retry_at = retry_deadline(failures);
            }
            _ = tokio::time::sleep_until(deadline) => {
                info!("retrying after {} failure(s)", failures);
                failures = attempt(&mut state, failures).await;
                retry_at = retry_deadline(failures);
            }
        }
    }

    Ok(())
}

async fn attempt(state: &mut State, previous_failures: u32) -> u32 {
    let config = match Settings::config() {
        Ok(config) => config,
        Err(error) => {
            warn!("cannot open configuration: {error:#}");
            return previous_failures.saturating_add(1);
        }
    };
    let settings = Settings::load(&config);

    if let Err(error) = settings.validate() {
        warn!("{error:#}");
        return previous_failures.saturating_add(1);
    }

    match sync::apply(&settings, state, ApplyOptions::default()).await {
        Ok(outcomes) => {
            let failed = outcomes
                .iter()
                .filter(|outcome| outcome.error.is_some())
                .count();

            for outcome in &outcomes {
                if let Some(error) = &outcome.error {
                    warn!("{}: {error}", outcome.url);
                } else if outcome.skipped {
                    info!("{}: unchanged", outcome.url);
                } else {
                    info!("{}: applied", outcome.url);
                }
            }

            if failed == 0 {
                0
            } else {
                previous_failures.saturating_add(1)
            }
        }
        Err(error) => {
            warn!("sync failed: {error:#}");
            previous_failures.saturating_add(1)
        }
    }
}

fn retry_deadline(failures: u32) -> Option<Instant> {
    if failures == 0 {
        None
    } else {
        Some(Instant::now() + backoff(failures))
    }
}

fn backoff(failures: u32) -> Duration {
    let seconds = 5u64.saturating_mul(2u64.saturating_pow(failures.min(6)));
    Duration::from_secs(seconds.min(300))
}

pub async fn reset(args: ResetArgs) -> Result<()> {
    let config = Settings::config()?;
    let settings = Settings::load(&config);
    settings.validate()?;

    for server in &settings.servers {
        let client = MattermostClient::from_server(server).await?;
        let user = client.me().await.context("cannot authenticate")?;
        let preferences = client.preferences().await?;

        let mut theme_preferences = preferences
            .into_iter()
            .filter(|preference| preference.is_theme())
            .collect::<Vec<_>>();

        if !args.all_teams {
            theme_preferences.retain(|preference| preference.name.is_empty());
        }

        if theme_preferences.is_empty() {
            println!("{}: no theme preferences found", server.url);
            continue;
        }

        if args.dry_run {
            println!(
                "{}: would remove {} theme preference(s) for {}",
                server.url,
                theme_preferences.len(),
                user.username
            );
        } else {
            client.delete_preferences(&theme_preferences).await?;
            println!(
                "{}: removed {} theme preference(s)",
                server.url,
                theme_preferences.len()
            );
        }
    }

    if args.forget {
        if args.dry_run {
            println!("would forget the stored state");
        } else {
            State::default().save()?;
            println!("forgot the stored state");
        }
    }

    Ok(())
}

pub fn status() -> Result<()> {
    let config = Settings::config()?;
    let settings = Settings::load(&config);
    let state = State::load_or_default();

    println!("Config: {}", display_path(&config));
    println!("State:  {}", display_path(&State::config()?));

    let rendered = sync::render(&settings, None);
    match &rendered {
        Ok(rendered) => {
            println!(
                "COSMIC: {} ({})",
                if rendered.theme.is_dark {
                    "dark"
                } else {
                    "light"
                },
                rendered.hash
            );
        }
        Err(error) => {
            warn!("cannot read the COSMIC theme: {error:#}");
        }
    }

    if settings.servers.is_empty() {
        println!("Servers: none (run `cosmicmost init`)");
        return Ok(());
    }

    let current_hash = rendered.as_ref().ok().map(|rendered| rendered.hash.clone());

    for server in &settings.servers {
        let url = server
            .normalized_url()
            .unwrap_or_else(|_| server.url.clone());
        println!();
        println!("{url}");
        println!(
            "  sync all teams: {}",
            if server.sync_all_teams { "yes" } else { "no" }
        );

        match state.get(&url) {
            Some(entry) => {
                println!("  user: {}", entry.user_id);
                println!(
                    "  last applied: {} ({})",
                    entry.last_applied_at,
                    relative_time(entry.last_applied_at)
                );
                println!("  teams: {}", entry.teams.len());
                if let Some(hash) = &current_hash {
                    println!("  up to date: {}", entry.last_theme_hash == *hash);
                }
                if let Some(error) = &entry.last_error {
                    println!("  last error: {error}");
                }
            }
            None => println!("  never applied"),
        }
    }

    Ok(())
}

pub fn paths() -> Result<()> {
    let config = Settings::config()?;
    let state = State::config()?;
    println!("Config:  {}", display_path(&config));
    println!("State:   {}", display_path(&state));
    println!("Service: {}", unit_path()?.display());
    Ok(())
}

pub fn install_service(args: InstallServiceArgs) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the cosmicmost binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);

    if exe
        .components()
        .any(|component| component.as_os_str() == "target")
    {
        warn!(
            "installing a service for a cargo build artifact at {}; \
             install the binary first for a stable path",
            exe.display()
        );
    }

    let unit_path = unit_path()?;
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let unit = unit_contents(&exe);

    std::fs::write(&unit_path, unit)
        .with_context(|| format!("cannot write {}", unit_path.display()))?;
    println!("Wrote {}", unit_path.display());

    if args.no_enable {
        println!("Not enabling. Run: systemctl --user enable --now cosmicmost.service");
        return Ok(());
    }

    run("systemctl", &["--user", "daemon-reload"])?;

    let mut enable = vec!["--user", "enable"];
    if !args.no_start {
        enable.push("--now");
    }
    enable.push("cosmicmost.service");
    run("systemctl", &enable)?;

    println!(
        "{} cosmicmost.service",
        if args.no_start {
            "Enabled"
        } else {
            "Enabled and started"
        }
    );
    Ok(())
}

fn unit_path() -> Result<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("systemd/user/cosmicmost.service"));
    }

    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/systemd/user/cosmicmost.service"))
}

fn unit_contents(exe: &std::path::Path) -> String {
    format!(
        "[Unit]\n\
         Description=Sync the COSMIC theme to Mattermost\n\
         Documentation=https://github.com/cosmic-utils/cosmic-ext-mattermost-theme-sync\n\
         After=graphical-session.target\n\
         PartOf=graphical-session.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} daemon\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         Environment=RUST_LOG=warn,cosmicmost=info\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        systemd_quote(&exe.display().to_string())
    )
}

fn systemd_quote(value: &str) -> String {
    let escaped = value
        .replace('%', "%%")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("cannot run {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

fn display_path(config: &Config) -> String {
    config
        .user_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_owned())
}

fn relative_time(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "never".to_owned();
    }

    let seconds = sync::unix_now().saturating_sub(timestamp);
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(default) => print!("{label} [{default}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush().context("cannot write to stdout")?;

    let mut line = String::new();
    if io::stdin()
        .read_line(&mut line)
        .context("cannot read from stdin")?
        == 0
    {
        bail!("input closed");
    }

    let line = line.trim().to_owned();
    if line.is_empty()
        && let Some(default) = default
    {
        return Ok(default.to_owned());
    }
    Ok(line)
}

fn prompt_bool(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        let answer = prompt(&format!("{label} ({hint})"), None)?;
        match answer.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            "" => return Ok(default),
            _ => eprintln!("Please answer yes or no."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_runs_the_daemon_subcommand() {
        let unit = unit_contents(std::path::Path::new("/usr/bin/cosmicmost"));
        assert!(
            unit.contains("ExecStart=\"/usr/bin/cosmicmost\" daemon\n"),
            "{unit}"
        );
    }

    #[test]
    fn unit_escapes_systemd_specifiers_and_quotes() {
        let unit = unit_contents(std::path::Path::new("/home/a b/%i\\c\"d/cosmicmost"));
        assert!(unit.contains(r#"ExecStart="/home/a b/%%i\\c\"d/cosmicmost" daemon"#));
    }
}
