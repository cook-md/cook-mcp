//! Cooklang knowledge shipped with the server: MCP resources (spec, syntax,
//! .menu format, skills) and workflow prompts. All compiled in.

use rmcp::{model::*, prompt, prompt_router};

use crate::server::CookMcp;

pub struct Doc {
    pub uri: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

pub struct Skill {
    pub name: &'static str,
    pub body: &'static str,
}

impl Skill {
    pub fn uri(&self) -> String {
        format!("cooklang://skills/{}", self.name)
    }

    /// The one-line `description:` from the skill's YAML frontmatter.
    pub fn description(&self) -> Option<&'static str> {
        let front = self.body.strip_prefix("---\n")?;
        let front = &front[..front.find("\n---")?];
        front
            .lines()
            .find_map(|l| l.strip_prefix("description:"))
            .map(str::trim)
            .filter(|d| !d.is_empty() && *d != ">")
    }
}

pub const RESOURCES: &[Doc] = &[
    Doc {
        uri: "cooklang://spec",
        name: "Cooklang specification",
        description: "The full Cooklang language spec",
        body: include_str!("../resources/spec.md"),
    },
    Doc {
        uri: "cooklang://syntax",
        name: "Cooklang syntax reference",
        description: "Compact syntax reference: ingredients, cookware, timers, references, frontmatter",
        body: include_str!("../resources/syntax.md"),
    },
    Doc {
        uri: "cooklang://menu-format",
        name: ".menu meal plan format",
        description: "How .menu plans reference recipes and scale servings",
        body: include_str!("../resources/menu-format.md"),
    },
];

macro_rules! skill {
    ($n:literal) => {
        Skill {
            name: $n,
            body: include_str!(concat!("../skills/", $n, ".md")),
        }
    };
}

pub const SKILLS: &[Skill] = &[
    skill!("cooklang-editing"),
    skill!("cooklang-validation"),
    skill!("meal-planning"),
    skill!("shopping-list"),
    skill!("pantry"),
    skill!("recipe-import"),
    skill!("report-authoring"),
    skill!("nutrition-reports"),
    skill!("nutrition-goals"),
    skill!("metadata"),
];

pub fn read(uri: &str) -> Option<&'static str> {
    if let Some(name) = uri.strip_prefix("cooklang://skills/") {
        return SKILLS.iter().find(|s| s.name == name).map(|s| s.body);
    }
    RESOURCES.iter().find(|r| r.uri == uri).map(|r| r.body)
}

pub fn list() -> Vec<Resource> {
    RESOURCES
        .iter()
        .map(|r| {
            Resource::new(r.uri, r.name)
                .with_description(r.description)
                .with_mime_type("text/markdown")
        })
        .chain(SKILLS.iter().map(|s| {
            let res = Resource::new(s.uri(), s.name).with_mime_type("text/markdown");
            match s.description() {
                Some(d) => res.with_description(d),
                None => res,
            }
        }))
        .collect()
}

pub const INSTRUCTIONS: &str = "\
You are working in a Cooklang recipe collection (.cook recipes, .menu meal plans, config/aisle.conf, \
config/pantry.conf). Rules: never invent recipes, only reference ones list_recipes/search_recipes \
returned; use paths exactly as tools return them; run validate before and after editing; metadata \
is YAML frontmatter, never `>>` lines; write_recipe/write_menu save for real, so say what you saved. \
Read cooklang://syntax before writing Cooklang; task guides are the cooklang://skills/* resources. \
Free, no login: list_recipes, read_recipe, search_recipes, validate, write_recipe, write_menu, \
shopping_list, pantry_*, write_config (aisle.conf, pantry.conf, .jinja report templates), render_report \
(plain templates), import_recipe from a web page or text. \
Cook Basic/Pro (cook.md): nutrition tools, nutrition reports, photo and social imports. On \
login_required call `login`; on plan_required show the user checkout_url.";

fn prompt_body(skill: &str) -> Vec<PromptMessage> {
    let body = SKILLS
        .iter()
        .find(|s| s.name == skill)
        .map(|s| s.body)
        .unwrap_or_default();
    vec![PromptMessage::new_text(
        Role::User,
        format!(
            "{body}\n\nCooklang syntax reference: read the `cooklang://syntax` resource before writing Cooklang."
        ),
    )]
}

