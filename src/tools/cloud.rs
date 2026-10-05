use rmcp::{handler::server::wrapper::Parameters, model::*, schemars, tool, tool_router};

use super::{json_err, json_ok, text_err};
use crate::api::ApiOutcome;
use crate::server::CookMcp;

fn outcome_to_result(out: anyhow::Result<ApiOutcome>) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(match out {
        Ok(ApiOutcome::Ok(v)) => json_ok(&v),
        Ok(ApiOutcome::Err { body, .. }) => json_err(&body),
        Err(e) => text_err(e.to_string()),
    })
}

// f64 Display already renders whole numbers without the trailing ".0".
fn fmt_f64(v: Option<f64>) -> Option<String> {
    v.map(|n| n.to_string())
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetNutritionArgs {
    /// Ingredient name, e.g. "salmon". Plain name only — put qualifiers like
    /// "fresh" in `prep`, not here.
    pub ingredient: String,
    /// Quantity. Required. Omit `unit` (not `amount`) to count default
    /// portions — e.g. amount=1 with no unit = one default portion.
    pub amount: f64,
    /// Unit slug, e.g. "g", "cup", "slice". Case- and plural-sensitive.
    pub unit: Option<String>,
    /// Preparation, e.g. "raw" (default), "cooked".
    pub prep: Option<String>,
    /// Region for cup/tbsp/tsp sizing: "us" (default), "uk", or "metric".
    pub region: Option<String>,
    /// Reference-intake standard for %DV: "fda", "eu", or "uk".
    pub reference: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AggregateItemArg {
    pub ingredient: String,
    /// Quantity. Required. As in get_nutrition, omit `unit` (not `amount`)
    /// to count default portions — e.g. amount=1 with no unit.
    pub amount: f64,
    pub unit: Option<String>,
    pub prep: Option<String>,
    /// Region for cup/tbsp/tsp sizing: "us" (default), "uk", or "metric".
    pub region: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AggregateArgs {
    /// Ingredient lines to sum. Per-item failures come back in `failures[]`
    /// with the original index — they are not fatal.
    pub items: Vec<AggregateItemArg>,
    /// Default region for cup/tbsp/tsp sizing, applied to items that don't
    /// set their own: "us" (default), "uk", or "metric".
    pub region: Option<String>,
    pub reference: Option<String>,
    /// Ingredient names/categories that must be absent; matches come back in
    /// the response's exclusion fields.
    pub exclusions: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LookupArgs {
    /// Free-text ingredient name to fuzzy-match against the catalog.
    pub q: String,
    /// ISO 639-1 language code, default "en".
    pub lang: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConvertArgs {
    pub amount: f64,
    pub from: String,
    pub to: String,
    /// Needed for volume<->mass conversions (density is per-ingredient).
    pub ingredient: Option<String>,
    pub prep: Option<String>,
    /// Region for cup/tbsp/tsp sizing: "us" (default), "uk", or "metric".
    pub region: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CategoryArgs {
    /// Category slug, e.g. "vegetables".
    pub slug: String,
    pub ingredient: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BrandedArgs {
    /// GTIN/UPC barcode for an exact match. Provide exactly one of upc | q.
    pub upc: Option<String>,
    /// Brand or product text search. Provide exactly one of upc | q.
    pub q: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReferenceIntakesArgs {
    /// "fda" (default), "eu", or "uk".
    pub standard: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenderReportArgs {
    /// Inline jinja template source. Provide exactly one of template | template_path.
    pub template: Option<String>,
    /// Absolute path to a .jinja template file.
    pub template_path: Option<String>,
    /// Inline cooklang source (requires `kind`). Provide exactly one of input | input_path.
    pub input: Option<String>,
    /// Absolute path to a .cook recipe or .menu plan (kind inferred from extension).
    pub input_path: Option<String>,
    /// "cook" or "menu" — required with inline `input`.
    pub kind: Option<String>,
    /// Absolute path to the recipe-directory root, used to resolve `@./` recipe
    /// references in .menu plans; defaults to the input file's parent directory.
    pub base_path: Option<String>,
    /// Recipe scaling factor. NOTE: scales .cook recipes only — .menu plan
    /// quantities are not scaled.
    pub scale: Option<f64>,
    /// Absolute path to a client profile YAML (dietary targets for checks).
    pub client_profile_path: Option<String>,
}

#[tool_router(router = cloud_router, vis = "pub(crate)")]
impl CookMcp {
    #[tool(description = "Nutrition facts for one ingredient amount (macros, \
        micros, allergens, %DV). Errors include `suggestions` with close \
        catalog names — retry with one of those on ingredient_not_found.")]
    async fn get_nutrition(
        &self,
        Parameters(a): Parameters<GetNutritionArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        outcome_to_result(
            self.api
                .get(
                    "/nutrition",
                    &[
                        ("ingredient", Some(a.ingredient)),
                        ("amount", fmt_f64(Some(a.amount))),
                        ("unit", a.unit),
                        ("prep", a.prep),
                        ("region", a.region),
                        ("reference", a.reference),
                    ],
                )
                .await,
        )
    }

    #[tool(description = "Sum nutrition across many ingredient lines (a whole \
        recipe). Per-item failures come back in `failures[]` with the item \
        index and `suggestions` — fix those items and retry.")]
    async fn aggregate_nutrition(
        &self,
        Parameters(a): Parameters<AggregateArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The service accepts region per-item only (no top-level `region` on
        // AggregateRequest) — apply the tool-level region as a per-item default.
        let mut body = serde_json::json!({ "items": a.items.iter().map(|i| {
            serde_json::json!({
                "ingredient": i.ingredient, "amount": i.amount,
                "unit": i.unit, "prep": i.prep,
                "region": i.region.clone().or_else(|| a.region.clone()),
            })
        }).collect::<Vec<_>>() });
        if let Some(r) = a.reference {
            body["reference"] = r.into();
        }
        if let Some(ex) = a.exclusions {
            body["exclusions"] = ex.into();
        }
        outcome_to_result(self.api.post("/aggregate", body).await)
    }

    #[tool(description = "Fuzzy-search the ingredient catalog. Use to debug why \
        get_nutrition or render_report couldn't resolve an ingredient name.")]
    async fn lookup_ingredient(
        &self,
        Parameters(a): Parameters<LookupArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        outcome_to_result(
            self.api
                .get("/ingredients/lookup", &[("q", Some(a.q)), ("lang", a.lang)])
                .await,
        )
    }

    #[tool(description = "Convert an amount between units. Volume<->mass needs \
        `ingredient` (density lookup). Use to debug density_unavailable failures.")]
    async fn convert_units(
        &self,
        Parameters(a): Parameters<ConvertArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        outcome_to_result(
            self.api
                .get(
                    "/convert",
                    &[
                        ("amount", fmt_f64(Some(a.amount))),
                        ("from", Some(a.from)),
                        ("to", Some(a.to)),
                        ("ingredient", a.ingredient),
                        ("prep", a.prep),
                        ("region", a.region),
                    ],
                )
                .await,
        )
    }

    #[tool(description = "Check whether an ingredient belongs to a category slug.")]
    async fn check_category(
        &self,
        Parameters(a): Parameters<CategoryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The slug is interpolated into the URL path — restrict it to slug
        // characters so a stray value can't rewrite the path or query.
        if a.slug.is_empty()
            || !a
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "invalid category slug: expected lowercase letters, digits, `_` or `-`",
            )]));
        }
        outcome_to_result(
            self.api
                .get(
                    &format!("/categories/{}/check", a.slug),
                    &[("ingredient", Some(a.ingredient))],
                )
                .await,
        )
    }

    #[tool(description = "Look up a branded/packaged product by UPC barcode or \
        text search. Provide exactly one of `upc` or `q`.")]
    async fn branded_lookup(
        &self,
        Parameters(a): Parameters<BrandedArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if a.upc.is_some() == a.q.is_some() {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "provide exactly one of `upc` or `q`",
            )]));
        }
        outcome_to_result(
            self.api
                .get("/branded/lookup", &[("upc", a.upc), ("q", a.q)])
                .await,
        )
    }

    #[tool(
        description = "Daily reference-intake tables (RDA/DV) for a standard: \
        fda, eu, or uk."
    )]
    async fn reference_intakes(
        &self,
        Parameters(a): Parameters<ReferenceIntakesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        outcome_to_result(
            self.api
                .get("/reference-intakes", &[("standard", a.standard)])
                .await,
        )
    }

    #[tool(description = "Render a jinja nutrition report template against a \
        .cook recipe or .menu plan. Returns {rendered, checks, \
        resolve_failures}. Iterate: fix the template or recipe using \
        resolve_failures[].error (code/message/suggestions), re-render until \
        clean. Paths must be absolute. Scaling applies to .cook recipes only, \
        not .menu quantities. Companion tools: lookup_ingredient for name \
        misses, convert_units for unit/density issues.")]
    async fn render_report(
        &self,
        Parameters(a): Parameters<RenderReportArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        use crate::render::{InputKind, RenderRequest, Source};
        let template = match (a.template, a.template_path) {
            (Some(t), None) => Source::Inline(t),
            (None, Some(p)) => Source::Path(p.into()),
            _ => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "provide exactly one of `template` or `template_path`",
                )]));
            }
        };
        let input = match (a.input, a.input_path) {
            (Some(t), None) => Source::Inline(t),
            (None, Some(p)) => Source::Path(p.into()),
            _ => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "provide exactly one of `input` or `input_path`",
                )]));
            }
        };
        let kind = match a.kind.as_deref() {
            Some("cook") => Some(InputKind::Cook),
            Some("menu") => Some(InputKind::Menu),
            Some(other) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "unknown kind `{other}`; use \"cook\" or \"menu\""
                ))]));
            }
            None => None,
        };
        let req = RenderRequest {
            template,
            input,
            kind,
            base_path: a.base_path.map(Into::into),
            scale: a.scale,
            client_profile_path: a.client_profile_path.map(Into::into),
        };
        let api_url = self.cfg.api_url.clone();
        let bearer = self.auth.bearer().await;
        let out = tokio::task::spawn_blocking(move || crate::render::render(req, &api_url, bearer))
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("render task failed: {e}"), None)
            })?;
        match out {
            Ok(o) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&o).unwrap_or_default(),
            )])),
            // `{e:#}` keeps the anyhow context chain (e.g. the io error under
            // "failed to read template ...") in the tool error.
            Err(e) => {
                let mut msg = format!("{e:#}");
                // No downcast is possible here: cooklang-reports-nutrition stringifies
                // ClientError at the minijinja boundary (`e.to_string()`), so
                // match the exact Display prefix of
                // `cookmd_nutrition_client::ClientError::Unauthorized` instead.
                if msg.contains("authentication required") {
                    msg.push_str(
                        "\nHint: not authenticated — run the `login` tool \
                         (or set NUTRITION_API_TOKEN) then retry.",
                    );
                }
                Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
            }
        }
    }

    #[tool(description = "Log in to cook.md (required for nutrition tools \
        unless NUTRITION_API_TOKEN is set). Returns a user_code and \
        verification_uri — show BOTH to the user and tell them to open the URL \
        and enter the code. Approval is detected automatically in the \
        background; verify with auth_status.")]
    async fn login(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        if self.cfg.token_override.is_some() {
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                "already authenticated via NUTRITION_API_TOKEN",
            )]));
        }
        if let Some(stored) = self.auth.current()
            && stored.expires_at > now_unix()
        {
            return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "already logged in as {}",
                stored.email.as_deref().unwrap_or("<unknown>")
            ))]));
        }
        let flow = crate::auth::device::DeviceFlow::new(self.cfg.cookmd_url.clone());
        let start = match flow.start().await {
            Ok(s) => s,
            Err(e) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "could not start login: {e} — is {} reachable?",
                    self.cfg.cookmd_url
                ))]));
            }
        };
        let auth = self.auth.clone();
        let device_code = start.device_code.clone();
        let interval = std::time::Duration::from_secs(start.interval.max(1));
        let expires_in = std::time::Duration::from_secs(start.expires_in);
        tokio::spawn(async move {
            match flow.poll(&device_code, interval, expires_in).await {
                Ok(token) => {
                    if let Err(e) = auth.store_token(&token) {
                        tracing::error!("failed to store login token: {e:#}");
                    }
                }
                Err(e) => tracing::warn!("device login polling failed: {e:#}"),
            }
        });
        let body = serde_json::json!({
            "user_code": start.user_code,
            "verification_uri": start.verification_uri,
            "verification_uri_complete": start.verification_uri_complete,
            "expires_in_seconds": start.expires_in,
            "instructions": "Show the user_code and verification_uri to the \
                user. Once they approve in the browser, tool calls will \
                authenticate automatically — verify with auth_status.",
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Report login + Cook Pro subscription status.")]
    async fn auth_status(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut status = serde_json::json!({});
        let mut probe_subscription = false;
        let authenticated = if self.cfg.token_override.is_some() {
            status["via"] = "NUTRITION_API_TOKEN".into();
            // Org API keys aren't cook.md JWTs — probing /api/subscription
            // with one always yields a confusing 401, so don't.
            status["subscription"] = "not_applicable (org API key)".into();
            true
        } else if let Some(stored) = self.auth.current().filter(|s| s.expires_at > now_unix()) {
            status["via"] = "cook.md login".into();
            status["email"] = stored.email.into();
            status["token_expires_at_unix"] = stored.expires_at.into();
            probe_subscription = true;
            true
        } else {
            status["via"] = serde_json::Value::Null;
            false
        };
        status["authenticated"] = authenticated.into();
        if probe_subscription {
            match self.api.cookmd_get("/api/subscription").await {
                Ok(ApiOutcome::Ok(body)) => status["subscription"] = body,
                Ok(ApiOutcome::Err { status: code, body }) => {
                    status["subscription_error"] =
                        serde_json::json!({ "status": code, "body": body });
                }
                Err(e) => status["subscription_error"] = e.to_string().into(),
            }
        } else if !authenticated {
            status["hint"] = "Run the `login` tool, or set NUTRITION_API_TOKEN.".into();
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&status).unwrap_or_default(),
        )]))
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::text_of;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn server_for(mock_uri: &str) -> CookMcp {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.keep().join("auth.json");
        let cfg = crate::config::Config::from_vars(|k| match k {
            "NUTRITION_API_URL" | "COOKMD_BASE_URL" => Some(mock_uri.to_string()),
            "COOK_MCP_AUTH_PATH" => Some(auth_path.to_string_lossy().into_owned()),
            _ => None,
        });
        CookMcp::new(
            cfg,
            crate::workspace::Workspace::new(std::env::temp_dir()).unwrap(),
        )
    }

    fn server_for_with_token(mock_uri: &str, token: &str) -> CookMcp {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.keep().join("auth.json");
        let cfg = crate::config::Config::from_vars(|k| match k {
            "NUTRITION_API_URL" | "COOKMD_BASE_URL" => Some(mock_uri.to_string()),
            "NUTRITION_API_TOKEN" => Some(token.to_string()),
            "COOK_MCP_AUTH_PATH" => Some(auth_path.to_string_lossy().into_owned()),
            _ => None,
        });
        CookMcp::new(
            cfg,
            crate::workspace::Workspace::new(std::env::temp_dir()).unwrap(),
        )
    }

    /// Full aggregate-response shape cookmd-nutrition-client deserializes (mirrors
    /// the known-good fixture in render.rs tests).
    fn agg_response(kcal: f64) -> serde_json::Value {
        serde_json::json!({
            "items": [],
            "failures": [],
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

    #[tokio::test]
    async fn get_nutrition_success_returns_body_json() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .and(query_param("ingredient", "salmon"))
            .and(query_param("amount", "150"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"kcal": 312})),
            )
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .get_nutrition(Parameters(GetNutritionArgs {
                ingredient: "salmon".into(),
                amount: 150.0,
                unit: Some("g".into()),
                prep: None,
                region: None,
                reference: None,
            }))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["kcal"], 312);
    }

    #[tokio::test]
    async fn get_nutrition_error_is_error_result_with_problem_body() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nutrition"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "code": "ingredient_not_found", "suggestions": ["salmon"]})))
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .get_nutrition(Parameters(GetNutritionArgs {
                ingredient: "slamon".into(),
                amount: 1.0,
                unit: None,
                prep: None,
                region: None,
                reference: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["code"], "ingredient_not_found");
        assert_eq!(v["suggestions"][0], "salmon");
    }

    #[tokio::test]
    async fn aggregate_posts_items() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"totals": {"kcal": 99}})),
            )
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .aggregate_nutrition(Parameters(AggregateArgs {
                items: vec![AggregateItemArg {
                    ingredient: "salmon".into(),
                    amount: 150.0,
                    unit: Some("g".into()),
                    prep: None,
                    region: None,
                }],
                region: None,
                reference: None,
                exclusions: None,
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["totals"]["kcal"], 99);
    }

    // The service's AggregateRequest has no top-level `region` — region lives
    // per-item. The exact body match asserts the tool-level region default is
    // pushed onto each item (without clobbering an item's own region), that
    // no top-level `region` key is sent, and that `exclusions` pass top-level.
    #[tokio::test]
    async fn aggregate_applies_tool_region_per_item_not_top_level() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "items": [
                    {"ingredient": "salmon", "amount": 150.0, "unit": "g",
                     "prep": null, "region": "us"},
                    {"ingredient": "rice", "amount": 1.0, "unit": "cup",
                     "prep": null, "region": "uk"},
                ],
                "exclusions": ["nuts"]
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"totals": {"kcal": 42}})),
            )
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .aggregate_nutrition(Parameters(AggregateArgs {
                items: vec![
                    AggregateItemArg {
                        ingredient: "salmon".into(),
                        amount: 150.0,
                        unit: Some("g".into()),
                        prep: None,
                        region: None,
                    },
                    AggregateItemArg {
                        ingredient: "rice".into(),
                        amount: 1.0,
                        unit: Some("cup".into()),
                        prep: None,
                        region: Some("uk".into()),
                    },
                ],
                region: Some("us".into()),
                reference: None,
                exclusions: Some(vec!["nuts".into()]),
            }))
            .await
            .unwrap();
        // An unmatched body means the mock 404s and the result is an error.
        assert_ne!(
            r.is_error,
            Some(true),
            "body did not match: {}",
            text_of(&r)
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["totals"]["kcal"], 42);
    }

    #[tokio::test]
    async fn check_category_rejects_bad_slug() {
        let mock = MockServer::start().await;
        let s = server_for(&mock.uri());
        let r = s
            .check_category(Parameters(CategoryArgs {
                slug: "Vegetables/../admin".into(),
                ingredient: "carrot".into(),
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("invalid category slug"));
    }

    #[tokio::test]
    async fn branded_lookup_requires_exactly_one_of_upc_q() {
        let mock = MockServer::start().await;
        let s = server_for(&mock.uri());
        let r = s
            .branded_lookup(Parameters(BrandedArgs { upc: None, q: None }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("exactly one"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_report_tool_happy_path() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(agg_response(312.0)))
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .render_report(Parameters(RenderReportArgs {
                template: Some("kcal: {{ total_calories(ingredients) }}".into()),
                template_path: None,
                input: Some("@salmon{150%g}".into()),
                input_path: None,
                kind: Some("cook".into()),
                base_path: None,
                scale: None,
                client_profile_path: None,
            }))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true), "body: {}", text_of(&r));
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert!(v["rendered"].as_str().unwrap().contains("312"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_report_renders_inline_menu_via_plan_context() {
        // Inline .menu with loose items only (no recipe files needed) rendered
        // through the plan.* context — the menu path must not go through the
        // .cook recipe parser.
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(agg_response(500.0)))
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        // Inline menus require an explicit base_path (recipe-directory root);
        // loose-item-only menus never read from it, so any dir works.
        let base_dir = tempfile::tempdir().unwrap();
        let r = s
            .render_report(Parameters(RenderReportArgs {
                template: Some(
                    "items: {{ plan.all_ingredients | length }} \
                     kcal: {{ total_calories(plan.all_ingredients) }}"
                        .into(),
                ),
                template_path: None,
                input: Some("= Snacks =\n\n@apples{5}\n@kefir{1%l}\n".into()),
                input_path: None,
                kind: Some("menu".into()),
                base_path: Some(base_dir.path().to_string_lossy().into_owned()),
                scale: None,
                client_profile_path: None,
            }))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true), "body: {}", text_of(&r));
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        let rendered = v["rendered"].as_str().unwrap();
        assert!(rendered.contains("items: 2"), "got: {rendered}");
        assert!(rendered.contains("kcal: 500"), "got: {rendered}");
    }

    #[tokio::test]
    async fn render_report_rejects_both_template_forms() {
        let mock = MockServer::start().await;
        let s = server_for(&mock.uri());
        let r = s
            .render_report(Parameters(RenderReportArgs {
                template: Some("x".into()),
                template_path: Some("/tmp/x.jinja".into()),
                input: Some("@a{1%g}".into()),
                input_path: None,
                kind: Some("cook".into()),
                base_path: None,
                scale: None,
                client_profile_path: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("exactly one"));
    }

    #[tokio::test]
    async fn login_tool_returns_user_code() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dev-1", "user_code": "ABCD-1234",
                "verification_uri": format!("{}/device", mock.uri()),
                "expires_in": 900, "interval": 5})))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "authorization_pending"})),
            )
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s.login().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["user_code"], "ABCD-1234");
        assert!(v["verification_uri"].as_str().unwrap().contains("/device"));
    }

    // Task 9 carry-over: proves the spawned background poll task's success arm
    // actually stores the token so later tool calls authenticate.
    #[tokio::test]
    async fn login_background_poll_stores_token() {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = serde_json::json!({"email": "a@b.c", "exp": 4_102_444_800i64});
        let jwt = format!("h.{}.s", b64.encode(payload.to_string()));

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dev-1", "user_code": "ABCD-1234",
                "verification_uri": format!("{}/device", mock.uri()),
                "expires_in": 900, "interval": 1})))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth/device/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": jwt, "token_type": "Bearer"})))
            .mount(&mock)
            .await;

        let s = server_for(&mock.uri());
        let r = s.login().await.unwrap();
        assert_ne!(r.is_error, Some(true), "login failed: {}", text_of(&r));

        // The poll runs in a spawned task; give it a moment to store the token.
        let mut stored = None;
        for _ in 0..50 {
            if let Some(a) = s.auth.current() {
                stored = Some(a);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let stored = stored.expect("background poll never stored a token");
        assert_eq!(stored.email.as_deref(), Some("a@b.c"));
    }

    /// Unsigned test JWT with the given `exp` — StoredAuth::from_token only
    /// decodes the middle segment.
    fn test_jwt(exp: i64) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = serde_json::json!({"uid": 1, "email": "a@b.c", "exp": exp});
        format!("h.{}.s", b64.encode(payload.to_string()))
    }

    #[tokio::test]
    async fn auth_status_reports_subscription_for_stored_login() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "has_access": true, "plan_slug": "pro_early_adopter_v1",
                "email": "a@b.c"})))
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        s.auth.store_token(&test_jwt(4_102_444_800)).unwrap();
        let r = s.auth_status().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["authenticated"], true);
        assert_eq!(v["via"], "cook.md login");
        assert_eq!(v["email"], "a@b.c");
        assert_eq!(v["subscription"]["has_access"], true);
    }

    #[tokio::test]
    async fn auth_status_org_key_skips_subscription_probe() {
        // No /api/subscription mock mounted: if the tool probed cook.md it
        // would surface a subscription_error — org keys must skip the probe.
        let mock = MockServer::start().await;
        let s = server_for_with_token(&mock.uri(), "org-key");
        let r = s.auth_status().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["authenticated"], true);
        assert_eq!(v["via"], "NUTRITION_API_TOKEN");
        assert_eq!(v["subscription"], "not_applicable (org API key)");
        assert!(
            v.get("subscription_error").is_none(),
            "probe should be skipped, got: {v}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_report_unauthenticated_adds_login_hint() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/aggregate"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "code": "unauthorized", "detail": "missing bearer token"})))
            .mount(&mock)
            .await;
        let s = server_for(&mock.uri());
        let r = s
            .render_report(Parameters(RenderReportArgs {
                template: Some("kcal: {{ total_calories(ingredients) }}".into()),
                template_path: None,
                input: Some("@salmon{150%g}".into()),
                input_path: None,
                kind: Some("cook".into()),
                base_path: None,
                scale: None,
                client_profile_path: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let text = text_of(&r);
        assert!(text.contains("authentication required"), "got: {text}");
        assert!(text.contains("login"), "got: {text}");
    }
}
