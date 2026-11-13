// SPDX-License-Identifier: GPL-3.0-only

// Tiny client for the bits of the Mattermost API we need (me/teams/preferences).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};

use crate::config::{Server, resolve_token};

/// Mattermost refuses preference writes larger than this.
pub const MAX_PREFERENCES_PER_REQUEST: usize = 100;

pub const THEME_CATEGORY: &str = "theme";

#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Team {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Preference {
    pub user_id: String,
    pub category: String,
    pub name: String,
    pub value: String,
}

impl Preference {
    pub fn theme(user_id: &str, team_id: &str, value: String) -> Self {
        Self {
            user_id: user_id.to_owned(),
            category: THEME_CATEGORY.to_owned(),
            name: team_id.to_owned(),
            value,
        }
    }

    pub fn is_theme(&self) -> bool {
        self.category == THEME_CATEGORY
    }
}

pub struct MattermostClient {
    http: Client,
    base: Url,
    token: String,
}

impl MattermostClient {
    pub async fn from_server(server: &Server) -> Result<Self> {
        let token = resolve_token(&server.auth, &server.url)
            .await
            .with_context(|| format!("cannot obtain a token for {}", server.url))?;
        Self::with_token(&server.url, &token)
    }

    pub fn with_token(base: &str, token: &str) -> Result<Self> {
        let base = Url::parse(&crate::config::normalize_url(base)?)
            .context("invalid Mattermost server URL")?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("cosmicmost/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("cannot build the HTTP client")?;
        Ok(Self {
            http,
            base,
            token: token.to_owned(),
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        let url = format!(
            "{}/api/v4/{}",
            self.base.as_str().trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        Url::parse(&url).expect("endpoint is built from a validated base URL")
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let response = self
            .http
            .get(self.endpoint(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("cannot reach {}{path}", self.base))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            bail!("Mattermost returned {}: {}", status, api_message(&body));
        }

        serde_json::from_str(&body)
            .with_context(|| format!("unexpected response from {}{path}", self.base))
    }

    async fn send<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<()> {
        let response = self
            .http
            .request(method, self.endpoint(path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("cannot reach {}{path}", self.base))?;

        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Mattermost returned {}: {}", status, api_message(&body))
    }

    pub async fn me(&self) -> Result<User> {
        self.get("users/me").await
    }

    pub async fn teams(&self) -> Result<Vec<Team>> {
        self.get("users/me/teams").await
    }

    pub async fn preferences(&self) -> Result<Vec<Preference>> {
        self.get("users/me/preferences").await
    }

    pub async fn save_preferences(&self, preferences: &[Preference]) -> Result<()> {
        for chunk in preferences.chunks(MAX_PREFERENCES_PER_REQUEST) {
            self.send(Method::PUT, "users/me/preferences", chunk)
                .await?;
        }
        Ok(())
    }

    pub async fn delete_preferences(&self, preferences: &[Preference]) -> Result<()> {
        for chunk in preferences.chunks(MAX_PREFERENCES_PER_REQUEST) {
            self.send(Method::POST, "users/me/preferences/delete", chunk)
                .await?;
        }
        Ok(())
    }
}

fn api_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<String>,
        id: Option<String>,
    }

    match serde_json::from_str::<ApiError>(body) {
        Ok(error) => error
            .message
            .or(error.id)
            .unwrap_or_else(|| body.trim().to_owned()),
        Err(_) => {
            let body = body.trim();
            if body.is_empty() {
                "<empty response>".to_owned()
            } else {
                body.chars().take(500).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn reads_the_current_user() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/users/me"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "user-id",
                "username": "eduardo"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = MattermostClient::with_token(&server.uri(), "secret").unwrap();
        let user = client.me().await.unwrap();
        assert_eq!(user.id, "user-id");
        assert_eq!(user.username, "eduardo");
    }

    #[tokio::test]
    async fn keeps_a_path_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/mattermost/api/v4/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "user-id",
                "username": "eduardo"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/mattermost", server.uri());
        let client = MattermostClient::with_token(&base, "secret").unwrap();
        assert_eq!(client.me().await.unwrap().id, "user-id");
    }

    #[tokio::test]
    async fn saves_preferences_in_batches() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/users/me/preferences"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "OK"})))
            .expect(2)
            .mount(&server)
            .await;

        let preferences: Vec<Preference> = (0..101)
            .map(|i| Preference::theme("user", &format!("team-{i}"), "{}".to_owned()))
            .collect();

        let client = MattermostClient::with_token(&server.uri(), "secret").unwrap();
        client.save_preferences(&preferences).await.unwrap();
    }

    #[tokio::test]
    async fn sends_the_expected_theme_payload() {
        let server = MockServer::start().await;
        let preferences = vec![Preference::theme(
            "user-id",
            "",
            "{\"type\":\"custom\"}".to_owned(),
        )];

        Mock::given(method("PUT"))
            .and(path("/api/v4/users/me/preferences"))
            .and(body_json(json!([{
                "user_id": "user-id",
                "category": "theme",
                "name": "",
                "value": "{\"type\":\"custom\"}"
            }])))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "OK"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = MattermostClient::with_token(&server.uri(), "secret").unwrap();
        client.save_preferences(&preferences).await.unwrap();
    }

    #[tokio::test]
    async fn surfaces_api_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/users/me"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "id": "api.context.session_expired.app_error",
                "message": "Invalid or expired session, please login again."
            })))
            .mount(&server)
            .await;

        let client = MattermostClient::with_token(&server.uri(), "secret").unwrap();
        let error = client.me().await.unwrap_err().to_string();
        assert!(error.contains("401"));
        assert!(error.contains("Invalid or expired session"));
    }
}
