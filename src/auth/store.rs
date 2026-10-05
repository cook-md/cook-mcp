use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Persisted login: `~/.config/cook-mcp/auth.json`, chmod 0600 — same
/// convention as cookbot's `~/.cookbot/auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub token: String,
    pub email: Option<String>,
    /// Unix seconds from the JWT `exp` claim.
    pub expires_at: i64,
}

impl StoredAuth {
    /// Build from a cook.md JWT by decoding (NOT verifying — only cook.md
    /// holds the HS256 secret) the payload for `email`/`exp`.
    pub fn from_token(token: &str) -> anyhow::Result<Self> {
        let payload_b64 = token
            .split('.')
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("not a JWT"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64)?;
        let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
        Ok(Self {
            token: token.to_string(),
            email: payload["email"].as_str().map(str::to_string),
            expires_at: payload["exp"].as_i64().unwrap_or(0),
        })
    }

    /// Renew when within 24h of expiry (tokens live 100 days).
    pub fn expiring_soon(&self, now_unix: i64) -> bool {
        self.expires_at - now_unix < 24 * 3600
    }
}

pub fn default_path() -> PathBuf {
    config_root().join("cook-mcp").join("auth.json")
}

/// Where nutrition-mcp (≤0.1) kept its login.
pub fn legacy_path() -> PathBuf {
    config_root().join("nutrition-mcp").join("auth.json")
}

fn config_root() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Move a nutrition-mcp login to the cook-mcp path once, so users upgrading
/// through the npm shim stay logged in. Never overwrites an existing login.
/// The copy is atomic (temp file in the destination dir, then rename), private
/// (0600 on unix), and the legacy file is removed afterwards (best effort) so a
/// later logout cannot be undone by re-migrating.
pub fn migrate_legacy(new: &Path, legacy: &Path) -> anyhow::Result<bool> {
    use std::io::Write as _;
    if new.exists() || !legacy.exists() {
        return Ok(false);
    }
    let dir = new
        .parent()
        .ok_or_else(|| anyhow::anyhow!("auth path has no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let bytes = std::fs::read(legacy)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(&bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    tmp.persist(new).map_err(|e| e.error)?;
    let _ = std::fs::remove_file(legacy);
    Ok(true)
}

pub fn load(path: &Path) -> anyhow::Result<Option<StoredAuth>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(serde_json::from_str(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn save(path: &Path, auth: &StoredAuth) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Create the file already locked down to 0600 rather than writing with
    // the umask-default mode and chmod-ing after: that would leave a brief
    // window (and a pre-existing looser-permission file) where the JWT is
    // world/group-readable.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(&serde_json::to_vec_pretty(auth)?)?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, serde_json::to_vec_pretty(auth)?)?;
    Ok(())
}

pub fn clear(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // JWT with payload {"uid":1,"email":"a@b.c","exp":...}, unsigned test
    // token — decode only reads the middle segment.
    fn test_jwt(exp: i64) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = serde_json::json!({"uid": 1, "email": "a@b.c", "exp": exp});
        format!("h.{}.s", b64.encode(payload.to_string()))
    }

    #[test]
    fn save_load_roundtrip_with_0600_perms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let creds = StoredAuth::from_token(&test_jwt(4102444800)).unwrap();
        save(&path, &creds).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.email.as_deref(), Some("a@b.c"));
        assert_eq!(loaded.expires_at, 4102444800);
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("nope.json")).unwrap().is_none());
    }

    #[test]
    fn expiring_soon_detects_24h_window() {
        let now = 1_800_000_000i64;
        let fresh = StoredAuth {
            token: "t".into(),
            email: None,
            expires_at: now + 200_000,
        };
        let stale = StoredAuth {
            token: "t".into(),
            email: None,
            expires_at: now + 3_600,
        };
        assert!(!fresh.expiring_soon(now));
        assert!(stale.expiring_soon(now));
    }

    #[test]
    fn from_token_rejects_non_jwt() {
        assert!(StoredAuth::from_token("not-a-token").is_err());
    }

    #[test]
    fn from_token_rejects_invalid_base64_payload() {
        assert!(StoredAuth::from_token("h.not!valid!base64.s").is_err());
    }

    #[test]
    fn from_token_rejects_non_json_payload() {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = b64.encode("not json");
        let token = format!("h.{payload}.s");
        assert!(StoredAuth::from_token(&token).is_err());
    }

    #[test]
    fn clear_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        save(
            &path,
            &StoredAuth::from_token(&test_jwt(4102444800)).unwrap(),
        )
        .unwrap();
        clear(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn migrate_copies_legacy_login_once() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("nutrition-mcp/auth.json");
        let new = dir.path().join("cook-mcp/auth.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, r#"{"token":"t"}"#).unwrap();

        assert!(migrate_legacy(&new, &legacy).unwrap());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), r#"{"token":"t"}"#);

        std::fs::write(&legacy, r#"{"token":"newer"}"#).unwrap();
        assert!(
            !migrate_legacy(&new, &legacy).unwrap(),
            "never overwrites an existing login"
        );
        assert_eq!(std::fs::read_to_string(&new).unwrap(), r#"{"token":"t"}"#);
    }

    #[test]
    fn migrate_is_a_noop_without_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let new = dir.path().join("cook-mcp/auth.json");
        assert!(!migrate_legacy(&new, &dir.path().join("missing.json")).unwrap());
        assert!(!new.exists());
    }

    #[test]
    fn default_path_is_cook_mcp() {
        assert!(default_path().ends_with("cook-mcp/auth.json"));
        assert!(legacy_path().ends_with("nutrition-mcp/auth.json"));
    }

    #[test]
    fn logout_after_migrate_is_not_resurrected() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("nutrition-mcp/auth.json");
        let new = dir.path().join("cook-mcp/auth.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, r#"{"token":"t"}"#).unwrap();
        assert!(migrate_legacy(&new, &legacy).unwrap());
        assert!(!legacy.exists(), "legacy login is moved, not copied");
        clear(&new).unwrap();
        clear(&legacy).unwrap();
        assert!(!migrate_legacy(&new, &legacy).unwrap());
        assert!(!new.exists());
    }

    #[cfg(unix)]
    #[test]
    fn migrated_file_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("old.json");
        let new = dir.path().join("cook-mcp/auth.json");
        std::fs::write(&legacy, "{}").unwrap();
        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(migrate_legacy(&new, &legacy).unwrap());
        let mode = std::fs::metadata(&new).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn migrate_errors_when_parent_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("old.json");
        std::fs::write(&legacy, "{}").unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        assert!(migrate_legacy(&blocker.join("auth.json"), &legacy).is_err());
        assert!(legacy.exists());
    }
}
