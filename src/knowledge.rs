//! Prompts (and, from Task 13, resources). Stub: the three workflow prompts
//! carried over from nutrition-mcp until the skills replace them.

use rmcp::{model::*, prompt, prompt_router};

use crate::server::CookMcp;

pub const INSTRUCTIONS: &str = "Cooklang recipe tools (free) plus cook.md nutrition and import.";

#[prompt_router(vis = "pub(crate)")]
impl CookMcp {
    #[prompt(
        name = "nutrition-report",
        description = "Author and iterate a jinja nutrition report for a .cook recipe or .menu plan"
    )]
    async fn nutrition_report_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("../prompts/nutrition-report.md"),
        )]
    }

    #[prompt(
        name = "meal-planning",
        description = "Build or edit a .menu meal plan from local recipes"
    )]
    async fn meal_planning_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("../prompts/meal-planning.md"),
        )]
    }

    #[prompt(
        name = "nutrition-goals",
        description = "Adjust a recipe or meal plan to hit numeric nutrition targets"
    )]
    async fn nutrition_goals_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("../prompts/nutrition-goals.md"),
        )]
    }
}
