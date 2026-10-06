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
            body: include_str!(concat!("../skills/", $n, "/SKILL.md")),
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
    skill!("scale-recipe"),
    skill!("recipe-search"),
    skill!("export-recipe"),
    skill!("organize-collection"),
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

    #[prompt(
        name = "scale-recipe",
        description = "Show a recipe scaled to more or fewer servings"
    )]
    async fn scale_recipe_prompt(&self) -> Vec<PromptMessage> {
        prompt_body("scale-recipe")
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

    /// Claude Code loads every `skills/<name>/SKILL.md` as a plugin skill; the
    /// MCP server serves only what `SKILLS` registers. Keep the two the same.
    #[test]
    fn every_skill_dir_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("SKILL.md").is_file())
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        on_disk.sort();
        let mut registered: Vec<String> = SKILLS.iter().map(|s| s.name.to_string()).collect();
        registered.sort();
        assert_eq!(on_disk, registered);
        assert_eq!(SKILLS.len(), 14);
    }

    /// Plugin skill names: lowercase, digits and hyphens, at most 64 chars, no
    /// reserved words. Descriptions are auto-invocation triggers.
    #[test]
    fn skills_are_valid_plugin_skills() {
        for s in SKILLS {
            assert!(
                s.name.len() <= 64
                    && s.name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "bad skill name {}",
                s.name
            );
            for reserved in ["claude", "anthropic"] {
                assert!(!s.name.contains(reserved), "reserved word in {}", s.name);
            }
            let d = s.description().unwrap_or_default();
            assert!(
                d.starts_with("Use when"),
                "{}: description should start with \"Use when\"",
                s.name
            );
            assert!(d.len() <= 1024, "{}: description too long", s.name);
            // Plain YAML scalar: `: ` or ` #` would break the frontmatter for
            // Claude Code's YAML parser (our own reader is line-based).
            assert!(
                !d.contains(": ") && !d.contains(" #"),
                "{}: description must be a plain YAML scalar",
                s.name
            );
        }
    }

    /// As a plugin skill, a skill can load without the server. Each one says,
    /// client-neutrally, how to get it connected instead of guessing.
    #[test]
    fn skills_say_how_to_add_the_server() {
        for s in SKILLS {
            assert!(
                s.body.contains("the Cook MCP server isn't connected")
                    && s.body.contains("(e.g. `/mcp`) or reinstall it")
                    && s.body
                        .contains("add it from https://github.com/cook-md/cook-mcp")
                    && !s.body.contains("claude mcp add"),
                "skill {} lacks the add-the-server line",
                s.name
            );
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
