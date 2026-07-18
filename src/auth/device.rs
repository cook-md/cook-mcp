use std::time::Duration;

use serde::Deserialize;

/// cook.md RFC 8628 device-authorization flow. The web app already ships the
/// endpoints; this is a client for them.
pub struct DeviceFlow {
    cookmd_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

/// Why `renew` failed: a 401/403 means the session is gone server-side and
/// the stored token must be dropped; anything else is transient.
#[derive(Debug, thiserror::Error)]
pub enum RenewError {
    #[error("session no longer valid")]
    Unauthorized,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl DeviceFlow {
    pub fn new(cookmd_url: impl Into<String>) -> Self {
        Self {
            cookmd_url: cookmd_url.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("client"),
        }
    }

    pub async fn start(&self) -> anyhow::Result<DeviceStart> {
        let resp = self
            .http
            .post(format!("{}/oauth/device/code", self.cookmd_url))
            .form(&[("client_name", "nutrition-mcp")])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Poll until approved, denied, or `expires_in` elapses. `interval` of
    /// zero is only used by tests.
    pub async fn poll(
        &self,
        device_code: &str,
        interval: Duration,
        expires_in: Duration,
    ) -> anyhow::Result<String> {
        let deadline = tokio::time::Instant::now() + expires_in;
        let mut interval_dur = interval;
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("device login expired before approval");
            }
            let resp = self
                .http
                .post(format!("{}/oauth/device/token", self.cookmd_url))
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device_code),
                ])
                .send()
                .await?;
            if resp.status().is_success() {
                #[derive(Deserialize)]
                struct TokenResp {
                    access_token: String,
                }
                return Ok(resp.json::<TokenResp>().await?.access_token);
            }
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            match body["error"].as_str() {
                Some("authorization_pending") => {}
                Some("slow_down") => interval_dur += Duration::from_secs(5),
                Some(other) => anyhow::bail!("device login failed: {other}"),
                None => anyhow::bail!("device login failed: unexpected response {body}"),
            }
            tokio::time::sleep(interval_dur).await;
        }
    }

    /// `POST /api/sessions/renew` — re-mint from a still-valid token.
    pub async fn renew(&self, token: &str) -> Result<String, RenewError> {
        #[derive(Deserialize)]
        struct RenewResp {
            token: String,
        }
        let resp = self
            .http
            .post(format!("{}/api/sessions/renew", self.cookmd_url))
            .bearer_auth(token)
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(RenewError::Unauthorized);
        }
        let resp = resp.error_for_status().map_err(anyhow::Error::from)?;
        let body = resp
            .json::<RenewResp>()
            .await
            .map_err(anyhow::Error::from)?;
        Ok(body.token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn start_returns_codes() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dev-1", "user_code": "ABCD-1234",
                "verification_uri": format!("{}/device", mock.uri()),
                "verification_uri_complete": format!("{}/device?code=ABCD-1234", mock.uri()),
                "expires_in": 900, "interval": 0
            })))
            .mount(&mock)
            .await;
        let flow = DeviceFlow::new(mock.uri());
        let start = flow.start().await.unwrap();
        assert_eq!(start.user_code, "ABCD-1234");
        assert_eq!(start.device_code, "dev-1");
    }

    #[tokio::test]
    async fn poll_pending_then_token() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "authorization_pending"})),
            )
            .up_to_n_times(2)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/token"))
            .and(body_string_contains("device_code=dev-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "h.eyJleHAiOjQxMDI0NDQ4MDB9.s",
                "token_type": "Bearer"
            })))
            .mount(&mock)
            .await;
        let flow = DeviceFlow::new(mock.uri());
        let token = flow
            .poll(
                "dev-1",
                std::time::Duration::ZERO,
                std::time::Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert!(token.starts_with("h."));
    }

    #[tokio::test]
    async fn poll_access_denied_errors() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "access_denied"})),
            )
            .mount(&mock)
            .await;
        let flow = DeviceFlow::new(mock.uri());
        let err = flow
            .poll(
                "dev-1",
                std::time::Duration::ZERO,
                std::time::Duration::from_secs(5),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn renew_returns_new_token() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/sessions/renew"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"token": "new.jwt.here"})),
            )
            .mount(&mock)
            .await;
        let flow = DeviceFlow::new(mock.uri());
        assert_eq!(flow.renew("old").await.unwrap(), "new.jwt.here");
    }
}
