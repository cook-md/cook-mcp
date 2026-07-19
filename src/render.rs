//! Embedded jinja render core: wires cooklang-reports + NutritionExtension
//! exactly like the `cook-nutrition-demo` example in cooklang-reports-nutrition, but returns the rendered text plus
//! check results and resolve failures instead of printing/exiting.
//!
//! Everything here is blocking (cookmd-nutrition-client is a blocking HTTP client);
//! the MCP tool layer runs `render()` inside `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use cooklang_reports::{Config as ReportConfig, render_template_with_config};
use cooklang_reports_nutrition::NutritionExtension;
use cookmd_nutrition_client::Client;

/// Template or input content: given inline or read from a file.
#[derive(Debug, Clone)]
pub enum Source {
    Inline(String),
    Path(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Cook,
    Menu,
}

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub template: Source,
    pub input: Source,
    /// Required for inline input; inferred from the extension for paths.
    pub kind: Option<InputKind>,
    /// Root for `.menu` recipe refs; defaults to the input file's parent.
    pub base_path: Option<PathBuf>,
    /// Recipe scaling factor (cooklang-reports scales `.cook` quantities and
    /// exposes `scale` to templates). `.menu` plan quantities are not scaled.
    pub scale: Option<f64>,
    pub client_profile_path: Option<PathBuf>,
}

#[derive(Debug, serde::Serialize)]
pub struct RenderOutcome {
    pub rendered: String,
    pub checks: Vec<serde_json::Value>,
    pub resolve_failures: Vec<serde_json::Value>,
}

/// Read the input source and settle its kind. Explicit `kind` wins; for path
/// input the file extension is used; inline input must state its kind.
pub fn resolve_input(
    input: &Source,
    kind: Option<InputKind>,
) -> anyhow::Result<(String, InputKind)> {
    match input {
        Source::Inline(text) => {
            let kind =
                kind.ok_or_else(|| anyhow!("inline input requires `kind` (\"cook\" or \"menu\")"))?;
            Ok((text.clone(), kind))
        }
        Source::Path(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("failed to read input {}", p.display()))?;
            let kind = match kind {
                Some(k) => k,
                None => infer_kind(p)?,
            };
            Ok((text, kind))
        }
    }
}

fn infer_kind(path: &Path) -> anyhow::Result<InputKind> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "cook" => Ok(InputKind::Cook),
        "menu" => Ok(InputKind::Menu),
        _ => bail!(
            "cannot infer input kind from extension of {}; pass `kind`",
            path.display()
        ),
    }
}

/// Deduplicate resolve failures preserving first-seen order. Every jinja
/// aggregate function issues its own HTTP call and records the response's
/// failures, so a template calling e.g. `total_calories` + `macros` over the
/// same list records the same failure repeatedly — collapse those. Key on
/// `(index, ingredient, error.code)` when present, else full-value equality.
fn dedup_failures(raw: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for f in raw {
        let key = match (
            f.get("index"),
            f.get("ingredient"),
            f.pointer("/error/code"),
        ) {
            (Some(i), Some(n), Some(c)) => format!("k:{i}|{n}|{c}"),
            _ => format!("v:{f}"),
        };
        if seen.insert(key) {
            out.push(f);
        }
    }
    out
}

