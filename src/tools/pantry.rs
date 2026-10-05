//! Free pantry tools over config/pantry.conf, backed by cookcli-core.

use camino::Utf8Path;
use cookcli_core::{
    ConfigSource, Context,
    pantry::{self, PantryItem},
};
use rmcp::{handler::server::wrapper::Parameters, model::*, schemars, tool, tool_router};

use super::{core_err, json_ok, text_err};
use crate::server::CookMcp;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PantryListArgs {
    /// Only this section, e.g. "fridge".
    pub section: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExpiringArgs {
    /// Window in days. Default 7.
    pub days: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PantryRecipesArgs {
    /// Minimum % of a recipe's ingredients in stock to list it as a partial match. Default 50.
    pub threshold: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PantryItemArg {
    pub section: String,
    pub name: String,
    /// e.g. "500%g", "2".
    pub quantity: Option<String>,
    /// Expiry date YYYY-MM-DD.
    pub expire: Option<String>,
    /// Purchase date YYYY-MM-DD.
    pub bought: Option<String>,
    /// Low-stock threshold, e.g. "100%g".
    pub low: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PantryRemoveArg {
    pub section: String,
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PantryUpdateArgs {
    #[serde(default)]
    pub add: Vec<PantryItemArg>,
    #[serde(default)]
    pub update: Vec<PantryItemArg>,
    #[serde(default)]
    pub remove: Vec<PantryRemoveArg>,
}

fn item_json(i: &PantryItem) -> serde_json::Value {
    serde_json::json!({
        "name": i.name, "section": i.section, "quantity": i.quantity,
        "bought": i.bought, "expire": i.expire, "low": i.low,
    })
}

/// True when the discovered pantry is a file inside the recipe root. `discover`
/// falls back to the global config dir, which pantry writes must never touch.
fn pantry_in_root(ctx: &Context, root: &Utf8Path) -> bool {
    matches!(ctx.pantry(), ConfigSource::Path(p) if p.starts_with(root))
}

#[tool_router(router = pantry_router, vis = "pub(crate)")]
impl CookMcp {
    #[tool(description = "List what's in the pantry (config/pantry.conf), by section.")]
    async fn pantry_list(
        &self,
        Parameters(a): Parameters<PantryListArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(e) = self.unset_guard() {
            return Ok(e);
        }
        Ok(
            match pantry::list(
                &self.workspace.context(),
                pantry::ListRequest { section: a.section },
            ) {
                Err(e) => core_err(e),
                Ok(o) => json_ok(&serde_json::json!({
                    "sections": o.value.sections.iter().map(|s| serde_json::json!({
                        "name": s.name, "items": s.items.iter().map(item_json).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                    "diagnostics": o.diagnostics,
                })),
            },
        )
    }

    #[tool(
        description = "Pantry items expiring within `days` (default 7). Use it to plan meals \
        that use them up."
    )]
    async fn pantry_expiring(
        &self,
        Parameters(a): Parameters<ExpiringArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(e) = self.unset_guard() {
            return Ok(e);
        }
        let req = pantry::ExpiringRequest {
            days: a.days.unwrap_or(7),
            include_unknown: false,
        };
        Ok(match pantry::expiring(&self.workspace.context(), req) {
            Err(e) => core_err(e),
            Ok(o) => json_ok(&serde_json::json!({
                "items": o.value.iter().map(|e| serde_json::json!({
                    "item": item_json(&e.item), "expire_date": e.expire_date, "days_until_expiry": e.days_until_expiry,
                })).collect::<Vec<_>>(),
                "diagnostics": o.diagnostics,
            })),
        })
    }

    #[tool(description = "Pantry items at or below their low-stock threshold.")]
    async fn pantry_depleted(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(e) = self.unset_guard() {
            return Ok(e);
        }
        Ok(
            match pantry::depleted(
                &self.workspace.context(),
                pantry::DepletedRequest { all: false },
            ) {
                Err(e) => core_err(e),
                Ok(o) => json_ok(&serde_json::json!({
                    "items": o.value.iter().map(item_json).collect::<Vec<_>>(),
                    "diagnostics": o.diagnostics,
                })),
            },
        )
    }

    #[tool(
        description = "What can I cook with what's in the pantry: recipes fully covered, and \
        partial matches with the missing ingredients."
    )]
    async fn pantry_recipes(
        &self,
        Parameters(a): Parameters<PantryRecipesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(e) = self.unset_guard() {
            return Ok(e);
        }
        let req = pantry::RecipesRequest {
            threshold: a.threshold.unwrap_or(50),
        };
        Ok(match pantry::recipes(&self.workspace.context(), req) {
            Err(e) => core_err(e),
            Ok(o) => json_ok(&serde_json::json!({
                "full": o.value.full,
                "partial": o.value.partial.iter().map(|p| serde_json::json!({
                    "name": p.name, "percentage": p.percentage, "missing": p.missing,
                })).collect::<Vec<_>>(),
                "diagnostics": o.diagnostics,
            })),
        })
    }

    #[tool(
        description = "Add, update or remove pantry items in config/pantry.conf. Applied in \
        order add → update → remove; stops at the first failure and reports what was applied."
    )]
    async fn pantry_update(
        &self,
        Parameters(a): Parameters<PantryUpdateArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(e) = self.unset_guard() {
            return Ok(e);
        }
        let ctx = self.workspace.context();
        if !pantry_in_root(&ctx, self.workspace.root()) {
            return Ok(text_err(
                "No pantry in the recipe folder. Writes only touch <recipe root>/config/pantry.conf \
                 (a global pantry, if any, is never modified): create it first with write_config \
                 (path \"config/pantry.conf\").",
            ));
        }
        let mut applied = Vec::new();
        for i in a.add {
            let label = format!("add {}/{}", i.section, i.name);
            let req = pantry::AddRequest {
                section: i.section,
                name: i.name,
                quantity: i.quantity,
                bought: i.bought,
                expire: i.expire,
                low: i.low,
            };
            if let Err(e) = pantry::add(&ctx, req) {
                return Ok(super::json_err(
                    &serde_json::json!({ "applied": applied, "failed": label, "error": e.to_string() }),
                ));
            }
            applied.push(label);
        }
        for i in a.update {
            let label = format!("update {}/{}", i.section, i.name);
            let req = pantry::UpdateRequest {
                section: i.section,
                name: i.name,
                quantity: i.quantity,
                bought: i.bought,
                expire: i.expire,
                low: i.low,
            };
            if let Err(e) = pantry::update(&ctx, req) {
                return Ok(super::json_err(
                    &serde_json::json!({ "applied": applied, "failed": label, "error": e.to_string() }),
                ));
            }
            applied.push(label);
        }
        for i in a.remove {
            let label = format!("remove {}/{}", i.section, i.name);
            if let Err(e) = pantry::remove(
                &ctx,
                pantry::RemoveRequest {
                    section: i.section,
                    name: i.name,
                },
            ) {
                return Ok(super::json_err(
                    &serde_json::json!({ "applied": applied, "failed": label, "error": e.to_string() }),
                ));
            }
            applied.push(label);
        }
        Ok(json_ok(&serde_json::json!({ "applied": applied })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture_workspace;
    use crate::tools::local::tests::server;
    use crate::tools::text_of;

    fn json(r: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&text_of(r)).unwrap()
    }

    #[tokio::test]
    async fn list_then_add_then_remove() {
        let (d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.pantry_list(Parameters(PantryListArgs { section: None }))
                .await
                .unwrap(),
        );
        assert!(v.to_string().contains("butter"));

        s.pantry_update(Parameters(PantryUpdateArgs {
            add: vec![PantryItemArg {
                section: "fridge".into(),
                name: "leeks".into(),
                quantity: Some("3".into()),
                expire: None,
                bought: None,
                low: None,
            }],
            update: vec![],
            remove: vec![],
        }))
        .await
        .unwrap();
        assert!(
            std::fs::read_to_string(d.path().join("config/pantry.conf"))
                .unwrap()
                .contains("leeks")
        );

        s.pantry_update(Parameters(PantryUpdateArgs {
            add: vec![],
            update: vec![],
            remove: vec![PantryRemoveArg {
                section: "fridge".into(),
                name: "leeks".into(),
            }],
        }))
        .await
        .unwrap();
        assert!(
            !std::fs::read_to_string(d.path().join("config/pantry.conf"))
                .unwrap()
                .contains("leeks")
        );
    }

    #[tokio::test]
    async fn expiring_finds_butter() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.pantry_expiring(Parameters(ExpiringArgs { days: Some(36500) }))
                .await
                .unwrap(),
        );
        assert!(v.to_string().contains("butter"));
    }

    #[tokio::test]
    async fn what_can_i_cook() {
        let (_d, ws) = fixture_workspace();
        let s = server(ws);
        let v = json(
            &s.pantry_recipes(Parameters(PantryRecipesArgs { threshold: Some(1) }))
                .await
                .unwrap(),
        );
        assert!(v["partial"].to_string().contains("Pancakes"), "{v}");
    }

    #[test]
    fn pantry_in_root_helper() {
        let root = Utf8Path::new("/r");
        let mk = |s: ConfigSource| Context::new("/r".into()).with_pantry(s);
        assert!(pantry_in_root(
            &mk(ConfigSource::Path("/r/config/pantry.conf".into())),
            root
        ));
        assert!(!pantry_in_root(
            &mk(ConfigSource::Path(
                "/home/u/.config/cook/pantry.conf".into()
            )),
            root
        ));
        assert!(!pantry_in_root(&mk(ConfigSource::None), root));
        assert!(!pantry_in_root(&mk(ConfigSource::Inline("x".into())), root));
    }
}
