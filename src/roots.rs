//! Choosing the recipe folder when `COOK_RECIPES_DIR` isn't set: which
//! folders are too broad (or not the user's) to treat as a recipe collection,
//! and which of the client's MCP roots to use.

use std::path::{Path, PathBuf};

/// Environment variables agent hosts set to a plugin's install folder. The
/// bare `PLUGIN_ROOT` is a generic name another tool could set for something
/// else; a false positive only makes us ignore that one folder (and say so in
/// the hint), which `COOK_RECIPES_DIR` always overrides.
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

    /// Whether a folder the client sent as a root must not be used. The user
    /// opened it on purpose, so only the hard cases are refused (no manifest
    /// walk: a plugin author's own source tree is a fine recipe folder).
    pub fn rejects_root(&self, dir: &Path) -> bool {
        let dir = canon(dir);
        self.hard_reason(&dir).is_some()
    }

    /// Why the working directory can't be the recipe folder, for the hint.
    pub fn unset_reason(&self, dir: &Path) -> Option<String> {
        let dir = canon(dir);
        if let Some(r) = self.hard_reason(&dir) {
            return Some(r);
        }
        dir.ancestors()
            .take(PLUGIN_MARKER_DEPTH + 1)
            .any(has_plugin_manifest)
            .then(|| plugin_reason(&dir))
    }

    fn hard_reason(&self, dir: &Path) -> Option<String> {
        if dir.parent().is_none() {
            return Some("the working directory is the filesystem root".into());
        }
        if self.home.as_deref().is_some_and(|h| canon(h) == dir) {
            return Some("the working directory is your home folder".into());
        }
        let slashed = dir.to_string_lossy().replace('\\', "/");
        let in_known_cache = ["/plugins/cache/", "/.gemini/extensions/"]
            .iter()
            .any(|m| slashed.contains(m));
        (in_known_cache || self.plugin_roots.iter().any(|r| dir.starts_with(canon(r))))
            .then(|| plugin_reason(dir))
    }
}

fn plugin_reason(dir: &Path) -> String {
    format!(
        "the server started in a plugin install folder ({})",
        dir.display()
    )
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

/// The local folder a `file://` root URI names, or `None` for other schemes,
/// remote hosts, malformed escapes and (on Unix) Windows drive paths.
pub fn path_from_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &uri[7..])?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let path = percent_decode(path)?;
    let b = path.as_bytes();
    let drive = b.len() >= 3 && b[1].is_ascii_alphabetic() && b[2] == b':';
    if cfg!(windows) {
        drive.then(|| PathBuf::from(&path[1..]))
    } else {
        (!drive).then(|| PathBuf::from(path))
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok().filter(|s| !s.contains('\0'))
}

