//! Free tools over the recipe root, all backed by cookcli-core. No login.

use camino::Utf8Path;
use cookcli_core::{RecipeSource, recipe, search};
use rmcp::{handler::server::wrapper::Parameters, model::*, schemars, tool, tool_router};

use super::{core_err, json_ok, text_err, workspace_err};
use crate::server::CookMcp;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListArgs {
    /// Folder to list, relative to the recipe root. Default: the whole collection.
    pub dir: Option<String>,
    /// "recipe" (.cook), "menu" (.menu), "all" (default: both), or "template" for the
    /// .jinja report templates under reports/ and config/reports/.
    pub kind: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// Path relative to the recipe root, exactly as list_recipes/search_recipes
    /// returned it. A bare name without extension also works.
    pub path: String,
    /// Scaling factor, e.g. 2 to double. Default 1.
    pub scale: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Words to match against recipe names and contents. May be empty when `tag` is set.
    #[serde(default)]
    pub query: String,
    /// Only recipes whose frontmatter `tags` contain this tag (case-insensitive).
    pub tag: Option<String>,
}

/// Recipe-root-relative form of a path cookcli-core handed back.
fn rel(ws: &crate::workspace::Workspace, p: &Utf8Path) -> String {
    p.strip_prefix(ws.root()).unwrap_or(p).to_string()
}

fn walk(dir: &Utf8Path, out: &mut Vec<camino::Utf8PathBuf>) -> std::io::Result<()> {
    walk_ext(dir, &["cook", "menu"], out)
}

fn walk_ext(
    dir: &Utf8Path,
    exts: &[&str],
    out: &mut Vec<camino::Utf8PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(path) = camino::Utf8PathBuf::from_path_buf(entry.path()) else {
            continue;
        };
        let name = path.file_name().unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_ext(&path, exts, out)?;
        } else if path.extension().is_some_and(|e| exts.contains(&e)) {
            out.push(path);
        }
    }
    Ok(())
}

/// True when `full` is under the recipe root with no symlink on the way
/// (so it cannot loop or leave the collection).
fn is_plain(ws: &crate::workspace::Workspace, full: &Utf8Path) -> bool {
    std::fs::canonicalize(full)
        .map(|c| c == full.as_std_path() && c.starts_with(ws.root().as_std_path()))
        .unwrap_or(false)
}

/// A scale factor must be a positive, finite number.
fn valid_scale(s: f64) -> bool {
    s.is_finite() && s > 0.0
}

