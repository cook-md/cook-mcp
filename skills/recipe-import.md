---
name: recipe-import
description: Use when bringing a recipe in from a URL, photos, or pasted text. Triggers - "import", "convert this", a pasted recipe, a link, a photo of a cookbook page. Not for editing an existing local recipe.
---

# Skill: recipe-import

Use when bringing a recipe in from a URL, photos, or pasted text.

## Workflow

1. Call `import_recipe` with exactly one source:
   - `url` — a recipe web page. Works without a login; social-media links (videos, posts) need a cook.md account and use the import allowance of Cook Basic or Cook Pro.
   - `text` — recipe text the user pasted. Works without a login.
   - `image_paths` — 1–10 photos (cookbook page, recipe card) as paths inside the collection. Photos need a cook.md account and use the import allowance of Cook Basic or Cook Pro.
   It returns Cooklang text (plus metadata) and does NOT save anything.
2. On `login_required`, call `login` and show the user the code and link. On `plan_required`, show the user the `checkout_url`. If they don't want to upgrade, follow the returned `next_step`: read the source yourself and write the Cooklang by hand (cooklang-editing skill).
3. Decide an output path: look at the collection layout with `list_recipes`, then pick a category folder and a name in the user's naming style. Check the name isn't taken — `write_recipe` overwrites.
4. Merge the returned metadata into YAML frontmatter (`title`, `source` with the URL, `servings`, times, `tags`). Keep `source` — the user will want to find the original.
5. Self-check against the cooklang-validation rules — no separate ingredient section, each quantity declared once inline, multi-word names braced — then run `validate` with the text as `content` and `as_path` set to the chosen path. Fix every error.
6. Save with `write_recipe` and tell the user what you saved and where.

## Rules

- Never copy a recipe out of a web page by hand when `import_recipe` can take the URL — it handles the page structure and returns clean Cooklang.
- See the `cooklang://syntax` resource and the metadata skill for field names if you adjust the converted output.
- Touch only the file you are importing into. Ask before overwriting an existing recipe of the same name.
- The import is a starting point: check that quantities and steps survived (especially from photos), and say if anything was unreadable.

## Do not skip the save

| Excuse | Rebuttal |
|--------|----------|
| "`import_recipe` ran, so the recipe is imported." | It only RETURNS Cooklang. Nothing is in the collection until you `validate` it and save it with `write_recipe`. |
| "I'll report success now and save in a moment." | Save first, then report the path you saved to. |
