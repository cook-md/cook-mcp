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

#[derive(Debug, serde::Serialize)]
pub struct WriteReport {
    /// Path relative to the root, as later tools expect it.
    pub path: String,
    /// `"created"` or `"overwritten"`.
    pub status: &'static str,
    /// Warnings (and, with `force`, errors) found in the content.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cooklang,
    Config,
}

fn writable_kind(rel: &Utf8Path) -> Option<Kind> {
    match rel.extension() {
        Some("cook" | "menu") => Some(Kind::Cooklang),
        _ if rel == "config/aisle.conf" || rel == "config/pantry.conf" => Some(Kind::Config),
        _ => None,
    }
}

/// `>>` metadata lines (pre-frontmatter Cooklang). The parser still accepts
/// them, so check textually.
pub fn has_legacy_metadata(content: &str) -> bool {
    content.lines().any(|l| l.trim_start().starts_with(">>"))
}

impl Workspace {
    /// Parse `content` as if saved at `rel` and return every problem: parse
    /// errors/warnings plus recipe references that don't resolve in the
    /// collection (reported as errors, same resolution rules as `cook doctor`).
    pub fn check(&self, content: &str, rel: &Utf8Path) -> Vec<Diagnostic> {
        let name = rel.file_stem().unwrap_or("recipe");
        match cookcli_core::parse_recipe_at(content, name, 1.0, Some(rel)) {
            Err(CoreError::Parse { diagnostics, .. }) => diagnostics,
            Err(e) => vec![Diagnostic::error(e.to_string()).at_file(rel)],
            Ok(outcome) => {
                let mut diags = outcome.diagnostics;
                let from = rel.parent().unwrap_or(Utf8Path::new("")).to_owned();
                for ingredient in &outcome.value.ingredients {
                    let Some(r) = ingredient.reference.as_ref() else { continue };
                    let reference = if r.components.is_empty() { r.name.clone() } else { r.path("/") };
                    let found = cookcli_core::resolve_reference(&from, &reference).is_some_and(|p| {
                        cookcli_core::find::get_recipe(&self.root, p.as_str()).is_ok()
                    });
                    if !found {
                        diags.push(
                            Diagnostic::error(format!(
                                "recipe reference `{reference}` does not resolve to a recipe in the collection"
                            ))
                            .at_file(rel),
                        );
                    }
                }
                diags
            }
        }
    }

    /// Validate, then atomically write `content` to `path`.
    pub fn write(&self, path: &str, content: &str, force: bool) -> Result<WriteReport, WorkspaceError> {
        let rel = self.relative(path)?;
        let kind = writable_kind(&rel).ok_or_else(|| WorkspaceError::Extension(path.into()))?;
        let full = self.resolve(path)?;
        let diagnostics = match kind {
            Kind::Config => Vec::new(),
            Kind::Cooklang => {
                if has_legacy_metadata(content) {
                    return Err(WorkspaceError::LegacyMetadata);
                }
                self.check(content, &rel)
            }
        };
        if !force && diagnostics.iter().any(|d| d.severity == Severity::Error) {
            return Err(WorkspaceError::Invalid { diagnostics });
        }
        let existed = full.exists();
        write_atomically(&full, content)?;
        Ok(WriteReport {
            path: rel.to_string(),
            status: if existed { "overwritten" } else { "created" },
            diagnostics,
        })
    }
}

fn write_atomically(path: &Utf8Path, content: &str) -> Result<(), WorkspaceError> {
    let io = |e: std::io::Error| WorkspaceError::Io(e.to_string());
    let dir = path.parent().expect("resolved paths sit inside the root");
    // Safe: `resolve` already checked the deepest existing ancestor.
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io)?;
    std::io::Write::write_all(&mut tmp, content.as_bytes()).map_err(io)?;
    tmp.persist(path).map_err(|e| io(e.error))?;
    Ok(())
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

    const GOOD: &str = "---\nservings: 1\n---\nFry @eggs{2} in @butter{5%g}.\n";

    #[test]
    fn write_creates_then_overwrites() {
        let (_d, ws) = fixture_workspace();
        let r = ws.write("Lunch/Eggs.cook", GOOD, false).unwrap();
        assert_eq!((r.path.as_str(), r.status), ("Lunch/Eggs.cook", "created"));
        let r = ws.write("Lunch/Eggs.cook", GOOD, false).unwrap();
        assert_eq!(r.status, "overwritten");
        assert_eq!(std::fs::read_to_string(ws.root().join("Lunch/Eggs.cook")).unwrap(), GOOD);
    }

    #[test]
    fn invalid_cooklang_is_refused_unless_forced() {
        let (_d, ws) = fixture_workspace();
        let bad = "Add @{1%tsp} to the pot.\n";
        let Err(WorkspaceError::Invalid { diagnostics }) = ws.write("Bad.cook", bad, false) else {
            panic!("expected Invalid");
        };
        assert!(diagnostics.iter().any(|d| d.severity == Severity::Error));
        assert!(!ws.root().join("Bad.cook").exists());
        let r = ws.write("Bad.cook", bad, true).unwrap();
        assert_eq!(r.status, "created");
        assert!(r.diagnostics.iter().any(|d| d.severity == Severity::Error));
    }

    #[test]
    fn legacy_metadata_is_refused_even_with_force() {
        let (_d, ws) = fixture_workspace();
        let legacy = ">> servings: 2\nFry @eggs{2}.\n";
        assert!(matches!(ws.write("Old.cook", legacy, true), Err(WorkspaceError::LegacyMetadata)));
    }

    #[test]
    fn broken_recipe_reference_is_an_error() {
        let (_d, ws) = fixture_workspace();
        let menu = "==Tue==\n\nDinner: \\\n- @./Dinner/Missing{2%servings}\n";
        let Err(WorkspaceError::Invalid { diagnostics }) = ws.write("Plans/Tue.menu", menu, false) else {
            panic!("expected Invalid for a broken reference");
        };
        assert!(diagnostics.iter().any(|d| d.message.contains("./Dinner/Missing")));
        let ok = "==Tue==\n\nDinner: \\\n- @./Dinner/Pasta{2%servings}\n";
        assert!(ws.write("Plans/Tue.menu", ok, false).is_ok());
    }

    #[test]
    fn only_cooklang_and_config_files_are_writable() {
        let (_d, ws) = fixture_workspace();
        assert!(matches!(ws.write("notes.txt", "hi", true), Err(WorkspaceError::Extension(_))));
        assert!(matches!(ws.write("config/other.conf", "x", true), Err(WorkspaceError::Extension(_))));
        assert!(ws.write("config/aisle.conf", "[produce]\nleek\n", false).is_ok());
    }

    #[test]
    fn check_reports_without_writing() {
        let (_d, ws) = fixture_workspace();
        let diags = ws.check("Add @{1%tsp}.\n", Utf8Path::new("X.cook"));
        assert!(diags.iter().any(|d| d.severity == Severity::Error));
        assert!(!ws.root().join("X.cook").exists());
    }
}