#[prompt_router(vis = "pub(crate)")]
impl CookMcp {
    #[prompt(
        name = "meal-planning",
        description = "Plan meals into a .menu file from your recipes"
    )]
    async fn meal_planning_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("meal-planning")
    }

    #[prompt(
        name = "shopping-list",
        description = "Build a shopping list from recipes or a meal plan"
    )]
    async fn shopping_list_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("shopping-list")
    }

    #[prompt(
        name = "pantry",
        description = "Track the pantry and cook from what you have"
    )]
    async fn pantry_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("pantry")
    }

    #[prompt(
        name = "import-recipe",
        description = "Import a recipe from a URL, photo or text into Cooklang"
    )]
    async fn import_recipe_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("recipe-import")
    }

    #[prompt(name = "edit-recipe", description = "Write or fix a Cooklang recipe")]
    async fn edit_recipe_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("cooklang-editing")
    }

    #[prompt(
        name = "nutrition-report",
        description = "Author and iterate a nutrition report for a recipe or plan"
    )]
    async fn nutrition_report_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("nutrition-reports")
    }

    #[prompt(
        name = "nutrition-goals",
        description = "Adjust a recipe or plan to hit nutrition targets"
    )]
    async fn nutrition_goals_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("nutrition-goals")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CookBot/editor tool names that don't exist in cook-mcp. A skill that
    /// still names one sends the agent after a tool it can't call.
    const FOREIGN_TOOLS: &[&str] = &[
        "getFileContent",
        "findFilesByPattern",
        "getWorkspaceFileList",
        "getWorkspaceDirectoryStructure",
        "suggestFileContent",
        "suggestFileReplacements",
        "updateRecipeMetadata",
        "getProposedFileState",
        "clearFileChanges",
        "searchRecipes",
        "searchRecipeCatalog",
        "addCatalogRecipe",
        "renderTemplate",
        "generateShoppingList",
        "openRecipeFolder",
        "searchWeb",
        "loadSkill",
        "fetchUrl",
        "convertUrlToCooklang",
        "convertTextToCooklang",
        "listReportTemplates",
        "getServerPreferences",
        "getPantry",
        "checkPantry",
    ];

    /// Prefixes our tool names start with; field names like `input_path` don't.
    const TOOL_LIKE_PREFIXES: &[&str] = &[
        "list_",
        "read_",
        "search_",
        "write_",
        "pantry_",
        "import_",
        "render_",
        "get_nutrition",
        "aggregate_",
        "lookup_",
        "convert_",
        "check_",
        "branded_",
        "reference_",
        "auth_",
        "shopping_",
    ];

    /// Template-side names that share a tool prefix but are jinja functions
    /// (or tool arguments), not tools.
    const NOT_TOOLS: &[&str] = &["reference_intake"];

    #[test]
    fn skills_name_only_cook_mcp_tools() {
        let tools: Vec<String> = CookMcp::all_tools()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for (name, body) in SKILLS.iter().map(|s| (s.name, s.body)) {
            for foreign in FOREIGN_TOOLS {
                assert!(
                    !body.contains(foreign),
                    "skill {name} still names `{foreign}`"
                );
            }
            assert!(
                !body.contains("staged") && !body.contains("proposal"),
                "skill {name} still talks about staged proposals; cook-mcp writes for real"
            );
            assert!(
                !body.lines().any(|l| l.trim_start().starts_with(">>")),
                "skill {name} shows `>>` metadata"
            );
            for word in body.split('`').skip(1).step_by(2) {
                // `name(args)` in prose is a jinja call; judge the bare name.
                let word = word.split('(').next().unwrap_or(word);
                let looks_like_tool = word.contains('_')
                    && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                    && TOOL_LIKE_PREFIXES.iter().any(|p| word.starts_with(p))
                    && !NOT_TOOLS.contains(&word);
                if looks_like_tool {
                    assert!(
                        tools.contains(&word.to_string()),
                        "skill {name} names unknown tool `{word}`"
                    );
                }
            }
        }
    }

    #[test]
    fn skills_keep_name_and_description_frontmatter() {
        for s in SKILLS {
            assert!(
                s.body.starts_with(&format!("---\nname: {}\n", s.name)),
                "skill {} frontmatter must start with its name",
                s.name
            );
            assert!(
                s.description().is_some(),
                "skill {} needs a one-line description",
                s.name
            );
            for key in ["attach_syntax_reference", "order:"] {
                assert!(!s.body.contains(key), "skill {} keeps `{key}`", s.name);
            }
        }
    }

    #[test]
    fn every_resource_reads_back() {
        for r in RESOURCES {
            assert!(read(r.uri).is_some(), "{}", r.uri);
        }
        for s in SKILLS {
            assert!(read(&s.uri()).is_some());
        }
        assert!(read("cooklang://nope").is_none());
        assert!(read("cooklang://skills/nope").is_none());
        assert_eq!(list().len(), RESOURCES.len() + SKILLS.len());
    }
}