/// Render `req` against the nutrition service at `api_url`. Blocking.
pub fn render(
    req: RenderRequest,
    api_url: &str,
    bearer: Option<String>,
) -> anyhow::Result<RenderOutcome> {
    let template = match &req.template {
        Source::Inline(text) => text.clone(),
        Source::Path(p) => std::fs::read_to_string(p)
            .with_context(|| format!("failed to read template {}", p.display()))?,
    };
    let (input_text, kind) = resolve_input(&req.input, req.kind)?;
    // A stdio MCP server's cwd is meaningless as a recipe root, so inline
    // menus must say where `@./` recipe refs resolve from. Inline .cook input
    // keeps the cwd fallback: recipes never reference sibling files.
    if kind == InputKind::Menu && matches!(req.input, Source::Inline(_)) && req.base_path.is_none()
    {
        bail!("inline menu input requires `base_path` (the recipe-directory root)");
    }

    let mut client = Client::new(api_url);
    if let Some(tok) = bearer {
        client = client.with_auth_token(tok);
    }
    let ext = NutritionExtension::new(Arc::new(client));
    let ext = match &req.client_profile_path {
        Some(p) => ext
            .with_client_path(p)
            .map_err(|e| anyhow!("client profile error: {e}"))?,
        None => ext,
    };
    // Grab handles before the extension moves into the config.
    let client_ctx = ext.client_context();
    let checks = ext.checks();
    let failures = ext.failures();

    let input_path = match &req.input {
        Source::Path(p) => Some(p.as_path()),
        Source::Inline(_) => None,
    };
    let base_path: PathBuf = if let Some(bp) = &req.base_path {
        bp.canonicalize()
            .with_context(|| format!("bad base_path {}", bp.display()))?
    } else if let Some(p) = input_path {
        let canonical = p
            .canonicalize()
            .with_context(|| format!("failed to resolve path {}", p.display()))?;
        canonical
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        std::env::current_dir().context("failed to resolve current dir for base_path")?
    };

    let mut builder = ReportConfig::builder();
    builder.base_path(&base_path);
    if let Some(s) = req.scale {
        builder.scale(s);
    }
    let mut config = builder.build().with_extension(ext);
    if let Some(ctx) = client_ctx {
        config = config.with_context("client", ctx);
    }

    if kind == InputKind::Menu {
        let plan = cooklang_reports_nutrition::plan::build_plan_from_source(
            &input_text,
            &base_path,
            input_path,
        )
        .map_err(|e| anyhow!("plan error: {e}"))?;
        config = config.with_context("plan", serde_json::to_value(&plan)?);
    }

    // `.menu` files aren't valid `.cook` recipes — render against an empty
    // recipe so cooklang-reports' parse step doesn't choke on menu syntax.
    // Templates consume the menu through `plan.*` instead.
    let render_source: &str = if kind == InputKind::Menu {
        ""
    } else {
        &input_text
    };

    // `format_with_source()` carries the minijinja detail, the source line
    // info, and fix hints — the bare Display is just "template error".
    let rendered = render_template_with_config(render_source, &template, &config)
        .map_err(|e| anyhow!("render error: {}", e.format_with_source()))?;

    let checks = checks
        .snapshot()
        .into_iter()
        .map(|c| serde_json::json!({ "label": c.label, "ok": c.ok }))
        .collect();
    let resolve_failures = dedup_failures(failures.snapshot());

    Ok(RenderOutcome {
        rendered,
        checks,
        resolve_failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Full aggregate-response shape cookmd-nutrition-client deserializes (mirrors
    /// the known-good fixture in cooklang-reports-nutrition's jinja_test.rs — all
    /// MacroTotals fields and confidence_breakdown are required).
    fn agg_response(kcal: f64, failures: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "items": [],
            "failures": failures,
            "totals": {
                "mass_g": 150.0,
                "macros": { "kcal": kcal, "protein_g": 20.0, "fat_g": 10.0,
                            "carb_g": 0.0, "fiber_g": 0.0, "sugar_g": 0.0, "sat_fat_g": 0.0 },
                "micros": {}, "vitamins": {},
                "confidence": "confirmed", "is_partial": false,
                "included_count": 1, "failed_count": 0
            },
            "confidence_breakdown": {
                "confirmed_items": 1, "partial_items": 0, "estimated_items": 0,
                "estimated_ingredients": [], "estimated_share_of_micronutrients": null
            }
        })
    }

    async fn render_with_mock(
        req: RenderRequest,
        agg: serde_json::Value,
    ) -> anyhow::Result<RenderOutcome> {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(agg))
            .mount(&mock)
            .await;
        let api_url = mock.uri();
        let out = tokio::task::spawn_blocking(move || render(req, &api_url, None)).await?;
        drop(mock);
        out
    }

    fn inline_req(template: &str, input: &str, kind: InputKind) -> RenderRequest {
        RenderRequest {
            template: Source::Inline(template.to_string()),
            input: Source::Inline(input.to_string()),
            kind: Some(kind),
            base_path: None,
            scale: None,
            client_profile_path: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn renders_recipe_with_inline_template() {
        let req = inline_req(
            "{% set m = macros(ingredients) %}kcal: {{ m.kcal }}",
            "Pan-fry @salmon{150%g} and serve.",
            InputKind::Cook,
        );
        let out = render_with_mock(req, agg_response(312.0, serde_json::json!([])))
            .await
            .unwrap();
        assert!(out.rendered.contains("kcal: 312"), "got: {}", out.rendered);
        assert!(
            out.resolve_failures.is_empty(),
            "expected no failures, got: {:?}",
            out.resolve_failures
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn surfaces_resolve_failures() {
        let failures = serde_json::json!([{
            "index": 0,
            "ingredient": "unobtanium",
            "error": {
                "code": "ingredient_not_found",
                "message": "no match",
                "suggestions": []
            }
        }]);
        let req = inline_req(
            "{% set m = macros(ingredients) %}kcal: {{ m.kcal }}",
            "Add @unobtanium{1%g} and stir.",
            InputKind::Cook,
        );
        let out = render_with_mock(req, agg_response(0.0, failures))
            .await
            .unwrap();
        assert_eq!(
            out.resolve_failures.len(),
            1,
            "got: {:?}",
            out.resolve_failures
        );
        assert_eq!(
            out.resolve_failures[0]["error"]["code"],
            "ingredient_not_found"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn template_syntax_error_is_err_with_line_info() {
        // Template never compiles, so no HTTP call happens — dummy URL is fine.
        let req = inline_req("{% if %}", "@salmon{150%g}", InputKind::Cook);
        let err = tokio::task::spawn_blocking(move || render(req, "http://127.0.0.1:9", None))
            .await
            .unwrap()
            .unwrap_err();
        let msg = format!("{err:#}").to_lowercase();
        assert!(msg.contains("syntax"), "got: {msg}");
    }

    #[test]
    fn path_input_infers_kind_and_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let menu = dir.path().join("dinner.menu");
        std::fs::write(&menu, "Day 1:\n- item\n").unwrap();
        let (text, kind) = resolve_input(&Source::Path(menu), None).unwrap();
        assert_eq!(kind, InputKind::Menu);
        assert!(text.contains("Day 1"));

        let missing = dir.path().join("nope.cook");
        assert!(resolve_input(&Source::Path(missing), None).is_err());

        // Unknown extension without explicit kind cannot be inferred.
        let odd = dir.path().join("notes.txt");
        std::fs::write(&odd, "hello").unwrap();
        let err = resolve_input(&Source::Path(odd), None).unwrap_err();
        assert!(err.to_string().contains("kind"), "got: {err}");

        // Inline without kind is an error too.
        let err = resolve_input(&Source::Inline("@a{1%g}".into()), None).unwrap_err();
        assert!(err.to_string().contains("kind"), "got: {err}");
    }

    #[test]
    fn inline_menu_without_base_path_errors() {
        // A stdio MCP server's cwd is meaningless as a recipe root — inline
        // menus must state where `@./` refs resolve from.
        let req = inline_req("x", "= Snacks =\n\n@apples{5}\n", InputKind::Menu);
        let err = render(req, "http://127.0.0.1:9", None).unwrap_err();
        assert!(err.to_string().contains("base_path"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dedups_repeated_failures() {
        let failures = serde_json::json!([{
            "index": 0,
            "ingredient": "unobtanium",
            "error": {
                "code": "ingredient_not_found",
                "message": "no match",
                "suggestions": []
            }
        }]);
        // Two aggregate-calling functions over the same list: the same failure
        // is recorded once per call and must be collapsed to one.
        let req = inline_req(
            "{{ total_calories(ingredients) }} {% set m = macros(ingredients) %}{{ m.kcal }}",
            "Add @unobtanium{1%g} and stir.",
            InputKind::Cook,
        );
        let out = render_with_mock(req, agg_response(0.0, failures))
            .await
            .unwrap();
        assert_eq!(
            out.resolve_failures.len(),
            1,
            "expected deduped failures, got: {:?}",
            out.resolve_failures
        );
    }
}
