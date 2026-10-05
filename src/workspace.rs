//! The recipe root every local tool works inside: path safety, validation
//! before writing, atomic writes.

use anyhow::{Context as _, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use cookcli_core::{Context, CoreError, Diagnostic, Severity};

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical (symlinks resolved), so `starts_with` checks are meaningful.
    root: Utf8PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("path `{0}` is not a plain relative path inside the recipe root (no `..`, no leading `./`)")]
    UnsafePath(String),
    #[error("path `{0}` resolves outside the recipe root")]
    Escapes(String),
    #[error(
        "cannot write `{0}`: only .cook and .menu files, config/aisle.conf and config/pantry.conf can be written"
    )]
    Extension(String),
    #[error(
        "content uses the deprecated `>>` metadata syntax; put metadata in YAML frontmatter between `---` lines instead"
    )]
    LegacyMetadata,
    #[error("content has Cooklang errors; fix them, or pass force: true to save anyway")]
    Invalid { diagnostics: Vec<Diagnostic> },
    #[error("{0}")]
    Io(String),
}

impl Workspace {
    pub fn new(root: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        let canon = std::fs::canonicalize(root)
            .with_context(|| format!("recipe root {} does not exist", root.display()))?;
        let root = Utf8PathBuf::from_path_buf(canon)
            .map_err(|p| anyhow!("recipe root {} is not valid UTF-8", p.display()))?;
        Ok(Self { root })
    }

    /// `COOK_RECIPES_DIR`, else the process working directory (MCP clients
    /// launch stdio servers in the project folder).
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        match &cfg.recipes_dir {
            Some(dir) => Self::new(dir),
            None => Self::new(std::env::current_dir()?),
        }
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// A `cookcli-core` context with aisle and pantry discovered the way the
    /// CLI does (`<root>/config/`, then the global config directory).
    pub fn context(&self) -> Context {
        Context::discover(self.root.clone())
    }

    /// The caller's path relative to the root. Absolute paths are accepted
    /// only when they sit under the (canonical) root.
    pub fn relative(&self, path: &str) -> Result<Utf8PathBuf, WorkspaceError> {
        let p = Utf8Path::new(path);
        let rel = if p.is_absolute() {
            p.strip_prefix(&self.root)
                .map_err(|_| WorkspaceError::Escapes(path.into()))?
                .to_owned()
        } else {
            p.to_owned()
        };
        if rel.as_str().is_empty() || !cookcli_core::is_safe_relative_path(rel.as_str()) {
            return Err(WorkspaceError::UnsafePath(path.into()));
        }
        Ok(rel)
    }

    /// Absolute path for `path`, refusing anything that escapes the root,
    /// including through a symlinked folder.
    pub fn resolve(&self, path: &str) -> Result<Utf8PathBuf, WorkspaceError> {
        let rel = self.relative(path)?;
        let full = self.root.join(&rel);
        // The deepest ancestor that exists decides where the path really
        // lands; the root itself always exists, so this terminates.
        let mut probe = full.as_path();
        while !probe.exists() {
            probe = probe.parent().unwrap_or(&self.root);
        }
        let canon = std::fs::canonicalize(probe).map_err(|e| WorkspaceError::Io(e.to_string()))?;
        if !canon.starts_with(self.root.as_std_path()) {
            return Err(WorkspaceError::Escapes(path.into()));
        }
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture_workspace;

    #[test]
    fn relative_paths_resolve_inside_root() {
        let (_d, ws) = fixture_workspace();
        let p = ws.resolve("Breakfast/Pancakes.cook").unwrap();
        assert_eq!(p, ws.root().join("Breakfast/Pancakes.cook"));
    }

    #[test]
    fn absolute_path_inside_root_is_accepted() {
        let (_d, ws) = fixture_workspace();
        let abs = ws.root().join("Breakfast/Pancakes.cook");
        assert_eq!(ws.relative(abs.as_str()).unwrap(), "Breakfast/Pancakes.cook");
    }

    #[test]
    fn escapes_are_refused() {
        let (_d, ws) = fixture_workspace();
        for bad in ["../x.cook", "./x.cook", "", "/etc/passwd", "a/../../x.cook"] {
            assert!(ws.resolve(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folder_pointing_outside_is_refused() {
        let (_d, ws) = fixture_workspace();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), ws.root().join("Linked")).unwrap();
        assert!(matches!(ws.resolve("Linked/x.cook"), Err(WorkspaceError::Escapes(_))));
    }

    #[test]
    fn new_paths_in_new_folders_resolve() {
        let (_d, ws) = fixture_workspace();
        assert!(ws.resolve("Lunch/Soup/Leek.cook").is_ok());
    }
}
