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
    /// The root as the caller gave it (absolute, symlinks kept), so absolute
    /// paths built from it are accepted too.
    given_root: Utf8PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(
        "path `{0}` is not a plain relative path inside the recipe root (no `..`, no leading `./`)"
    )]
    UnsafePath(String),
    #[error("path `{path}` resolves outside the recipe root {root}")]
    Escapes { path: String, root: String },
    #[error(
        "cannot write `{0}`: only .cook and .menu files, config/aisle.conf, config/pantry.conf and reports/**/*.jinja templates can be written"
    )]
    Extension(String),
    #[error(
        "write_config writes config/aisle.conf, config/pantry.conf or a report template under reports/ (*.jinja); `{0}` is none of those (use write_recipe / write_menu for Cooklang)"
    )]
    NotConfig(String),
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
        let given = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let given_root = Utf8PathBuf::from_path_buf(given)
            .map_err(|p| anyhow!("recipe root {} is not valid UTF-8", p.display()))?;
        let canon = std::fs::canonicalize(root)
            .with_context(|| format!("recipe root {} does not exist", root.display()))?;
        let root = Utf8PathBuf::from_path_buf(canon)
            .map_err(|p| anyhow!("recipe root {} is not valid UTF-8", p.display()))?;
        Ok(Self { root, given_root })
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
        let unsafe_path = || WorkspaceError::UnsafePath(path.into());
        if path.contains(['\\', '\0']) || path.ends_with('/') {
            return Err(unsafe_path());
        }
        let p = Utf8Path::new(path);
        let rel = if p.is_absolute() {
            p.strip_prefix(&self.root)
                .or_else(|_| p.strip_prefix(&self.given_root))
                .map_err(|_| WorkspaceError::Escapes {
                    path: path.into(),
                    root: self.root.to_string(),
                })?
                .to_owned()
        } else {
            p.to_owned()
        };
        if rel.as_str().is_empty() || !cookcli_core::is_safe_relative_path(rel.as_str()) {
            return Err(unsafe_path());
        }
        // Rebuild from components so `a//b.cook` is reported as `a/b.cook`.
        Ok(rel
            .components()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("/")
            .into())
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
            return Err(WorkspaceError::Escapes {
                path: path.into(),
                root: self.root.to_string(),
            });
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
    Aisle,
    Pantry,
    /// A jinja report template under `reports/`; free text, not validated.
    Template,
}

fn writable_kind(rel: &Utf8Path) -> Option<Kind> {
    match rel.extension() {
        Some("cook" | "menu") => Some(Kind::Cooklang),
        Some("jinja") if rel.starts_with("reports") => Some(Kind::Template),
        _ if rel == "config/aisle.conf" => Some(Kind::Aisle),
        _ if rel == "config/pantry.conf" => Some(Kind::Pantry),
        _ => None,
    }
}

