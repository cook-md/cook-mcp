# Nutrition goals: measure → edit → verify

You are adjusting a recipe or meal plan to hit numeric nutrition targets
(e.g. "under 1800 kcal/day", "≥30% protein", "less sodium").

## Loop

1. **Parse the target** into nutrient keys and bounds (per person, per day
   for plans; per serving for recipes unless the user says otherwise).
2. **Measure the current state** — `render_report` with a small measuring
   template, e.g.:
   `{% set m = macros(ingredients) %}{{ m.kcal }} kcal, {{ m.protein_g }} g protein`
   (for plans, iterate `plan.*` days/meals). Or call `aggregate_nutrition`
   directly with the ingredient list.
3. **Edit** the recipe/plan files with your file tools: swap ingredients,
   adjust amounts/servings, or swap referenced recipes in a `.menu`.
   Prefer edits that keep the dish coherent; mention trade-offs.
4. **Re-measure and verify** with the same measuring template so numbers are
   comparable. Use `within_tol`/`compare` semantics: get within tolerance
   (±5% unless the user sets one), don't chase exact figures.
5. **Report** before → after per target, and note any `resolve_failures`
   that make numbers partial (`confidence` fields tell you coverage).

Ingredient swap candidates: `lookup_ingredient` for catalog names,
`check_category` to keep swaps in-category (e.g. oily fish), `get_nutrition`
to compare per-100g profiles before committing an edit.
