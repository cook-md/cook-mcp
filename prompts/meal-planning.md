# Meal planning (.menu files)

You are building or editing a Cooklang `.menu` meal plan. If the work is
driven by numeric nutrition targets ("1800 kcal/day", "35% protein"), use the
`nutrition-goals` prompt instead.

## Workflow

1. Read existing plans and recipes in the workspace (`**/*.menu`,
   `**/*.cook`) with your file tools; follow the user's layout and naming.
2. A `.menu` may only reference recipes that exist locally — reference them
   as `@./path/to/recipe{servings}`; never inline steps, never reference a
   file you haven't confirmed exists.
3. Match servings/yield metadata to head-count; cooking once and eating
   twice is fine. Mis-scaling propagates into every downstream number.
4. Save plans in the user's plans/menus folder (or alongside existing ones).
5. Evaluate: `render_report` with `input_path` = the `.menu` file (kind is
   inferred) and a nutrition template; `base_path` = the recipe-directory
   root so `@./` references resolve. Fix any `resolve_failures`.
6. Use YAML frontmatter for any metadata — never the deprecated `>>` syntax.
