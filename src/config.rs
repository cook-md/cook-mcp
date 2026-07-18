/// Runtime configuration, read once at startup from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    /// Nutrition service base URL (`NUTRITION_API_URL`).
    pub api_url: String,
    /// cook.md web app base URL for login/subscription (`COOKMD_BASE_URL`).
    pub cookmd_url: String,
    /// Org API key / pre-made token (`NUTRITION_API_TOKEN`); beats stored login.
    pub token_override: Option<String>,
    /// Override for the auth.json path (`NUTRITION_MCP_AUTH_PATH`, tests).
    pub auth_path: Option<std::path::PathBuf>,
}

pub const DEFAULT_CHECKOUT_PLAN: &str = "pro_early_adopter_v1";

impl Config {
    pub fn from_env() -> Self {
        Self::from_vars(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let trim = |s: String| s.trim_end_matches('/').to_string();
        Self {
            api_url: get("NUTRITION_API_URL")
                .map(&trim)
                .unwrap_or_else(|| "https://nutrition.cook.md".into()),
            cookmd_url: get("COOKMD_BASE_URL")
                .map(&trim)
                .unwrap_or_else(|| "https://cook.md".into()),
            token_override: get("NUTRITION_API_TOKEN"),
            auth_path: get("NUTRITION_MCP_AUTH_PATH").map(std::path::PathBuf::from),
        }
    }

    pub fn checkout_url(&self) -> String {
        format!("{}/checkout/{}", self.cookmd_url, DEFAULT_CHECKOUT_PLAN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_env_unset() {
        let c = Config::from_vars(|_| None);
        assert_eq!(c.api_url, "https://nutrition.cook.md");
        assert_eq!(c.cookmd_url, "https://cook.md");
        assert!(c.token_override.is_none());
    }

    #[test]
    fn env_overrides_and_trailing_slash_trimmed() {
        let c = Config::from_vars(|k| match k {
            "NUTRITION_API_URL" => Some("http://127.0.0.1:8080/".into()),
            "COOKMD_BASE_URL" => Some("http://127.0.0.1:3000".into()),
            "NUTRITION_API_TOKEN" => Some("org-key".into()),
            _ => None,
        });
        assert_eq!(c.api_url, "http://127.0.0.1:8080");
        assert_eq!(c.cookmd_url, "http://127.0.0.1:3000");
        assert_eq!(c.token_override.as_deref(), Some("org-key"));
    }

    #[test]
    fn checkout_url_appends_plan() {
        let c = Config::from_vars(|_| None);
        assert_eq!(
            c.checkout_url(),
            "https://cook.md/checkout/pro_early_adopter_v1"
        );
    }
}