/// The first root that is a local folder that exists and isn't refused by
/// [`Guards::rejects_root`].
pub fn pick_root<'a>(uris: impl IntoIterator<Item = &'a str>, guards: &Guards) -> Option<PathBuf> {
    uris.into_iter()
        .filter_map(path_from_file_uri)
        .find(|p| p.is_dir() && !guards.rejects_root(p))
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
        assert!(g.unset_reason(Path::new("/")).is_some());
        assert!(g.unset_reason(tmp.path()).is_some());
        let proj = tmp.path().join("proj");
        std::fs::create_dir(&proj).unwrap();
        assert!(!g.unset_reason(&proj).is_some());
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
        assert!(g.unset_reason(&claude).is_some());
        assert!(g.unset_reason(&gemini).is_some());
        assert!(g.unset_reason(&agent).is_some());
        assert!(!g.unset_reason(&other).is_some());
        // A subfolder up to four levels down is still the plugin's.
        let deep = claude.join("a/b/c/d");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(g.unset_reason(&deep).is_some());
        let deeper = deep.join("e");
        std::fs::create_dir_all(&deeper).unwrap();
        assert!(!g.unset_reason(&deeper).is_some());
    }

    #[test]
    fn client_roots_skip_the_manifest_guard_but_keep_the_hard_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let g = guards(tmp.path());
        // A folder the user opened that happens to be a plugin's source tree.
        let plugin_src = tmp.path().join("my-plugin");
        std::fs::create_dir_all(plugin_src.join(".claude-plugin")).unwrap();
        std::fs::write(plugin_src.join(".claude-plugin/plugin.json"), "{}").unwrap();
        assert!(!g.rejects_root(&plugin_src));
        assert!(g.unset_reason(&plugin_src).is_some());
        assert_eq!(
            pick_root([uri(&plugin_src).as_str()], &g),
            Some(plugin_src.clone())
        );
        assert!(
            g.unset_reason(&plugin_src)
                .unwrap()
                .contains("plugin install folder")
        );

        assert!(g.rejects_root(Path::new("/")));
        assert!(g.rejects_root(tmp.path()));
        let cached = tmp.path().join(".codex/plugins/cache/x/y/1.0.0");
        let ext = tmp.path().join(".gemini/extensions/cooklang");
        std::fs::create_dir_all(&cached).unwrap();
        std::fs::create_dir_all(&ext).unwrap();
        assert!(g.rejects_root(&cached));
        assert!(g.rejects_root(&ext));
        let env_dir = tmp.path().join("envroot");
        std::fs::create_dir_all(&env_dir).unwrap();
        let g = Guards::from_vars(None, |k| {
            (k == "CLAUDE_PLUGIN_ROOT").then(|| env_dir.clone().into_os_string())
        });
        assert!(g.rejects_root(&env_dir));
    }

    #[test]
    fn unset_reasons_say_why() {
        let tmp = tempfile::tempdir().unwrap();
        let g = guards(tmp.path());
        assert!(
            g.unset_reason(Path::new("/"))
                .unwrap()
                .contains("filesystem root")
        );
        assert!(g.unset_reason(tmp.path()).unwrap().contains("home folder"));
        let cached = tmp.path().join("plugins/cache/x");
        std::fs::create_dir_all(&cached).unwrap();
        assert!(
            g.unset_reason(&cached)
                .unwrap()
                .contains("plugin install folder")
        );
        assert!(g.unset_reason(&tmp.path().join("proj")).is_none());
    }

    fn uri(p: &Path) -> String {
        format!("file://{}", p.display())
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
        assert!(g.unset_reason(&cached).is_some());
        assert!(g.unset_reason(&ext).is_some());
        assert!(!g.unset_reason(&plain).is_some());

        let g = Guards::from_vars(None, |k| {
            (k == "CURSOR_PLUGIN_ROOT").then(|| plain.clone().into_os_string())
        });
        assert!(g.unset_reason(&plain).is_some());
        std::fs::create_dir(plain.join("examples")).unwrap();
        assert!(g.unset_reason(&plain.join("examples")).is_some());
        assert!(g.rejects_root(&plain.join("examples")));
        assert!(!g.unset_reason(tmp.path()).is_some());
    }

    #[test]
    fn file_uris_parse_with_percent_encoding() {
        let p = |u: &str| path_from_file_uri(u);
        assert_eq!(
            p("file:///Users/a/My%20Recipes"),
            Some("/Users/a/My Recipes".into())
        );
        assert_eq!(p("FILE:///tmp/x"), Some("/tmp/x".into()));
        assert_eq!(p("file://localhost/tmp/x"), Some("/tmp/x".into()));
        assert_eq!(p("file:///tmp/caf%C3%A9?q=1#f"), Some("/tmp/café".into()));
        assert_eq!(p("file://server/share/x"), None);
        assert_eq!(p("https://example.com/x"), None);
        assert_eq!(p("file:///tmp/bad%zz"), None);
        assert_eq!(p("file:///tmp/trunc%2"), None);
        assert_eq!(p("file:///tmp/nul%00"), None);
        assert_eq!(p("file://"), None);
        if cfg!(windows) {
            assert_eq!(p("file:///C:/Users/a"), Some("C:/Users/a".into()));
        } else {
            assert_eq!(p("file:///C:/Users/a"), None);
            assert_eq!(p("file:///c%3A/Users/a"), None);
        }
    }

    #[test]
    fn pick_root_takes_first_existing_allowed_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let a = tmp.path().join("a b");
        let b = tmp.path().join("b");
        for d in [&home, &a, &b] {
            std::fs::create_dir_all(d).unwrap();
        }
        let file = tmp.path().join("file.txt");
        std::fs::write(&file, "").unwrap();
        let g = guards(&home);
        let uri = |p: &Path| format!("file://{}", p.display()).replace(' ', "%20");
        let missing = uri(&tmp.path().join("missing"));
        let (home_u, file_u, a_u, b_u) = (uri(&home), uri(&file), uri(&a), uri(&b));
        let roots = [
            "https://example.com",
            missing.as_str(),
            home_u.as_str(),
            file_u.as_str(),
            a_u.as_str(),
            b_u.as_str(),
        ];
        assert_eq!(pick_root(roots, &g), Some(a.clone()));
        assert_eq!(pick_root([missing.as_str(), home_u.as_str()], &g), None);
        assert_eq!(pick_root([], &g), None);
    }
}
