// SPDX-License-Identifier: GPL-3.0-only

//! Configuration and state storage, backed by [`cosmic_config`].
//!
//! User configuration lives in the XDG config directory:
//! `~/.config/cosmic/io.github.cosmic-utils.cosmicmost/v1/`.
//!
//! Runtime state lives in the XDG state directory:
//! `~/.local/state/cosmic/io.github.cosmic-utils.cosmicmost/v1/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};
use cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic_config::{Config, CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Reverse-DNS identifier used for both the config and state directories.
pub const APP_ID: &str = "io.github.cosmic-utils.cosmicmost";

/// How to obtain the Mattermost personal access token.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Auth {
    /// No authentication configured yet.
    #[default]
    Unset,
    /// Token stored in the system keychain.
    Keyring,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unset => formatter.write_str("Unset"),
            Self::Keyring => formatter.write_str("Keyring"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Server {
    pub url: String,
    pub auth: Auth,
    pub sync_all_teams: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            url: String::new(),
            auth: Auth::Unset,
            sync_all_teams: true,
        }
    }
}

impl Server {
    pub fn normalized_url(&self) -> Result<String> {
        normalize_url(&self.url)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ThemePrefs {
    pub code_theme_dark: String,
    pub code_theme_light: String,
}

impl Default for ThemePrefs {
    fn default() -> Self {
        Self {
            code_theme_dark: "solarized-dark".to_owned(),
            code_theme_light: "github".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Overrides {
    pub dark: BTreeMap<String, String>,
    pub light: BTreeMap<String, String>,
}

impl Overrides {
    pub fn for_mode(&self, is_dark: bool) -> &BTreeMap<String, String> {
        if is_dark { &self.dark } else { &self.light }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
#[serde(default)]
pub struct Settings {
    pub servers: Vec<Server>,
    pub theme: ThemePrefs,
    pub overrides: Overrides,
}

impl Settings {
    pub fn config() -> Result<Config> {
        Config::new(APP_ID, Self::VERSION).context("cannot open the cosmicmost config directory")
    }

    /// Load the configuration, falling back to defaults for missing or invalid keys.
    pub fn load(config: &Config) -> Self {
        match Self::get_entry(config) {
            Ok(settings) => settings,
            Err((errors, settings)) => {
                for error in errors {
                    warn!("ignoring invalid config value: {error}");
                }
                settings
            }
        }
    }

    pub fn load_or_default() -> Self {
        match Self::config() {
            Ok(config) => Self::load(&config),
            Err(error) => {
                debug!("using default config: {error:#}");
                Self::default()
            }
        }
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        self.write_entry(config)
            .context("cannot write cosmicmost config")?;
        secure_config_path(config);
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.servers.is_empty() {
            bail!(
                "no Mattermost servers configured; run `cosmicmost init` or edit {}",
                config_dir_display()
            );
        }

        for server in &self.servers {
            server.normalized_url()?;
            if server.auth == Auth::Unset {
                bail!(
                    "server {} has no authentication configured; run `cosmicmost init`",
                    server.url
                );
            }
        }

        if self.theme.code_theme_dark.trim().is_empty() {
            bail!("theme.code_theme_dark must not be empty");
        }
        if self.theme.code_theme_light.trim().is_empty() {
            bail!("theme.code_theme_light must not be empty");
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerState {
    pub url: String,
    pub user_id: String,
    pub last_theme_hash: String,
    pub last_applied_at: i64,
    pub teams: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
#[serde(default)]
pub struct State {
    pub servers: Vec<ServerState>,
}

impl State {
    pub fn config() -> Result<Config> {
        Config::new_state(APP_ID, Self::VERSION)
            .context("cannot open the cosmicmost state directory")
    }

    pub fn load_or_default() -> Self {
        match Self::config() {
            Ok(config) => match Self::get_entry(&config) {
                Ok(state) => state,
                Err((errors, state)) => {
                    for error in errors {
                        warn!("ignoring invalid state value: {error}");
                    }
                    state
                }
            },
            Err(error) => {
                debug!("using empty state: {error:#}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let config = Self::config()?;
        self.write_entry(&config).context("cannot write state")
    }

    pub fn get(&self, url: &str) -> Option<&ServerState> {
        self.servers.iter().find(|state| state.url == url)
    }

    pub fn get_mut(&mut self, url: &str) -> Option<&mut ServerState> {
        self.servers.iter_mut().find(|state| state.url == url)
    }

    pub fn upsert(&mut self, state: ServerState) {
        match self.get_mut(&state.url) {
            Some(existing) => *existing = state,
            None => self.servers.push(state),
        }
    }
}

/// Resolve an [`Auth`] value to a token without ever logging the secret.
pub async fn resolve_token(auth: &Auth, server_url: &str) -> Result<String> {
    match auth {
        Auth::Unset => bail!("no authentication configured"),
        Auth::Keyring => {
            let entry = keyring::Entry::new(APP_ID, server_url)
                .context("cannot access the system keychain")?;
            let token = entry.get_password().context(
                "cannot retrieve the token from the keychain; run `cosmicmost init` again",
            )?;
            let token = token.trim().to_owned();
            if token.is_empty() {
                bail!("the token stored in the keychain is empty; run `cosmicmost init` again");
            }
            Ok(token)
        }
    }
}

/// Normalize a user-supplied Mattermost URL.
pub fn normalize_url(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("server URL must not be empty");
    }

    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };

    let mut url = reqwest::Url::parse(&with_scheme)
        .with_context(|| format!("invalid Mattermost URL: {input}"))?;

    if !matches!(url.scheme(), "http" | "https") {
        bail!(
            "unsupported URL scheme `{}`; use http or https",
            url.scheme()
        );
    }

    url.set_fragment(None);
    url.set_query(None);

    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);

    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// Path that users should edit for the main configuration.
pub fn config_dir_display() -> String {
    match Settings::config().and_then(|config| {
        config
            .user_path()
            .map(PathBuf::from)
            .context("config directory is unavailable")
    }) {
        Ok(path) => path.display().to_string(),
        Err(_) => format!("~/.config/cosmic/{APP_ID}/v1"),
    }
}

/// Tighten permissions on the config dir/file a bit.
///
/// Tokens live in the keychain, so this is mostly about not leaving server URLs
/// and other prefs world-readable on multi-user systems. cosmic-config doesn't do
/// this for us.
pub fn secure_config_path(config: &Config) {
    #[cfg(unix)]
    {
        if let Some(dir) = config.user_path() {
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
            let path = dir.join("servers");
            if path.is_file() {
                if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
                    debug!("could not chmod {}: {error}", path.display());
                }
            }
        }
    }

    #[cfg(not(unix))]
    let _ = config;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_config::Config as CosmicConfig;

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            normalize_url("chat.example.com").unwrap(),
            "https://chat.example.com"
        );
        assert_eq!(
            normalize_url("https://chat.example.com/").unwrap(),
            "https://chat.example.com"
        );
        assert_eq!(
            normalize_url("http://localhost:8065/").unwrap(),
            "http://localhost:8065"
        );
        assert!(normalize_url("ftp://chat.example.com").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn server_defaults_to_all_teams() {
        assert!(Server::default().sync_all_teams);
        assert_eq!(Server::default().auth, Auth::Unset);
    }

    #[test]
    fn settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            CosmicConfig::with_custom_path(APP_ID, Settings::VERSION, dir.path().into()).unwrap();

        let settings = Settings {
            servers: vec![Server {
                url: "https://chat.example.com".to_owned(),
                auth: Auth::Keyring,
                sync_all_teams: false,
            }],
            theme: ThemePrefs {
                code_theme_dark: "monokai".to_owned(),
                code_theme_light: "github".to_owned(),
            },
            overrides: Overrides {
                dark: BTreeMap::from([("linkColor".to_owned(), "#ff0000".to_owned())]),
                light: BTreeMap::new(),
            },
        };

        settings.save(&config).unwrap();
        let loaded = Config::with_custom_path(APP_ID, Settings::VERSION, dir.path().into())
            .map(|config| Settings::load(&config))
            .unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            CosmicConfig::with_custom_path(APP_ID, State::VERSION, dir.path().into()).unwrap();
        let state = State {
            servers: vec![ServerState {
                url: "https://chat.example.com".to_owned(),
                user_id: "user".to_owned(),
                last_theme_hash: "sha256:abc".to_owned(),
                last_applied_at: 42,
                teams: vec!["team".to_owned()],
                last_error: None,
            }],
        };
        state.write_entry(&config).unwrap();
        let loaded = State::get_entry(&config).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn deserializes_auth_variants() {
        let keyring: Auth = ron::from_str("Keyring").unwrap();
        assert_eq!(keyring, Auth::Keyring);
    }
}
