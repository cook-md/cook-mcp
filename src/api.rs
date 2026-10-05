use std::sync::Arc;

use crate::auth::AuthManager;
use crate::config::Config;

/// Raw JSON passthrough to the nutrition service. Deliberately untyped: the
/// service's response and RFC 9457 error bodies ARE the contract; narrowing
/// them here would strip fields agents use to self-correct.
pub struct ApiClient {
    cfg: Config,
    auth: Arc<AuthManager>,
    http: reqwest::Client,
}

#[derive(Debug)]
pub enum ApiOutcome {
    Ok(serde_json::Value),
    Err {
        status: u16,
        body: serde_json::Value,
    },
}

/// Which upstream a request targets, for accurate unreachable-error messages.
#[derive(Clone, Copy)]
enum Target {
    Nutrition,
    Cookmd,
}

impl ApiClient {
    pub fn new(cfg: Config, auth: Arc<AuthManager>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("client");
        Self { cfg, auth, http }
    }

    pub async fn get(
        &self,
        route: &str,
        query: &[(&str, Option<String>)],
    ) -> anyhow::Result<ApiOutcome> {
        let pairs: Vec<(&str, String)> = query
            .iter()
            .filter_map(|(k, v)| v.clone().map(|v| (*k, v)))
            .collect();
        let mut req = self
            .http
            .get(format!("{}{}", self.cfg.api_url, route))
            .query(&pairs);
        if let Some(tok) = self.auth.bearer().await {
            req = req.bearer_auth(tok);
        }
        self.finish(req, Target::Nutrition).await
    }

    pub async fn post(&self, route: &str, body: serde_json::Value) -> anyhow::Result<ApiOutcome> {
        let mut req = self
            .http
            .post(format!("{}{}", self.cfg.api_url, route))
            .json(&body);
        if let Some(tok) = self.auth.bearer().await {
            req = req.bearer_auth(tok);
        }
        self.finish(req, Target::Nutrition).await
    }

    /// GET against the cook.md web app (subscription status).
    pub async fn cookmd_get(&self, route: &str) -> anyhow::Result<ApiOutcome> {
        let mut req = self.http.get(format!("{}{}", self.cfg.cookmd_url, route));
        if let Some(tok) = self.auth.bearer().await {
            req = req.bearer_auth(tok);
        }
        self.finish(req, Target::Cookmd).await
    }

    async fn finish(
        &self,
        req: reqwest::RequestBuilder,
        target: Target,
    ) -> anyhow::Result<ApiOutcome> {
        let resp = req.send().await.map_err(|e| {
            let (name, base, env_var) = match target {
                Target::Nutrition => ("nutrition service", &self.cfg.api_url, "NUTRITION_API_URL"),
                Target::Cookmd => ("cook.md", &self.cfg.cookmd_url, "COOKMD_BASE_URL"),
            };
            anyhow::anyhow!("{name} unreachable at {base} ({e}); check {env_var}")
        })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        // Preserve non-JSON bodies (e.g. an HTML 502 from a reverse proxy)
        // verbatim instead of narrowing them to null; the wrapper object also
        // lets the 401/403 decoration below still attach.
        let mut body: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw_body": text }));
        if (200..300).contains(&status) {
            return Ok(ApiOutcome::Ok(body));
        }
        if let Some(obj) = body.as_object_mut() {
            match status {
                401 => {
                    obj.insert(
                        "hint".into(),
                        "Not authenticated. Run the `login` tool (or set NUTRITION_API_TOKEN) \
                         then retry."
                            .into(),
                    );
                }
                403 => {
                    obj.insert("checkout_url".into(), self.cfg.pricing_url().into());
                    obj.insert(
                        "hint".into(),
                        "Authenticated but no active Cook Pro subscription. \
                         Open checkout_url in a browser to subscribe."
                            .into(),
                    );
                }
                _ => {}
            }
        }
        Ok(ApiOutcome::Err { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthManager;
    use crate::config::Config;
    use std::sync::Arc;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_uri: &str, token: Option<&str>) -> ApiClient {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.keep().join("auth.json");
        let cfg = Config::from_vars(|k| match k {
            "NUTRITION_API_URL" => Some(mock_uri.to_string()),
            "COOKMD_BASE_URL" => Some(mock_uri.to_string()),
            "NUTRITION_API_TOKEN" => token.map(String::from),
            "COOK_MCP_AUTH_PATH" => Some(auth_path.to_string_lossy().into_owned()),
            _ => None,
        });
        ApiClient::new(cfg.clone(), Arc::new(AuthManager::new(&cfg)))
    }

    #[tokio::test]
    async fn get_passes_query_and_bearer_and_returns_body() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .and(query_param("ingredient", "salmon"))
            .and(header("authorization", "Bearer tok-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"kcal": 208})),
            )
            .mount(&mock)
            .await;
        let api = client_for(&mock.uri(), Some("tok-1"));
        match api
            .get("/nutrition", &[("ingredient", Some("salmon".into()))])
            .await
            .unwrap()
        {
            ApiOutcome::Ok(v) => assert_eq!(v["kcal"], 208),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_body_passes_through_with_suggestions() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                    "code": "ingredient_not_found", "detail": "no match",
                    "suggestions": ["salmon", "salmon, smoked"]})))
            .mount(&mock)
            .await;
        let api = client_for(&mock.uri(), None);
        match api.get("/nutrition", &[]).await.unwrap() {
            ApiOutcome::Err { status, body } => {
                assert_eq!(status, 404);
                assert_eq!(body["suggestions"][0], "salmon");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unauth_gets_login_hint_and_forbidden_gets_checkout() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({"code": "unauthorized"})),
            )
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({"code": "subscription_required"})),
            )
            .mount(&mock)
            .await;
        let api = client_for(&mock.uri(), None);
        let ApiOutcome::Err { body, .. } = api.get("/nutrition", &[]).await.unwrap() else {
            panic!()
        };
        assert!(body["hint"].as_str().unwrap().contains("login"));
        let ApiOutcome::Err { body, .. } = api.get("/nutrition", &[]).await.unwrap() else {
            panic!()
        };
        assert!(body["checkout_url"].as_str().unwrap().contains("/pricing"));
    }

    #[tokio::test]
    async fn non_json_401_body_survives_as_raw_body_with_hint() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .respond_with(ResponseTemplate::new(401).set_body_string("<html>auth wall</html>"))
            .mount(&mock)
            .await;
        let api = client_for(&mock.uri(), None);
        let ApiOutcome::Err { status, body } = api.get("/nutrition", &[]).await.unwrap() else {
            panic!()
        };
        assert_eq!(status, 401);
        assert!(
            body["raw_body"]
                .as_str()
                .unwrap()
                .contains("<html>auth wall</html>")
        );
        assert!(body["hint"].as_str().unwrap().contains("login"));
    }

    #[tokio::test]
    async fn post_forwards_json_body() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"totals": {}})),
            )
            .mount(&mock)
            .await;
        let api = client_for(&mock.uri(), None);
        let out = api
            .post("/aggregate", serde_json::json!({"items": []}))
            .await
            .unwrap();
        assert!(matches!(out, ApiOutcome::Ok(_)));
    }
}
