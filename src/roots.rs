//! Choosing the recipe folder when `COOK_RECIPES_DIR` isn't set: which
//! folders are too broad (or not the user's) to treat as a recipe collection,
//! and which of the client's MCP roots to use.

use std::path::{Path, PathBuf};

/// Environment variables agent hosts set to a plugin's install folder.
pub const PLUGIN_ROOT_VARS: [&str; 3] = ["CLAUDE_PLUGIN_ROOT", "CURSOR_PLUGIN_ROOT", "PLUGIN_ROOT"];

/// How many ancestors above a folder to search for plugin manifests (a
/// plugin's server may be started in a subfolder of its install folder).
const PLUGIN_MARKER_DEPTH: usize = 4;

/// Folders that are never a recipe collection: `/`, the home directory, and
/// an agent plugin's install folder (Codex, Cursor, VS Code Agent Plugins and
/// Claude Code start a plugin's stdio server there, and wipe it on update).
#[derive(Debug, Clone, Default)]
pub struct Guards {
    pub home: Option<PathBuf>,
    /// Values of [`PLUGIN_ROOT_VARS`] that are set.
    pub plugin_roots: Vec<PathBuf>,
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

impl Guards {
    pub fn from_env() -> Self {
        Self::from_vars(dirs::home_dir(), |k| {
            std::env::var_os(k).filter(|v| !v.is_empty())
        })
    }

    pub fn from_vars(
        home: Option<PathBuf>,
        get: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Self {
        Self {
            home,
            plugin_roots: PLUGIN_ROOT_VARS
                .iter()
                .filter_map(|k| get(k).map(PathBuf::from))
                .collect(),
        }
    }

    /// Whether `dir` must not be used as the recipe root.
    pub fn rejects(&self, dir: &Path) -> bool {
        let dir = canon(dir);
        dir.parent().is_none()
            || self.home.as_deref().is_some_and(|h| canon(h) == dir)
            || self.is_plugin_dir(&dir)
    }

    fn is_plugin_dir(&self, dir: &Path) -> bool {
        let slashed = dir.to_string_lossy().replace('\\', "/");
        if ["/plugins/cache/", "/.gemini/extensions/"]
            .iter()
            .any(|m| slashed.contains(m))
        {
            return true;
        }
        if self.plugin_roots.iter().any(|r| dir.starts_with(canon(r))) {
            return true;
        }
        dir.ancestors()
            .take(PLUGIN_MARKER_DEPTH + 1)
            .any(has_plugin_manifest)
    }
}

/// A Claude Code plugin, an Agent Plugins (agent-plugins.org) plugin, or a
/// Gemini CLI extension.
fn has_plugin_manifest(dir: &Path) -> bool {
    if dir.join(".claude-plugin/plugin.json").is_file()
        || dir.join("gemini-extension.json").is_file()
    {
        return true;
    }
    std::fs::read_to_string(dir.join("plugin.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("$schema")?
                .as_str()
                .map(|s| s.contains("agent-plugins.org"))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guards(home: &Path) -> Guards {
        Guards {
            home: Some(home.to_path_buf()),
            plugin_roots: vec![],
        }
    }

    #[test]
    fn rejects_filesystem_root_and_home() {
        let tmp = tempfile::tempdir().unwrap();
        let g = guards(tmp.path());
        assert!(g.rejects(Path::new("/")));
        assert!(g.rejects(tmp.path()));
        let proj = tmp.path().join("proj");
        std::fs::create_dir(&proj).unwrap();
        assert!(!g.rejects(&proj));
    }

    #[test]
    fn rejects_plugin_install_folders_by_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let g = guards(Path::new("/nonexistent-home"));
        let mk = |name: &str, file: &str, body: &str| {
            let dir = tmp.path().join(name);
            let f = dir.join(file);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
            dir
        };
        let claude = mk("claude", ".claude-plugin/plugin.json", "{}");
        let gemini = mk("gemini", "gemini-extension.json", "{}");
        let agent = mk(
            "agent",
            "plugin.json",
            r#"{"$schema": "https://agent-plugins.org/schema/plugin.json"}"#,
        );
        let other = mk("other", "plugin.json", r#"{"name": "not a plugin"}"#);
        assert!(g.rejects(&claude));
        assert!(g.rejects(&gemini));
        assert!(g.rejects(&agent));
        assert!(!g.rejects(&other));
        // A subfolder up to four levels down is still the plugin's.
        let deep = claude.join("a/b/c/d");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(g.rejects(&deep));
        let deeper = deep.join("e");
        std::fs::create_dir_all(&deeper).unwrap();
        assert!(!g.rejects(&deeper));
    }

    #[test]
    fn rejects_plugin_cache_paths_and_plugin_root_env() {
        let tmp = tempfile::tempdir().unwrap();
        let g = guards(Path::new("/nonexistent-home"));
        let cached = tmp
            .path()
            .join(".codex/plugins/cache/cooklang-skills/cooklang/2.0.0");
        let ext = tmp.path().join(".gemini/extensions/cooklang");
        let plain = tmp.path().join("recipes");
        for d in [&cached, &ext, &plain] {
            std::fs::create_dir_all(d).unwrap();
        }
        assert!(g.rejects(&cached));
        assert!(g.rejects(&ext));
        assert!(!g.rejects(&plain));

        let g = Guards::from_vars(None, |k| {
            (k == "CURSOR_PLUGIN_ROOT").then(|| plain.clone().into_os_string())
        });
        assert!(g.rejects(&plain));
        std::fs::create_dir(plain.join("examples")).unwrap();
        assert!(g.rejects(&plain.join("examples")));
        assert!(!g.rejects(tmp.path()));
    }
}
