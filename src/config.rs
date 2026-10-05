/// Runtime configuration, read once at startup from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    /// Nutrition service base URL (`NUTRITION_API_URL`).
    pub api_url: String,
    /// cook.md web app base URL for login, entitlements and import (`COOKMD_BASE_URL`).
    pub cookmd_url: String,
    /// Org API key / pre-made token (`NUTRITION_API_TOKEN`); beats stored login.
    pub token_override: Option<String>,
    /// Override for the auth.json path (`COOK_MCP_AUTH_PATH`, or the legacy
    /// `NUTRITION_MCP_AUTH_PATH`).
    pub auth_path: Option<std::path::PathBuf>,
    /// Recipe root (`COOK_RECIPES_DIR`); `None` means the working directory.
    pub recipes_dir: Option<std::path::PathBuf>,
}

/// Plans differ per tool (nutrition and photo import are Basic+), so link the
/// pricing page rather than one checkout.
pub const PRICING_PATH: &str = "/pricing?utm_source=cook-mcp";

impl Config {
    pub fn from_env() -> Self {
        Self::from_vars(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let trim = |s: String| s.trim_end_matches('/').to_string();
        Self {
            api_url: get("NUTRITION_API_URL")
                .map(trim)
                .unwrap_or_else(|| "https://nutrition.cook.md".into()),
            cookmd_url: get("COOKMD_BASE_URL")
                .map(trim)
                .unwrap_or_else(|| "https://cook.md".into()),
            token_override: get("NUTRITION_API_TOKEN"),
            auth_path: get("COOK_MCP_AUTH_PATH")
                .or_else(|| get("NUTRITION_MCP_AUTH_PATH"))
                .map(std::path::PathBuf::from),
            recipes_dir: get("COOK_RECIPES_DIR").map(std::path::PathBuf::from),
        }
    }

    pub fn pricing_url(&self) -> String {
        format!("{}{}", self.cookmd_url, PRICING_PATH)
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
        assert!(c.recipes_dir.is_none());
        assert!(c.auth_path.is_none());
    }

    #[test]
    fn env_overrides_and_trailing_slash_trimmed() {
        let c = Config::from_vars(|k| match k {
            "NUTRITION_API_URL" => Some("http://127.0.0.1:8080/".into()),
            "COOKMD_BASE_URL" => Some("http://127.0.0.1:3000".into()),
            "NUTRITION_API_TOKEN" => Some("org-key".into()),
            "COOK_RECIPES_DIR" => Some("/tmp/recipes".into()),
            _ => None,
        });
        assert_eq!(c.api_url, "http://127.0.0.1:8080");
        assert_eq!(c.cookmd_url, "http://127.0.0.1:3000");
        assert_eq!(c.token_override.as_deref(), Some("org-key"));
        assert_eq!(c.recipes_dir.as_deref(), Some(std::path::Path::new("/tmp/recipes")));
    }

    #[test]
    fn auth_path_prefers_new_var_and_accepts_legacy_alias() {
        let both = Config::from_vars(|k| match k {
            "COOK_MCP_AUTH_PATH" => Some("/new.json".into()),
            "NUTRITION_MCP_AUTH_PATH" => Some("/old.json".into()),
            _ => None,
        });
        assert_eq!(both.auth_path.as_deref(), Some(std::path::Path::new("/new.json")));
        let legacy = Config::from_vars(|k| (k == "NUTRITION_MCP_AUTH_PATH").then(|| "/old.json".into()));
        assert_eq!(legacy.auth_path.as_deref(), Some(std::path::Path::new("/old.json")));
    }

    #[test]
    fn pricing_url_is_tagged_for_attribution() {
        let c = Config::from_vars(|_| None);
        assert_eq!(c.pricing_url(), "https://cook.md/pricing?utm_source=cook-mcp");
    }
}