/// Lenient-parse problems in an aisle/pantry config, all as warnings: the
/// apps parse these files leniently too, so a bad line is skipped, not fatal.
fn config_warnings(
    report: &cookcli_core::cooklang::error::SourceReport,
    rel: &Utf8Path,
) -> Vec<Diagnostic> {
    report
        .iter()
        .map(|d| Diagnostic::warning(d.message.to_string()).at_file(rel))
        .collect()
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
                    let Some(r) = ingredient.reference.as_ref() else {
                        continue;
                    };
                    let reference = if r.components.is_empty() {
                        r.name.clone()
                    } else {
                        r.path("/")
                    };
                    let found =
                        cookcli_core::resolve_reference(&from, &reference).is_some_and(|p| {
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

    /// Write an aisle/pantry config or a report template (never Cooklang).
    /// Config problems come back as warnings; nothing here blocks the write.
    pub fn write_config(&self, path: &str, content: &str) -> Result<WriteReport, WorkspaceError> {
        let rel = self.relative(path)?;
        match writable_kind(&rel) {
            Some(Kind::Aisle | Kind::Pantry | Kind::Template) => self.write(path, content, false),
            _ => Err(WorkspaceError::NotConfig(path.into())),
        }
    }

    /// Validate, then atomically write `content` to `path`.
    pub fn write(
        &self,
        path: &str,
        content: &str,
        force: bool,
    ) -> Result<WriteReport, WorkspaceError> {
        let rel = self.relative(path)?;
        let kind = writable_kind(&rel).ok_or_else(|| WorkspaceError::Extension(path.into()))?;
        let full = self.resolve(path)?;
        let diagnostics = match kind {
            Kind::Template => Vec::new(),
            Kind::Aisle => config_warnings(
                cookcli_core::cooklang::aisle::parse_lenient(content).report(),
                &rel,
            ),
            Kind::Pantry => config_warnings(
                cookcli_core::cooklang::pantry::parse_lenient(content).report(),
                &rel,
            ),
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
        // `resolve` proved an existing target sits inside the root; write to
        // what a symlink points at so the link stays a link.
        let target = if existed {
            std::fs::canonicalize(&full)
                .ok()
                .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
                .unwrap_or_else(|| full.clone())
        } else {
            full.clone()
        };
        write_atomically(&target, content)?;
        Ok(WriteReport {
            path: rel.to_string(),
            status: if existed { "overwritten" } else { "created" },
            diagnostics,
        })
    }
}

/// Same approach as `cookcli-core`'s `fs_atomic`: a temp file beside the
/// target, fsynced, given the old file's permissions (a new file gets the
/// process umask default), then renamed over it.
fn write_atomically(path: &Utf8Path, content: &str) -> Result<(), WorkspaceError> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let io = |e: std::io::Error| WorkspaceError::Io(e.to_string());
    let dir = path.parent().expect("resolved paths sit inside the root");
    // Safe: `resolve` already checked the deepest existing ancestor.
    std::fs::create_dir_all(dir).map_err(io)?;
    let name = path.file_name().unwrap_or("file");
    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp, meta.permissions())?;
        }
        std::fs::rename(&temp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(io(e));
    }
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
        assert_eq!(
            ws.relative(abs.as_str()).unwrap(),
            "Breakfast/Pancakes.cook"
        );
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
        assert!(matches!(
            ws.resolve("Linked/x.cook"),
            Err(WorkspaceError::Escapes { .. })
        ));
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
        assert_eq!(
            std::fs::read_to_string(ws.root().join("Lunch/Eggs.cook")).unwrap(),
            GOOD
        );
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
        assert!(matches!(
            ws.write("Old.cook", legacy, true),
            Err(WorkspaceError::LegacyMetadata)
        ));
    }

    #[test]
    fn broken_recipe_reference_is_an_error() {
        let (_d, ws) = fixture_workspace();
        let menu = "==Tue==\n\nDinner: \\\n- @./Dinner/Missing{2%servings}\n";
        let Err(WorkspaceError::Invalid { diagnostics }) = ws.write("Plans/Tue.menu", menu, false)
        else {
            panic!("expected Invalid for a broken reference");
        };
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("./Dinner/Missing"))
        );
        let ok = "==Tue==\n\nDinner: \\\n- @./Dinner/Pasta{2%servings}\n";
        assert!(ws.write("Plans/Tue.menu", ok, false).is_ok());
    }

    #[test]
    fn only_cooklang_and_config_files_are_writable() {
        let (_d, ws) = fixture_workspace();
        assert!(matches!(
            ws.write("notes.txt", "hi", true),
            Err(WorkspaceError::Extension(_))
        ));
        assert!(matches!(
            ws.write("config/other.conf", "x", true),
            Err(WorkspaceError::Extension(_))
        ));
        assert!(
            ws.write("config/aisle.conf", "[produce]\nleek\n", false)
                .is_ok()
        );
    }

    #[test]
    fn templates_under_reports_are_writable_without_cooklang_checks() {
        let (_d, ws) = fixture_workspace();
        let r = ws
            .write_config("reports/nutrition/week.md.jinja", "{{ >> not cooklang }}")
            .unwrap();
        assert_eq!(r.status, "created");
        assert!(ws.root().join("reports/nutrition/week.md.jinja").exists());
        for bad in [
            "reports/x.txt",
            "other/x.jinja",
            "x.jinja",
            "Dinner/Pasta.cook",
        ] {
            assert!(
                matches!(ws.write_config(bad, "x"), Err(WorkspaceError::NotConfig(_))),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn config_files_can_be_created_and_report_lenient_warnings() {
        let (_d, ws) = fixture_workspace();
        std::fs::remove_file(ws.root().join("config/pantry.conf")).unwrap();
        let r = ws
            .write_config("config/pantry.conf", "[fridge]\nmilk = \"1%l\"\n")
            .unwrap();
        assert_eq!(r.status, "created");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        // Broken TOML is still saved, with warnings, never refused.
        let r = ws
            .write_config("config/pantry.conf", "[fridge\nmilk = \n")
            .unwrap();
        assert!(!r.diagnostics.is_empty());
        assert!(
            r.diagnostics
                .iter()
                .all(|d| d.severity == Severity::Warning)
        );
        let r = ws
            .write_config("config/aisle.conf", "[produce]\nleek\n")
            .unwrap();
        assert_eq!(r.status, "overwritten");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    }

    #[test]
    fn check_reports_without_writing() {
        let (_d, ws) = fixture_workspace();
        let diags = ws.check("Add @{1%tsp}.\n", Utf8Path::new("X.cook"));
        assert!(diags.iter().any(|d| d.severity == Severity::Error));
        assert!(!ws.root().join("X.cook").exists());
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_keeps_permissions_and_new_files_are_not_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, ws) = fixture_workspace();
        let existing = ws.root().join("Breakfast/Pancakes.cook");
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();
        ws.write("Breakfast/Pancakes.cook", GOOD, false).unwrap();
        let mode = |p: &Utf8Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&existing), 0o644);
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o600)).unwrap();
        ws.write("Breakfast/Pancakes.cook", GOOD, false).unwrap();
        assert_eq!(mode(&existing), 0o600);
        ws.write("New.cook", GOOD, false).unwrap();
        assert_ne!(mode(&ws.root().join("New.cook")), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn write_goes_through_in_root_symlink() {
        let (_d, ws) = fixture_workspace();
        std::os::unix::fs::symlink("Shared/Tomato Sauce.cook", ws.root().join("Link.cook"))
            .unwrap();
        ws.write("Link.cook", GOOD, false).unwrap();
        assert!(
            std::fs::symlink_metadata(ws.root().join("Link.cook"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(ws.root().join("Shared/Tomato Sauce.cook")).unwrap(),
            GOOD
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_path_via_given_non_canonical_root_is_accepted() {
        let (d, _ws) = fixture_workspace();
        let holder = tempfile::tempdir().unwrap();
        let link = holder.path().join("link");
        std::os::unix::fs::symlink(d.path(), &link).unwrap();
        let ws = Workspace::new(&link).unwrap();
        let abs = link.join("Breakfast/Pancakes.cook");
        assert_eq!(
            ws.relative(abs.to_str().unwrap()).unwrap(),
            "Breakfast/Pancakes.cook"
        );
        let err = ws.relative("/etc/passwd").unwrap_err().to_string();
        assert!(err.contains(ws.root().as_str()), "{err}");
    }

    #[test]
    fn double_slashes_are_normalized() {
        let (_d, ws) = fixture_workspace();
        let r = ws.write("a//b.cook", GOOD, false).unwrap();
        assert_eq!(r.path, "a/b.cook");
    }

    #[test]
    fn trailing_slash_backslash_and_nul_are_refused() {
        let (_d, ws) = fixture_workspace();
        for bad in [
            "Dinner/",
            "Dinner\\",
            "a\\b.cook",
            "a/b\\.cook",
            "a\0b.cook",
            "a/\0",
        ] {
            assert!(
                matches!(ws.relative(bad), Err(WorkspaceError::UnsafePath(_))),
                "{bad:?}"
            );
        }
    }
}