fn tags_of(path: &Utf8Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let name = path.file_stem().unwrap_or("recipe");
    match cookcli_core::parse_recipe(&text, name, 1.0) {
        Ok(o) => o
            .value
            .metadata
            .tags()
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.to_lowercase())
            .collect(),
        Err(_) => vec![],
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ValidateArgs {
    /// A .cook/.menu file or a folder, relative to the recipe root. Omit both
    /// `path` and `content` to validate the whole collection.
    pub path: Option<String>,
    /// Unsaved Cooklang text to check instead of a file.
    pub content: Option<String>,
    /// With `content`: where it would be saved (decides .cook vs .menu and how
    /// `@./` references resolve). Default "Untitled.cook".
    pub as_path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteArgs {
    /// Destination relative to the recipe root, e.g. "Dinner/Leek Soup.cook".
    pub path: String,
    /// Full file content. Metadata goes in YAML frontmatter (--- ... ---), never `>>` lines.
    pub content: String,
    /// Save even if validation finds errors. Default false.
    pub force: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteConfigArgs {
    /// "config/aisle.conf", "config/pantry.conf", or a report template ending in
    /// .jinja under reports/ (CookCLI) or config/reports/ (Cook Editor), e.g.
    /// "reports/nutrition.md.jinja".
    pub path: String,
    /// Full file content.
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ShoppingArgs {
    /// Recipes and/or .menu plans, relative to the recipe root. Append `:N` to
    /// scale one, e.g. "Dinner/Pasta.cook:2".
    pub recipes: Vec<String>,
    /// Don't subtract what config/pantry.conf already holds. Default false.
    pub ignore_pantry: Option<bool>,
    /// "json" (default, grouped by aisle) or "markdown".
    pub format: Option<String>,
}

#[tool_router(router = local_router, vis = "pub(crate)")]
impl CookMcp {
    #[tool(
        description = "List the .cook recipes and .menu meal plans in the recipe collection. \
        Set `kind` to \"recipe\", \"menu\", \"template\" (report templates) or \"all\". \
        Returns paths relative to the recipe root; pass them unchanged to other tools."
    )]
    async fn list_recipes(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        let dir = match a.dir.as_deref() {
            None | Some("") | Some(".") => ws.root().to_owned(),
            Some(d) => match ws.resolve(d) {
                Ok(p) => p,
                Err(e) => return Ok(workspace_err(e)),
            },
        };
        let want = a.kind.as_deref().unwrap_or("all");
        if !matches!(want, "all" | "recipe" | "menu" | "template") {
            return Ok(text_err(format!(
                "unknown kind {want:?}: use \"recipe\", \"menu\", \"all\" or \"template\""
            )));
        }
        let mut files = Vec::new();
        let listed = if want == "template" {
            // Templates live under reports/ and config/reports/; a missing
            // folder is just empty. Like walk(), never follow a symlinked
            // folder (it could lead out of the root).
            crate::workspace::TEMPLATE_DIRS
                .iter()
                .map(|d| ws.root().join(d))
                .filter(|d| d.is_dir() && is_plain(&ws, d))
                .try_for_each(|d| walk_ext(&d, &["jinja"], &mut files).map_err(|e| (d.clone(), e)))
        } else {
            walk(&dir, &mut files).map_err(|e| (dir.clone(), e))
        };
        if let Err((folder, e)) = listed {
            return Ok(text_err(format!("cannot list {folder}: {e}")));
        }
        let mut recipes: Vec<serde_json::Value> = files
            .iter()
            .filter(|p| match want {
                "recipe" => p.extension() == Some("cook"),
                "menu" => p.extension() == Some("menu"),
                // `dir` narrows templates too.
                "template" => p.starts_with(&dir),
                _ => true,
            })
            .map(|p| {
                let kind = match p.extension() {
                    Some("menu") => "menu",
                    Some("jinja") => "template",
                    _ => "recipe",
                };
                serde_json::json!({ "path": rel(&ws, p), "kind": kind })
            })
            .collect();
        recipes.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        Ok(json_ok(
            &serde_json::json!({ "root": ws.root(), "recipes": recipes }),
        ))
    }

    #[tool(
        description = "Read one recipe or meal plan: its Cooklang source plus the parsed \
        ingredients, cookware, steps and metadata, optionally scaled."
    )]
    async fn read_recipe(
        &self,
        Parameters(a): Parameters<ReadArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        let scale = a.scale.unwrap_or(1.0);
        if !valid_scale(scale) {
            return Ok(text_err("scale must be a positive number"));
        }
        let rel_path = match ws.relative(&a.path) {
            Ok(p) => p,
            Err(e) => return Ok(workspace_err(e)),
        };
        if !matches!(rel_path.extension(), None | Some("cook" | "menu")) {
            return Ok(text_err("read_recipe reads .cook recipes and .menu plans"));
        }
        if let Err(e) = ws.resolve(&a.path) {
            return Ok(workspace_err(e));
        }
        let req = recipe::ReadRequest {
            source: RecipeSource::Path(rel_path),
            scale,
        };
        match recipe::read(&ws.context(), req) {
            Err(e) => Ok(core_err(e)),
            Ok(outcome) => {
                let path = outcome.value.path.clone();
                let source = path
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(ws.root().join(p)).ok());
                Ok(json_ok(&serde_json::json!({
                    "path": path.as_deref().map(|p| rel(&ws, p)),
                    "title": outcome.value.title,
                    "source": source,
                    "recipe": outcome.value.recipe,
                    "diagnostics": outcome.diagnostics,
                })))
            }
        }
    }

    #[tool(
        description = "Search the recipe collection by words (matches names and contents) \
        and/or a frontmatter tag. Prefer this over reading files one by one."
    )]
    async fn search_recipes(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        let tag = a.tag.as_deref().map(str::to_lowercase);
        let candidates: Vec<(String, Option<String>)> = if a.query.trim().is_empty() {
            if tag.is_none() {
                return Ok(text_err("provide `query`, `tag`, or both"));
            }
            let mut files = Vec::new();
            let _ = walk(ws.root(), &mut files);
            files.iter().map(|p| (rel(&ws, p), None)).collect()
        } else {
            match search::search(
                &ws.context(),
                search::SearchRequest {
                    query: a.query,
                    base_dir: None,
                },
            ) {
                Err(e) => return Ok(core_err(e)),
                Ok(o) => o
                    .value
                    .into_iter()
                    .map(|h| (h.relative_path.to_string(), h.name))
                    .collect(),
            }
        };
        let mut hits: Vec<serde_json::Value> = candidates
            .into_iter()
            .filter(|(p, _)| is_plain(&ws, &ws.root().join(p)))
            .filter(|(p, _)| {
                tag.as_ref()
                    .is_none_or(|t| tags_of(&ws.root().join(p)).contains(t))
            })
            .map(|(p, name)| serde_json::json!({ "path": p, "name": name }))
            .collect();
        if a.tag.is_some() {
            hits.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        }
        Ok(json_ok(&serde_json::json!({ "hits": hits })))
    }

    #[tool(
        description = "Check Cooklang for errors: a file, a folder, unsaved `content`, or \
        (no arguments) the whole collection. Reports parse errors/warnings and recipe references \
        that don't resolve; when validating a folder or the whole collection, also ingredients \
        missing from config/aisle.conf. Run before and after editing."
    )]
    async fn validate(
        &self,
        Parameters(a): Parameters<ValidateArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        use cookcli_core::doctor;
        if let Some(content) = a.content {
            let as_path = a.as_path.unwrap_or_else(|| "Untitled.cook".into());
            let rel_path = match ws.relative(&as_path) {
                Ok(p) => p,
                Err(e) => return Ok(workspace_err(e)),
            };
            let mut diagnostics = ws.check(&content, &rel_path);
            if crate::workspace::has_legacy_metadata(&content) {
                diagnostics.push(cookcli_core::Diagnostic::error(
                    "deprecated `>>` metadata: move it into YAML frontmatter between `---` lines",
                ));
            }
            let ok = !diagnostics
                .iter()
                .any(|d| d.severity == cookcli_core::Severity::Error);
            return Ok(json_ok(
                &serde_json::json!({ "ok": ok, "diagnostics": diagnostics }),
            ));
        }
        // A single file: same checks as a write, on the saved text.
        if let Some(p) = a
            .path
            .as_deref()
            .filter(|p| p.ends_with(".cook") || p.ends_with(".menu"))
        {
            let full = match ws.resolve(p) {
                Ok(f) => f,
                Err(e) => return Ok(workspace_err(e)),
            };
            let Ok(text) = std::fs::read_to_string(&full) else {
                return Ok(text_err(format!("no such file: {p}")));
            };
            let rel_path = ws.relative(p).expect("resolved above");
            let diagnostics = ws.check(&text, &rel_path);
            let ok = !diagnostics
                .iter()
                .any(|d| d.severity == cookcli_core::Severity::Error);
            return Ok(json_ok(
                &serde_json::json!({ "ok": ok, "path": rel_path, "diagnostics": diagnostics }),
            ));
        }
        let base_dir = match a.path.as_deref() {
            None | Some("") | Some(".") => None,
            Some(d) => match ws.resolve(d) {
                Ok(p) if p.is_file() => {
                    return Ok(text_err("not a .cook/.menu file or folder"));
                }
                Ok(p) => Some(p),
                Err(e) => return Ok(workspace_err(e)),
            },
        };
        let ctx = ws.context();
        let report = match doctor::validate(
            &ctx,
            doctor::ValidateRequest {
                base_dir: base_dir.clone(),
                ..Default::default()
            },
        ) {
            Ok(o) => o.value,
            Err(e) => return Ok(core_err(e)),
        };
        // Drop anything reached through a symlink (loops, outside targets).
        let scanned_from = if report.base_dir.is_absolute() {
            report.base_dir.clone()
        } else {
            ws.root().join(&report.base_dir)
        };
        let plain: Vec<&_> = report
            .recipes
            .iter()
            .filter(|r| is_plain(&ws, &scanned_from.join(&r.path)))
            .collect();
        let total = plain.len();
        let with_errors = plain
            .iter()
            .filter(|r| {
                r.diagnostics
                    .iter()
                    .any(|d| d.severity == cookcli_core::Severity::Error)
            })
            .count();
        let broken = doctor::broken_references(&report);
        let problems: Vec<serde_json::Value> = plain
            .iter()
            .filter(|r| !r.diagnostics.is_empty() || broken.contains_key(r.path.as_path()))
            .map(|r| {
                serde_json::json!({
                    "path": r.path,
                    "diagnostics": r.diagnostics,
                    "broken_references": broken.get(r.path.as_path()).cloned().unwrap_or_default(),
                })
            })
            .collect();
        let aisle = doctor::aisle_coverage(&ctx, doctor::CoverageRequest { base_dir })
            .ok()
            .map(|o| {
                let unknown: Vec<&str> = o
                    .value
                    .ingredients
                    .iter()
                    .filter(|i| !i.known)
                    .map(|i| i.name.as_str())
                    .collect();
                serde_json::json!({ "configured": !ctx.aisle().is_unset(), "unknown": unknown })
            });
        Ok(json_ok(&serde_json::json!({
            "total_recipes": total,
            "recipes_with_errors": with_errors,
            "recipes_with_broken_references": plain.iter().filter(|r| broken.contains_key(r.path.as_path())).count(),
            "problems": problems,
            "aisle": aisle,
        })))
    }

    #[tool(
        description = "Save a .cook recipe in the recipe collection. The content is validated \
        first and NOT saved if it has errors (pass force: true to override). Metadata must be YAML \
        frontmatter; `>>` lines are always refused. This writes the file for real; tell the user \
        what you saved and where."
    )]
    async fn write_recipe(
        &self,
        Parameters(a): Parameters<WriteArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        if !a.path.ends_with(".cook") {
            return Ok(text_err(
                "write_recipe writes .cook files; use write_menu for .menu plans",
            ));
        }
        Ok(
            match ws.write(&a.path, &a.content, a.force.unwrap_or(false)) {
                Ok(report) => json_ok(&report),
                Err(e) => workspace_err(e),
            },
        )
    }

    #[tool(
        description = "Save a .menu meal plan. Reference only recipes that exist, as \
        `@./path/to/Recipe{N%servings}` (path from list_recipes, no extension); every reference \
        is checked and the plan is NOT saved if one doesn't resolve (force: true overrides)."
    )]
    async fn write_menu(
        &self,
        Parameters(a): Parameters<WriteArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        if !a.path.ends_with(".menu") {
            return Ok(text_err(
                "write_menu writes .menu files; use write_recipe for .cook recipes",
            ));
        }
        Ok(
            match ws.write(&a.path, &a.content, a.force.unwrap_or(false)) {
                Ok(report) => json_ok(&report),
                Err(e) => workspace_err(e),
            },
        )
    }

    #[tool(
        description = "Save config/aisle.conf (store aisles for shopping lists), \
        config/pantry.conf (pantry stock; creates it if missing) or a jinja report template \
        under reports/ or config/reports/ (path ending .jinja, rendered later with render_report template_path). \
        Always the full file. Config problems come back as warnings in `diagnostics`; nothing is \
        refused for them. Not for Cooklang: use write_recipe / write_menu."
    )]
    async fn write_config(
        &self,
        Parameters(a): Parameters<WriteConfigArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        Ok(match ws.write_config(&a.path, &a.content) {
            Ok(report) => json_ok(&report),
            Err(e) => workspace_err(e),
        })
    }

    #[tool(
        description = "Build a shopping list from recipes and/or .menu plans: merges duplicate \
        ingredients, follows recipe references, groups by config/aisle.conf and subtracts \
        config/pantry.conf."
    )]
    async fn shopping_list(
        &self,
        Parameters(a): Parameters<ShoppingArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = match self.local_workspace().await {
            Ok(ws) => ws,
            Err(e) => return Ok(e),
        };
        use cookcli_core::shopping_list::{self, GenerateRequest, ScaledRecipe};
        if a.recipes.is_empty() {
            return Ok(text_err(
                "`recipes` is empty: list at least one recipe or .menu",
            ));
        }
        let mut recipes = Vec::new();
        for entry in &a.recipes {
            let (name, scale) =
                recipe::split_name_and_scale(entry).unwrap_or((entry.as_str(), 1.0));
            if !valid_scale(scale) {
                return Ok(text_err(format!(
                    "scale in {entry:?} must be a positive number"
                )));
            }
            if let Err(e) = ws.resolve(name) {
                return Ok(workspace_err(e));
            }
            match ws.relative(name) {
                Ok(p) => recipes.push(ScaledRecipe::scaled(RecipeSource::Path(p), scale)),
                Err(e) => return Ok(workspace_err(e)),
            }
        }
        let mut ctx = ws.context();
        if a.ignore_pantry.unwrap_or(false) {
            ctx = ctx.with_pantry(cookcli_core::ConfigSource::None);
        }
        let outcome = match shopping_list::generate(
            &ctx,
            GenerateRequest {
                recipes,
                ..Default::default()
            },
        ) {
            Ok(o) => o,
            Err(e) => return Ok(core_err(e)),
        };
        let diagnostics = outcome.diagnostics;
        let list = outcome.value;
        Ok(match a.format.as_deref() {
            Some("markdown") => json_ok(&serde_json::json!({
                "markdown": cookcli_core::format::shopping_list::build_md_value(list, false, false),
                "diagnostics": diagnostics,
            })),
            _ => json_ok(&serde_json::json!({
                "list": cookcli_core::format::shopping_list::build_json_value(list, false),
                "diagnostics": diagnostics,
            })),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::fixture_workspace;
    use crate::tools::text_of;

    pub(crate) fn server(ws: crate::workspace::Workspace) -> CookMcp {
        let dir = tempfile::tempdir().unwrap();
        let auth = dir.keep().join("auth.json");
        let cfg = crate::config::Config::from_vars(|k| {
            (k == "COOK_MCP_AUTH_PATH").then(|| auth.to_string_lossy().into_owned())
        });
        CookMcp::new(cfg, ws)
    }

    #[tokio::test]
    async fn unset_workspace_returns_hint_from_local_tools() {
        let (_d, mut ws) = fixture_workspace();
        ws.set_unset_for_test();
        let s = server(ws);
        let r = s
            .list_recipes(Parameters(ListArgs {
                dir: None,
                kind: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("COOK_RECIPES_DIR"));
    }

    fn json(r: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&text_of(r)).unwrap()
    }

    #[tokio::test]
    async fn list_recipes_lists_cook_and_menu_files() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: None,
            }))
            .await
            .unwrap(),
        );
        let paths: Vec<&str> = v["recipes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            [
                "Breakfast/Pancakes.cook",
                "Dinner/Pasta.cook",
                "Plans/Week.menu",
                "Shared/Tomato Sauce.cook"
            ]
        );
        let menus = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: Some("menu".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(menus["recipes"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_recipes_kind_template_lists_report_templates_only() {
        let (_d, ws) = fixture_workspace();
        std::fs::create_dir_all(ws.root().join("reports/nutrition")).unwrap();
        std::fs::write(ws.root().join("reports/nutrition/week.md.jinja"), "x").unwrap();
        std::fs::write(ws.root().join("reports/notes.txt"), "x").unwrap();
        std::fs::write(ws.root().join("Dinner/stray.jinja"), "x").unwrap();
        std::fs::create_dir_all(ws.root().join("config/reports")).unwrap();
        std::fs::write(ws.root().join("config/reports/cost.md.jinja"), "x").unwrap();
        std::fs::write(ws.root().join("config/stray.jinja"), "x").unwrap();
        let s = server(ws);
        let v = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: Some("template".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(
            v["recipes"],
            serde_json::json!([
                { "path": "config/reports/cost.md.jinja", "kind": "template" },
                { "path": "reports/nutrition/week.md.jinja", "kind": "template" }
            ])
        );
        // The default listing stays Cooklang-only.
        let all = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(all["recipes"].as_array().unwrap().len(), 4);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_recipes_kind_template_skips_symlinked_template_dirs() {
        let (_d, ws) = fixture_workspace();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("leak.jinja"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), ws.root().join("reports")).unwrap();
        std::fs::create_dir_all(outside.path().join("reports")).unwrap();
        std::fs::write(outside.path().join("reports/leak2.jinja"), "x").unwrap();
        std::fs::remove_dir_all(ws.root().join("config")).unwrap();
        std::os::unix::fs::symlink(outside.path(), ws.root().join("config")).unwrap();
        let s = server(ws);
        let v = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: Some("template".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["recipes"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn list_recipes_kind_template_without_reports_folder_is_empty() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.list_recipes(Parameters(ListArgs {
                dir: None,
                kind: Some("template".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["recipes"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn write_config_writes_each_kind_and_refuses_others() {
        let (_d, ws) = fixture_workspace();
        let root = ws.root().to_owned();
        let s = server(ws);
        let write = |path: &str, content: &str| {
            s.write_config(Parameters(WriteConfigArgs {
                path: path.into(),
                content: content.into(),
            }))
        };
        for (path, content) in [
            ("config/aisle.conf", "[produce]\nleek\n"),
            ("config/pantry.conf", "[fridge]\nmilk = \"1%l\"\n"),
            ("reports/cost.md.jinja", "{{ metadata.title }}"),
            ("config/reports/cost.md.jinja", "{{ metadata.title }}"),
        ] {
            let r = write(path, content).await.unwrap();
            assert_ne!(r.is_error, Some(true), "{path}: {}", text_of(&r));
            assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), content);
        }
        for bad in ["reports/x.txt", "other/x.jinja", "Dinner/Pasta.cook"] {
            let r = write(bad, "x").await.unwrap();
            assert_eq!(r.is_error, Some(true), "{bad}");
            assert!(
                text_of(&r).contains("write_config writes"),
                "{}",
                text_of(&r)
            );
        }
        assert!(!root.join("other/x.jinja").exists());
    }

    #[tokio::test]
    async fn read_recipe_scales_and_returns_source() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.read_recipe(Parameters(ReadArgs {
                path: "Breakfast/Pancakes.cook".into(),
                scale: Some(2.0),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["title"], "Pancakes");
        assert_eq!(v["path"], "Breakfast/Pancakes.cook");
        assert!(v["source"].as_str().unwrap().contains("@flour{200%g}"));
        let flour = v["recipe"]["ingredients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == "flour")
            .unwrap();
        assert_eq!(
            flour["quantity"]["value"]["value"]["value"], 400.0,
            "scaled ×2: {flour}"
        );
    }

    #[tokio::test]
    async fn read_recipe_by_bare_name() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let r = s
            .read_recipe(Parameters(ReadArgs {
                path: "Breakfast/Pancakes".into(),
                scale: None,
            }))
            .await
            .unwrap();
        assert_eq!(json(&r)["path"], "Breakfast/Pancakes.cook");
    }

    #[tokio::test]
    async fn search_matches_content_and_filters_by_tag() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.search_recipes(Parameters(SearchArgs {
                query: "garlic".into(),
                tag: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["hits"][0]["path"], "Shared/Tomato Sauce.cook");
        let v = json(
            &s.search_recipes(Parameters(SearchArgs {
                query: "".into(),
                tag: Some("breakfast".into()),
            }))
            .await
            .unwrap(),
        );
        let hits = v["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["path"], "Breakfast/Pancakes.cook");
    }

    #[tokio::test]
    async fn validate_inline_content_reports_errors_and_broken_refs() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.validate(Parameters(ValidateArgs {
                path: None,
                content: Some("Toss with @./Shared/Nope{1%servings}.\n".into()),
                as_path: Some("Dinner/New.cook".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["ok"], false);
        assert!(v["diagnostics"].to_string().contains("./Shared/Nope"));
    }

    #[tokio::test]
    async fn validate_collection_summarises() {
        let (d, ws) = fixture_workspace();
        std::fs::write(d.path().join("Broken.cook"), "Add @{1%tsp}.\n").unwrap();
        let s = server(ws);
        let v = json(
            &s.validate(Parameters(ValidateArgs {
                path: None,
                content: None,
                as_path: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["total_recipes"], 5);
        assert_eq!(v["recipes_with_errors"], 1);
        assert_eq!(v["problems"][0]["path"], "Broken.cook");
        assert!(
            v["aisle"]["unknown"].as_array().unwrap().is_empty(),
            "fixture aisle covers everything: {v}"
        );
    }

    #[tokio::test]
    async fn write_recipe_refuses_invalid_and_reports_success() {
        let (d, ws) = fixture_workspace();
        let s = server(ws);
        let r = s
            .write_recipe(Parameters(WriteArgs {
                path: "X.cook".into(),
                content: "Add @{1%tsp}.\n".into(),
                force: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("invalid_cooklang"));
        assert!(!d.path().join("X.cook").exists());
        let r = s
            .write_recipe(Parameters(WriteArgs {
                path: "X.cook".into(),
                content: "Fry @eggs{2}.\n".into(),
                force: None,
            }))
            .await
            .unwrap();
        assert_eq!(json(&r)["status"], "created");
    }

    #[tokio::test]
    async fn write_menu_requires_menu_extension() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let r = s
            .write_menu(Parameters(WriteArgs {
                path: "Plans/Next.cook".into(),
                content: "x".into(),
                force: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
    }

    #[tokio::test]
    async fn shopping_list_from_menu_subtracts_pantry() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.shopping_list(Parameters(ShoppingArgs {
                recipes: vec!["Plans/Week.menu".into()],
                ignore_pantry: None,
                format: None,
            }))
            .await
            .unwrap(),
        );
        let text = v.to_string();
        assert!(
            text.contains("tomatoes"),
            "menu → pasta → sauce reference resolved: {text}"
        );
        assert!(
            !text.contains("\"flour\""),
            "flour is in the pantry: {text}"
        );
    }

    #[cfg(unix)]
    mod symlinks {
        use super::*;
        use std::os::unix::fs::symlink;

        fn outside() -> tempfile::TempDir {
            let o = tempfile::tempdir().unwrap();
            std::fs::write(o.path().join("Secret.cook"), "Eat @secret{1}.\n").unwrap();
            o
        }

        #[tokio::test]
        async fn read_recipe_refuses_symlinks_out_of_root() {
            let (d, ws) = fixture_workspace();
            let o = outside();
            symlink(o.path().join("Secret.cook"), d.path().join("Link.cook")).unwrap();
            symlink(o.path(), d.path().join("Escape")).unwrap();
            let s = server(ws);
            for p in ["Link.cook", "Escape/Secret.cook"] {
                let r = s
                    .read_recipe(Parameters(ReadArgs {
                        path: p.into(),
                        scale: None,
                    }))
                    .await
                    .unwrap();
                assert_eq!(r.is_error, Some(true), "{p}: {}", text_of(&r));
                assert!(!text_of(&r).contains("secret"), "{p}");
            }
        }

        #[tokio::test]
        async fn shopping_list_refuses_symlinks_out_of_root() {
            let (d, ws) = fixture_workspace();
            let o = outside();
            symlink(o.path(), d.path().join("Escape")).unwrap();
            let s = server(ws);
            let r = s
                .shopping_list(Parameters(ShoppingArgs {
                    recipes: vec!["Escape/Secret.cook".into()],
                    ignore_pantry: None,
                    format: None,
                }))
                .await
                .unwrap();
            assert_eq!(r.is_error, Some(true), "{}", text_of(&r));
        }

        #[tokio::test]
        async fn list_skips_symlinks() {
            let (d, ws) = fixture_workspace();
            let o = outside();
            symlink(o.path().join("Secret.cook"), d.path().join("Link.cook")).unwrap();
            symlink(o.path(), d.path().join("Escape")).unwrap();
            symlink(d.path(), d.path().join("Dinner/Loop")).unwrap();
            let s = server(ws);
            let v = json(
                &s.list_recipes(Parameters(ListArgs {
                    dir: None,
                    kind: None,
                }))
                .await
                .unwrap(),
            );
            let n = v["recipes"].as_array().unwrap().len();
            assert_eq!(n, 4, "{v}");
        }

        #[tokio::test]
        async fn search_ignores_loops_and_escapes() {
            let (d, ws) = fixture_workspace();
            let o = outside();
            symlink(o.path(), d.path().join("Escape")).unwrap();
            symlink(d.path(), d.path().join("Dinner/Loop")).unwrap();
            let s = server(ws);
            for q in ["secret", "garlic", "pasta"] {
                let v = json(
                    &s.search_recipes(Parameters(SearchArgs {
                        query: q.into(),
                        tag: None,
                    }))
                    .await
                    .unwrap(),
                );
                for h in v["hits"].as_array().unwrap() {
                    let p = h["path"].as_str().unwrap();
                    assert!(!p.contains("Loop/") && !p.starts_with("Escape"), "{q}: {v}");
                }
            }
        }

        #[tokio::test]
        async fn validate_ignores_loops_and_escapes() {
            let (d, ws) = fixture_workspace();
            let o = outside();
            symlink(o.path(), d.path().join("Escape")).unwrap();
            symlink(d.path(), d.path().join("Dinner/Loop")).unwrap();
            let s = server(ws);
            let v = json(
                &s.validate(Parameters(ValidateArgs {
                    path: None,
                    content: None,
                    as_path: None,
                }))
                .await
                .unwrap(),
            );
            assert_eq!(v["total_recipes"], 4, "{v}");
            assert!(!v.to_string().contains("Loop/"), "{v}");
            assert!(!v.to_string().contains("Escape"), "{v}");
        }
    }

    #[tokio::test]
    async fn input_checks() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        for scale in [0.0, -1.0, f64::NAN] {
            let r = s
                .read_recipe(Parameters(ReadArgs {
                    path: "Breakfast/Pancakes.cook".into(),
                    scale: Some(scale),
                }))
                .await
                .unwrap();
            assert_eq!(r.is_error, Some(true));
        }
        let r = s
            .read_recipe(Parameters(ReadArgs {
                path: "config/aisle.conf".into(),
                scale: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let r = s
            .validate(Parameters(ValidateArgs {
                path: Some("config/aisle.conf".into()),
                content: None,
                as_path: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let r = s
            .shopping_list(Parameters(ShoppingArgs {
                recipes: vec![],
                ignore_pantry: None,
                format: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let r = s
            .shopping_list(Parameters(ShoppingArgs {
                recipes: vec!["Breakfast/Pancakes.cook:0".into()],
                ignore_pantry: None,
                format: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let r = s
            .list_recipes(Parameters(ListArgs {
                dir: None,
                kind: Some("bogus".into()),
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
    }
}
