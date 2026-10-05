# Cooklang — Syntax & Scaling

## Basics

- A recipe is plain text; each paragraph is a step. Ingredients, cookware and timers are marked
  inline where the step uses them. There is no separate ingredient list: it is derived from the
  steps.
- Ingredient: `@salt`, `@potato{2}`, `@olive oil{2%tbsp}` (multi-word names need `{}`; `%`
  separates amount and unit).
- Cookware: `#pot`, `#baking sheet{}`.
- Timer: `~{25%minutes}`, or named `~eggs{3%minutes}`.
- Comments: `-- to end of line`, `[- block -]`. Notes: a line starting with `>`.
- Sections: `= Dough` (or `== Dough ==`).
- Metadata: YAML frontmatter between `---` lines at the top of the file.
- The first mention of an ingredient carries its quantity; later mentions are bare (`@bacon`).

Example:

```
---
title: Garlic Toast
servings: 2
tags: [snack, quick]
---

Rub @sourdough bread{4%slices} with @garlic{1%clove}(halved).

Drizzle with @olive oil{1%tbsp} and toast in a #frying pan{} for ~{3%minutes}.
```

## Ingredients (full)

- With quantity: `@potato{2}`, `@chicken{500%g}`, `@milk{1/2%cup}`
- Fixed (don't scale): `@salt{=1%tsp}` — stays the same regardless of servings. `validate` warns "Unnecessary scaling lock modifier" on `{=…}`: a known parser quirk; keep the `=` and ignore that warning.
- With preparation: `@onion{1}(peeled and chopped)`
- Recipe reference: `@./sauces/hollandaise{150%g}` — references another .cook file. Use the file path relative to the recipes folder. NEVER include the `.cook` extension.

Scaling rules for referenced recipes:
1. No units provided → scales the whole referenced recipe by the given factor.
2. Servings provided → reads the referenced recipe's `servings` metadata and computes the factor to match (`{3%servings}` of a 6-serving recipe = ×0.5). If the recipe has no numeric `servings`, the number is used as a plain multiplier instead (`{3%servings}` = ×3).
3. Units provided → reads the referenced recipe's `yield` value and computes the factor (same units only, for now).

## Metadata (YAML frontmatter)

Metadata is YAML frontmatter between `---` markers at the file start. `servings`/`serves` and `yield` drive scaling (see Ingredients above and the scaling rules). For the full field list and any metadata-focused work, read the `cooklang://skills/metadata` resource.

## Menu files (.menu)

A .menu file is a valid Cooklang file containing mostly sections (days) and links to other recipes. Reference recipes with `@./path/to/recipe{N%servings}` relative to the recipes root — never include the `.cook` extension. The full format is in the `cooklang://menu-format` resource. Days and meals are written as blocks. A block is a labelled group (e.g. `Breakfast:` and its `- @item` lines, or a `== Snacks ==` list). End every line of a block with a trailing `\` EXCEPT the last line of the block; a blank line separates blocks. The `\` is a soft line break that keeps the block as one rendered step.

Example:

```
---
servings: 3
description: |
    3-day plan for family of 3 with light low-carb dinners.
---

== Day 1 (2026-04-17) ==

Breakfast: \
- @./Breakfast/Oats{3%servings} with @sour cream{3%tbsp} \
- @filter coffee{2%cup} and @tea{1%cup}

Lunch: \
- @./Slowcooker/Slow Cooker Tuscan Chicken{3%servings} with @rice{}(boiled) \
- @./Salads/Caprese{3%servings}

Dinner: \
- @./Salads/Boring With Sour Cream{3%servings}

== Snacks ==

- @apples{5} \
- @mixed nuts{150%g}

== Batch Prep ==

> Day 0: Make Tuscan Chicken (4-5 servings) in slow cooker for Days 1-2 lunches
```

## Images

Give a recipe an image by placing an image file (`.png`/`.jpg`) next to the `.cook` with the same base name:

```
Baked Potato.cook
Baked Potato.jpg
```

For a step image, put the step number before the extension:

```
Chicken French.cook
Chicken French.0.jpg
Chicken French.3.jpg
```

Alternatively, set the `image` metadata key to a URL.
