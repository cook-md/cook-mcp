---
name: report-authoring
description: Use when the user wants to write or run a custom Jinja report template with render_report (a computed value, a custom summary or printout), or save a reusable report template. For nutrition use nutrition-reports.
---

# Skill: Report Authoring

Author and run **Jinja2 report templates** over the user's recipes with the
`render_report` tool — or reuse a saved one — then answer from the output or
present the report. Use this skill when the user wants a computed value
(ingredient counts, a scaled printout), a custom report or summary, or a new
report template. For nutrition or dietitian-style evaluations, follow the
nutrition-reports skill instead.

Templates that don't call nutrition functions render locally, with no login.

## The tool

`render_report` renders one template against one `.cook` recipe or `.menu`
plan and returns `{ "rendered", "checks", "resolve_failures" }`, or an error
naming the template line. It never edits files.

Arguments — the template, **exactly one** of:
- `template` — inline Jinja2 source, for drafts and one-off reports.
- `template_path` — a saved template file, relative to the collection root
  (e.g. `config/reports/cost.md.jinja`). Prefer it whenever a suitable saved
  template exists.

The input, **exactly one** of:
- `input_path` — a `.cook` or `.menu` file as `list_recipes` returned it (kind
  inferred from the extension).
- `input` — inline Cooklang text, with `kind` (`"cook"` or `"menu"`).

Optional:
- `scale` — recipe scale factor (default 1). Scales `.cook` recipes only, not
  `.menu` quantities.
- `base_path` — folder that `@./` references resolve from; defaults to the
  collection root, which is almost always right.

## Template context and filters

Within a template you have:
- `ingredients` — list; each item has `.name` and `.quantity`.
- `metadata` — recipe metadata, including `metadata.title` and any custom keys.
- `scale` — the numeric scale factor.
- For `.menu` input: `plan` — the plan's days and meals with their expanded
  ingredients (a menu is read through `plan.*`, not `ingredients`).

Filters available:
- Standard Jinja: `sort(attribute='name')`, `default(...)`, `round`, `join`,
  `selectattr`, `items`, `length`.
- Text: `titleize`, `humanize`, `upcase_first`.
- Numbers: `number_with_precision`, `number_with_delimiter`,
  `number_to_percentage`, `number_to_currency`, `numeric`.

Example — an ingredients list:

```jinja
# {{ metadata.title | default("Ingredients") }}

{% for ingredient in ingredients | sort(attribute='name') -%}
- {{ ingredient.name }}{% if ingredient.quantity %}: {{ ingredient.quantity }}{% endif %}
{% endfor %}
```

For a shopping list, use the `shopping_list` tool rather than a template: it
merges duplicates, groups by aisle and subtracts the pantry.

## Workflow: reuse or author -> render -> present

1. Saved templates conventionally live in `config/reports/` (or `reports/`) in
   the collection. If the user names one, or your client can list that folder,
   render it with `template_path` and skip to step 4.
2. Otherwise draft the template inline.
3. Call `render_report` with `template`. If it errors, read the message
   (minijinja reports the line), fix the template, and render again until it
   renders cleanly.
4. To answer a question (e.g. "how many ingredients?"), read `rendered` and
   reply with the answer. To present a report, show the rendered markdown to
   the user as it came back.

## Saving a reusable template

When the user wants to keep a report, save the template as a `.jinja` file:
- Put templates in `config/reports/` by convention.
- Declare the output format via the inner extension:
  `weekly-cost.md.jinja` -> markdown, `menu.html.jinja` -> HTML,
  `shopping.txt.jinja` -> plain text.
- cook-mcp's write tools only write `.cook` and `.menu` files. Save the
  template with your client's own file tools if it has them; otherwise give
  the user the full template and the path to save it at.
- From then on, render it with `template_path` — never re-send the source
  inline.

Render inline for one-off questions; offer once to save a template, and save
only if the user accepts.
