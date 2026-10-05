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
    /// "recipe" (.cook), "menu" (.menu) or "all" (default).
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
    pub query: String,
    /// Only recipes whose frontmatter `tags` contain this tag (case-insensitive).
    pub tag: Option<String>,
}

/// Recipe-root-relative form of a path cookcli-core handed back.
fn rel(ws: &crate::workspace::Workspace, p: &Utf8Path) -> String {
    p.strip_prefix(ws.root()).unwrap_or(p).to_string()
}

fn walk(dir: &Utf8Path, out: &mut Vec<camino::Utf8PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(path) = camino::Utf8PathBuf::from_path_buf(entry.path()) else { continue };
        let name = path.file_name().unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if entry.file_type()?.is_dir() {
            walk(&path, out)?;
        } else if matches!(path.extension(), Some("cook" | "menu")) {
            out.push(path);
        }
    }
    Ok(())
}

fn tags_of(ws: &crate::workspace::Workspace, path: &Utf8Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let name = path.file_stem().unwrap_or("recipe");
    let _ = ws;
    match cookcli_core::parse_recipe(&text, name, 1.0) {
        Ok(o) => o.value.metadata.tags().unwrap_or_default().into_iter().map(|t| t.to_lowercase()).collect(),
        Err(_) => vec![],
    }
}

#[tool_router(router = local_router, vis = "pub(crate)")]
impl CookMcp {
    #[tool(description = "List the .cook recipes and .menu meal plans in the recipe collection. \
        Returns paths relative to the recipe root; pass them unchanged to other tools.")]
    async fn list_recipes(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = &self.workspace;
        let dir = match a.dir.as_deref() {
            None | Some("") | Some(".") => ws.root().to_owned(),
            Some(d) => match ws.resolve(d) {
                Ok(p) => p,
                Err(e) => return Ok(workspace_err(e)),
            },
        };
        let mut files = Vec::new();
        if let Err(e) = walk(&dir, &mut files) {
            return Ok(text_err(format!("cannot list {dir}: {e}")));
        }
        let want = a.kind.as_deref().unwrap_or("all");
        let mut recipes: Vec<serde_json::Value> = files
            .iter()
            .filter(|p| match want {
                "recipe" => p.extension() == Some("cook"),
                "menu" => p.extension() == Some("menu"),
                _ => true,
            })
            .map(|p| serde_json::json!({
                "path": rel(ws, p),
                "kind": if p.extension() == Some("menu") { "menu" } else { "recipe" },
            }))
            .collect();
        recipes.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        Ok(json_ok(&serde_json::json!({ "root": ws.root(), "recipes": recipes })))
    }

    #[tool(description = "Read one recipe or meal plan: its Cooklang source plus the parsed \
        ingredients, cookware, steps and metadata, optionally scaled.")]
    async fn read_recipe(&self, Parameters(a): Parameters<ReadArgs>) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = &self.workspace;
        let rel_path = match ws.relative(&a.path) {
            Ok(p) => p,
            Err(e) => return Ok(workspace_err(e)),
        };
        let req = recipe::ReadRequest {
            source: RecipeSource::Path(rel_path),
            scale: a.scale.unwrap_or(1.0),
        };
        match recipe::read(&ws.context(), req) {
            Err(e) => Ok(core_err(e)),
            Ok(outcome) => {
                let path = outcome.value.path.clone();
                let source = path.as_ref().and_then(|p| std::fs::read_to_string(ws.root().join(p)).ok());
                Ok(json_ok(&serde_json::json!({
                    "path": path.as_deref().map(|p| rel(ws, p)),
                    "title": outcome.value.title,
                    "source": source,
                    "recipe": outcome.value.recipe,
                    "diagnostics": outcome.diagnostics,
                })))
            }
        }
    }

    #[tool(description = "Search the recipe collection by words (matches names and contents) \
        and/or a frontmatter tag. Prefer this over reading files one by one.")]
    async fn search_recipes(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, rmcp::ErrorData> {
        let ws = &self.workspace;
        let tag = a.tag.as_deref().map(str::to_lowercase);
        let candidates: Vec<(String, Option<String>)> = if a.query.trim().is_empty() {
            if tag.is_none() {
                return Ok(text_err("provide `query`, `tag`, or both"));
            }
            let mut files = Vec::new();
            let _ = walk(ws.root(), &mut files);
            files.iter().map(|p| (rel(ws, p), None)).collect()
        } else {
            match search::search(&ws.context(), search::SearchRequest { query: a.query, base_dir: None }) {
                Err(e) => return Ok(core_err(e)),
                Ok(o) => o.value.into_iter().map(|h| (h.relative_path.to_string(), h.name)).collect(),
            }
        };
        let mut hits: Vec<serde_json::Value> = candidates
            .into_iter()
            .filter(|(p, _)| tag.as_ref().is_none_or(|t| tags_of(ws, &ws.root().join(p)).contains(t)))
            .map(|(p, name)| serde_json::json!({ "path": p, "name": name }))
            .collect();
        if a.tag.is_some() {
            hits.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        }
        Ok(json_ok(&serde_json::json!({ "hits": hits })))
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

    fn json(r: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&text_of(r)).unwrap()
    }

    #[tokio::test]
    async fn list_recipes_lists_cook_and_menu_files() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(&s.list_recipes(Parameters(ListArgs { dir: None, kind: None })).await.unwrap());
        let paths: Vec<&str> = v["recipes"].as_array().unwrap().iter().map(|r| r["path"].as_str().unwrap()).collect();
        assert_eq!(
            paths,
            ["Breakfast/Pancakes.cook", "Dinner/Pasta.cook", "Plans/Week.menu", "Shared/Tomato Sauce.cook"]
        );
        let menus = json(&s.list_recipes(Parameters(ListArgs { dir: None, kind: Some("menu".into()) })).await.unwrap());
        assert_eq!(menus["recipes"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn read_recipe_scales_and_returns_source() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(&s.read_recipe(Parameters(ReadArgs { path: "Breakfast/Pancakes.cook".into(), scale: Some(2.0) })).await.unwrap());
        assert_eq!(v["title"], "Pancakes");
        assert_eq!(v["path"], "Breakfast/Pancakes.cook");
        assert!(v["source"].as_str().unwrap().contains("@flour{200%g}"));
        let flour = v["recipe"]["ingredients"].as_array().unwrap().iter().find(|i| i["name"] == "flour").unwrap();
        assert_eq!(flour["quantity"]["value"]["value"]["value"], 400.0, "scaled ×2: {flour}");
    }

    #[tokio::test]
    async fn read_recipe_by_bare_name() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let r = s.read_recipe(Parameters(ReadArgs { path: "Breakfast/Pancakes".into(), scale: None })).await.unwrap();
        assert_eq!(json(&r)["path"], "Breakfast/Pancakes.cook");
    }

    #[tokio::test]
    async fn search_matches_content_and_filters_by_tag() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(&s.search_recipes(Parameters(SearchArgs { query: "garlic".into(), tag: None })).await.unwrap());
        assert_eq!(v["hits"][0]["path"], "Shared/Tomato Sauce.cook");
        let v = json(&s.search_recipes(Parameters(SearchArgs { query: "".into(), tag: Some("breakfast".into()) })).await.unwrap());
        let hits = v["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["path"], "Breakfast/Pancakes.cook");
    }
}
