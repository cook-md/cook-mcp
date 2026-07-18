pub mod device;
pub mod store;

use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::Config;
use store::StoredAuth;

/// Resolves the token to send: env override > stored login. Renews stored
/// tokens when they're within 24h of expiry; clears them on renew-401.
pub struct AuthManager {
    pub path: PathBuf,
    cookmd_url: String,
    token_override: Option<String>,
    cached: Mutex<Option<StoredAuth>>,
}

impl AuthManager {
    pub fn new(cfg: &Config) -> Self {
        Self {
            path: cfg.auth_path.clone().unwrap_or_else(store::default_path),
            cookmd_url: cfg.cookmd_url.clone(),
            token_override: cfg.token_override.clone(),
            cached: Mutex::new(None),
        }
    }

    pub fn store_token(&self, token: &str) -> anyhow::Result<StoredAuth> {
        let auth = StoredAuth::from_token(token)?;
        store::save(&self.path, &auth)?;
        *self.cached.lock().unwrap() = Some(auth.clone());
        Ok(auth)
    }

    pub fn current(&self) -> Option<StoredAuth> {
        if let Some(a) = self.cached.lock().unwrap().clone() {
            return Some(a);
        }
        let loaded = store::load(&self.path).ok().flatten();
        *self.cached.lock().unwrap() = loaded.clone();
        loaded
    }

    /// The bearer to attach to service calls, if any.
    pub async fn bearer(&self) -> Option<String> {
        if let Some(t) = &self.token_override {
            return Some(t.clone());
        }
        let auth = self.current()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        if auth.expiring_soon(now) {
            let flow = device::DeviceFlow::new(self.cookmd_url.clone());
            match flow.renew(&auth.token).await {
                // Concurrent renewals are benign: cook.md renew doesn't
                // invalidate the old token, so last write wins.
                Ok(new_token) => return self.store_token(&new_token).ok().map(|a| a.token),
                Err(device::RenewError::Unauthorized) => {
                    // Session revoked server-side — the stored token is dead
                    // regardless of its exp claim.
                    let _ = store::clear(&self.path);
                    *self.cached.lock().unwrap() = None;
                    return None;
                }
                Err(device::RenewError::Other(_)) => {
                    // Transient failure; if the token is outright expired,
                    // drop it, otherwise keep serving the still-valid one.
                    if auth.expires_at <= now {
                        let _ = store::clear(&self.path);
                        *self.cached.lock().unwrap() = None;
                        return None;
                    }
                }
            }
        }
        Some(auth.token)
    }

    pub fn logout(&self) -> anyhow::Result<()> {
        *self.cached.lock().unwrap() = None;
        store::clear(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unsigned test JWT with a far-future `exp` — decode only reads the
    /// middle segment. (store.rs has its own copy, private to its test mod.)
    fn test_jwt(exp: i64) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = serde_json::json!({"uid": 1, "email": "a@b.c", "exp": exp});
        format!("h.{}.s", b64.encode(payload.to_string()))
    }

    fn config_with(
        auth_path: &std::path::Path,
        token_override: Option<&str>,
        cookmd_url: Option<&str>,
    ) -> Config {
        let path = auth_path.to_string_lossy().to_string();
        let over = token_override.map(str::to_string);
        let cookmd = cookmd_url.map(str::to_string);
        Config::from_vars(move |k| match k {
            "NUTRITION_MCP_AUTH_PATH" => Some(path.clone()),
            "NUTRITION_API_TOKEN" => over.clone(),
            "COOKMD_BASE_URL" => cookmd.clone(),
            _ => None,
        })
    }

    fn now_unix() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[tokio::test]
    async fn bearer_prefers_token_override_over_stored_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let stored = StoredAuth::from_token(&test_jwt(4_102_444_800)).unwrap();
        store::save(&path, &stored).unwrap();
        let mgr = AuthManager::new(&config_with(&path, Some("org-key"), None));
        assert_eq!(mgr.bearer().await.as_deref(), Some("org-key"));
    }

    #[tokio::test]
    async fn bearer_returns_stored_token_without_renewal_when_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let jwt = test_jwt(4_102_444_800); // year 2100 — nowhere near expiry
        store::save(&path, &StoredAuth::from_token(&jwt).unwrap()).unwrap();
        // No mock server is running: any attempted renew would hit the real
        // cook.md URL and fail, surfacing as a changed or absent token.
        let mgr = AuthManager::new(&config_with(&path, None, None));
        assert_eq!(mgr.bearer().await.as_deref(), Some(jwt.as_str()));
    }

    #[tokio::test]
    async fn bearer_renews_expiring_token_and_persists_it() {
        use wiremock::matchers::{method, path as url_path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let new_jwt = test_jwt(4_102_444_800);
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(url_path("/api/sessions/renew"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": new_jwt})),
            )
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let old_jwt = test_jwt(now_unix() + 3_600); // inside the 24h window
        store::save(&auth_path, &StoredAuth::from_token(&old_jwt).unwrap()).unwrap();

        let mgr = AuthManager::new(&config_with(&auth_path, None, Some(&mock.uri())));
        assert_eq!(mgr.bearer().await.as_deref(), Some(new_jwt.as_str()));
        let on_disk = store::load(&auth_path).unwrap().unwrap();
        assert_eq!(on_disk.token, new_jwt);
    }

    #[tokio::test]
    async fn bearer_renew_401_clears_stored_login() {
        use wiremock::matchers::{method, path as url_path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(url_path("/api/sessions/renew"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        // Expiring soon but NOT expired — only the 401 justifies clearing.
        let old_jwt = test_jwt(now_unix() + 3_600);
        store::save(&auth_path, &StoredAuth::from_token(&old_jwt).unwrap()).unwrap();

        let mgr = AuthManager::new(&config_with(&auth_path, None, Some(&mock.uri())));
        assert_eq!(mgr.bearer().await, None);
        assert!(store::load(&auth_path).unwrap().is_none());
    }
}
