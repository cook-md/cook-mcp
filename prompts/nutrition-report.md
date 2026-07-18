# Nutrition report authoring

You are writing a **jinja nutrition report** for a `.cook` recipe or `.menu`
plan, rendered by the `render_report` tool (cook.md nutrition service).

## Loop

1. Draft the template (start inline via the `template` argument; save to a
   `.jinja` file once it works and switch to `template_path`).
2. Call `render_report`. Read `rendered`, `checks`, and `resolve_failures`.
3. Fix and re-render until `resolve_failures` is empty and all checks pass.
   - `ingredient_not_found` → try `lookup_ingredient`; retry with a
     suggestion, or fix the ingredient name in the recipe. Do NOT bake
     qualifiers into names — `fresh garlic` should be `@garlic{}(fresh)`.
   - `density_unavailable` / unit errors → probe with `convert_units`; prefer
     mass units in the recipe when density is missing.
4. Dietary targets/exclusions: pass a client profile YAML via
   `client_profile_path`, or define targets inline in the template.

## Template cheatsheet

Functions (full reference: `<NUTRITION_API_URL>/docs/guides/functions`):
`nutrition_for(ingredient)` (macros), `nutrition_for_amount(name, amount,
unit, prep, standard?)` (incl. micros/vitamins), `aggregate_nutrition(
ingredients)`, `macros(ingredients)`, `total_calories(ingredients)`,
`vitamins(ingredients)`, `nutrient_total(ingredients, key)`,
`reference_intake(name, standard?)`, `dv_percent(name, amount, standard?)`,
`is_in_category(ingredient, slug)`, `category_servings(plan, slug)`,
`convert(amount, from, to, ingredient?)`, `compare(actual, target, op)`,
`within_tol(actual, target, tol_pct)`, `record_check(label, ok)`,
`all_checks()`, `failed_checks()`, `matched_exclusions(ingredients,
exclusions)`, `unresolved_exclusions(ingredients, exclusions)`.

Check macros — `{% import "ck" as ck %}`: `ck.min/max/range/within(value,
target, label)`, `ck.between(value, lo, hi, label)`, `ck.absent(ingredient)`.
Recorded checks come back in `render_report`'s `checks` output.

For `.menu` plans the template reads the plan via `plan.*` (days/meals with
expanded ingredients); recipes render via the recipe context directly.
