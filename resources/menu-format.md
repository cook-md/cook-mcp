# .menu meal plans

A `.menu` file is Cooklang whose ingredients are mostly references to recipes in the collection.

- Reference a recipe as `@./Folder/Recipe Name{N%servings}`: path from the collection root, no
  `.cook` extension, `N%servings` scales it to N servings (needs a numeric `servings` in that
  recipe's frontmatter; without one the number becomes a plain multiplier, so `{3%servings}`
  triples the recipe). `{}` means as written; a bare factor such as `{1.5}` scales by that factor.
- Plain ingredients (`@coffee{1%cup}`) are allowed for things that aren't recipes.
- Never inline a recipe's steps in a plan. Never reference a file you haven't seen in
  `list_recipes` / `search_recipes`.
- Sections group days or weeks: `== Day 1 (2026-03-07) ==`. Use `YYYY-MM-DD` dates in day
  sections; apps use them to jump to today's plan.
- Meals are blocks: a label line (`Dinner:`) and its `- ` item lines. End every line of a block
  with a trailing `\` except the last one; a blank line separates blocks. The `\` is a soft line
  break that keeps the block together as one step.
- Metadata goes in YAML frontmatter (`---` … `---`), e.g. `servings: 2` (how many people eat;
  reports divide totals by it, it does not scale the references) and `description`.

Example:

    ---
    servings: 2
    description: Two weekday dinners for two.
    ---

    == Day 1 (2026-10-06) ==

    Breakfast: \
    - @./Breakfast/Pancakes{2%servings}

    Dinner: \
    - @./Dinner/Pasta{2%servings} \
    - @green salad{1%bowl}

    == Snacks ==

    - @apples{4} \
    - @mixed nuts{100%g}

`write_menu` checks every reference before saving. `shopping_list` takes the plan and follows its
references. `render_report` with a nutrition template evaluates it (the template reads the plan
through `plan.*`).
