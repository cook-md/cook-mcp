# cook-mcp

An MCP server that gives your AI agent (Claude Code, Claude Desktop, Cursor, ChatGPT, or any other MCP client) access to your Cooklang recipe collection. Recipes stay as plain `.cook` and `.menu` files in a folder you own. The agent can read, search, validate and write them, build shopping lists, track a pantry and render reports, all locally and without an account. With a cook.md login it can also compute nutrition and import recipes from photos and social links.

## Install

Claude Code:

```sh
claude mcp add cook -- npx -y @cookmd/mcp
```

Any client that takes an `mcpServers` config:

```json
{
  "mcpServers": {
    "cook": {
      "command": "npx",
      "args": ["-y", "@cookmd/mcp"],
      "env": { "COOK_RECIPES_DIR": "/path/to/recipes" }
    }
  }
}
```

If `COOK_RECIPES_DIR` is not set, the server uses the workspace folder your client shares, or the folder the client was started in. See [How the recipe folder is chosen](#how-the-recipe-folder-is-chosen).

Supported platforms: macOS (arm64, x64) and Linux (x64, arm64, glibc 2.35 or newer). There are no Windows builds yet.

Setup notes for specific clients: https://cook.md/help/mcp

## Claude Code plugin

Claude Code users can install the `cooklang` plugin instead of adding the server by hand. It lives in [cooklang/cooklang-skills](https://github.com/cooklang/cooklang-skills) and bundles this server plus the skills below:

```
/plugin marketplace add cooklang/cooklang-skills
/plugin install cooklang@cooklang-skills
```

The plugin starts the server with your Claude Code project folder as the recipe root, and Claude Code picks the right skill from what you ask ("plan dinners for next week", "is this recipe valid?"). If you already added the server with `claude mcp add cook`, remove that entry so you don't run two copies.

The skills (also served by this server as `cooklang://skills/<name>` resources):

| Skill | Use it for |
|-------|------------|
| `cooklang-editing` | Write a new recipe or edit and fix a `.cook` file |
| `cooklang-validation` | Check recipes, a folder or the whole collection for errors and broken references |
| `metadata` | Add or fix YAML frontmatter (title, tags, servings, times), including bulk changes |
| `recipe-import` | Import a recipe from a URL, photos or pasted text |
| `recipe-search` | Find recipes by ingredient, tag, cuisine, course or a remembered phrase |
| `scale-recipe` | Show a recipe for more or fewer servings |
| `export-recipe` | Turn a recipe into Markdown, JSON, plain text or HTML |
| `organize-collection` | Folder layout, metadata audit, aisle and pantry config, whole-library checks |
| `meal-planning` | Build or edit a `.menu` meal plan |
| `shopping-list` | Shopping lists from recipes or plans, grouped by aisle, minus the pantry |
| `pantry` | Track stock, expiry and low items; what can I cook with what I have |
| `report-authoring` | Custom Jinja reports and printouts with `render_report` |
| `nutrition-reports` | Nutrition evaluation and screening (Cook Basic or Pro) |
| `nutrition-goals` | Change a recipe or plan to hit nutrition targets (Cook Basic or Pro) |

`skills/` in this repo is the canonical copy; the plugin repo syncs from it. Each skill is `skills/<name>/SKILL.md`.

## Tools

### Free (local, no login)

| Tool | What it does |
|------|--------------|
| `list_recipes` | List recipes, meal plans or report templates (`kind`: recipe, menu, template, all) |
| `read_recipe` | Read a recipe or menu: source plus parsed ingredients, cookware, steps and metadata; optional scaling |
| `search_recipes` | Search names and contents, optionally filtered by tag |
| `validate` | Check a file, a folder, the whole collection, or unsaved content for errors and broken references |
| `write_recipe` | Save a `.cook` recipe (validated first) |
| `write_menu` | Save a `.menu` meal plan (validated first) |
| `write_config` | Save `config/aisle.conf`, `config/pantry.conf`, or a `.jinja` report template under `reports/` or `config/reports/` |
| `shopping_list` | Build a shopping list from recipes and/or menus: merges duplicates, groups by aisle, subtracts the pantry |
| `pantry_list` | Show the pantry, by section |
| `pantry_expiring` | Items expiring soon |
| `pantry_depleted` | Items at or below their low-stock threshold |
| `pantry_recipes` | Which recipes you can cook with what is in the pantry |
| `pantry_update` | Add, update or remove pantry items |
| `render_report` | Render a jinja report template against a recipe or menu (plain templates need no login) |

### Cook Basic / Pro (cook.md login)

| Tool | What it does |
|------|--------------|
| `login` | Start a cook.md device login; shows a code and a URL |
| `auth_status` | Show login status, plan and import allowance |
| `get_nutrition` | Nutrition facts for one ingredient amount |
| `aggregate_nutrition` | Sum nutrition across many ingredient lines |
| `lookup_ingredient` | Fuzzy-search the ingredient catalog |
| `convert_units` | Convert between units (volume to mass needs a density) |
| `check_category` | Check whether an ingredient belongs to a category |
| `branded_lookup` | Look up a packaged product by barcode or text |
| `reference_intakes` | Daily reference-intake tables (RDA/DV) |
| `import_recipe` | Convert a web page, photos or pasted text to Cooklang |

`import_recipe` from a web page or pasted text works without a login. Photos and social-media links need a cook.md account and use your import allowance. It returns Cooklang text and does not save it; the agent validates it and calls `write_recipe`. Nutrition functions inside `render_report` also need Cook Basic or Pro.

## Prompts and resources

Prompts: `meal-planning`, `shopping-list`, `pantry`, `import-recipe`, `edit-recipe`, `nutrition-report`, `nutrition-goals`, `scale-recipe`.

Resources:

- `cooklang://spec`: the Cooklang specification
- `cooklang://syntax`: a syntax reference
- `cooklang://menu-format`: the `.menu` meal plan format
- `cooklang://skills/<name>`: working guides for the agent, one per skill in [the table above](#claude-code-plugin): `cooklang-editing`, `cooklang-validation`, `export-recipe`, `meal-planning`, `metadata`, `nutrition-goals`, `nutrition-reports`, `organize-collection`, `pantry`, `recipe-import`, `recipe-search`, `report-authoring`, `scale-recipe`, `shopping-list`

## Safety

- Writes stay inside the recipe root. Paths outside it are refused.
- Every write is validated first; invalid Cooklang is not saved.
- The deprecated `>>` metadata syntax is refused. Use YAML frontmatter.
- There is no delete tool. The agent cannot remove your files.

## Known limitations

- Pantry subtraction in shopping lists only works when units match. For example, 1 kg in the pantry does not cancel 200 g in a recipe. This is a limitation of cookcli-core.
- Symlinks inside the recipe folder are mostly not followed for reads and listing. A symlink requested by bare name without an extension, or reached through a recipe's `@./` reference, may still be followed. This only matters if you put symlinks pointing outside the folder into your recipes.
- No Windows builds yet.
- No delete tool.

## Environment variables

| Var | Default | Meaning |
|-----|---------|---------|
| `COOK_RECIPES_DIR` | the client's roots, then its working directory | Recipe root. Every path is relative to it. Always wins when set. |
| `COOKMD_BASE_URL` | `https://cook.md` | Login, entitlements, import |
| `NUTRITION_API_URL` | `https://nutrition.cook.md` | Nutrition service |
| `NUTRITION_API_TOKEN` | none | Org key or pre-made token. Takes priority over the stored login. |
| `COOK_MCP_AUTH_PATH` | `~/.config/cook-mcp/auth.json` | Token store. The old `NUTRITION_MCP_AUTH_PATH` still works as an alias. |

## How the recipe folder is chosen

1. `COOK_RECIPES_DIR`, if set. Nothing else is consulted.
2. The client's MCP roots. If the client supports roots (it shares its open workspace folders with the server), the first `file://` root that is an existing folder becomes the recipe root. The server asks on the first recipe tool call and again whenever the client reports that its roots changed. If the client doesn't answer within 5 seconds, the server uses the working directory and asks again later (at most once every 30 seconds).
3. The folder the client started the server in.

A root is a folder you opened on purpose, so it is used unless it is `/`, your home folder or inside an agent plugin install folder (for example `~/.codex/plugins/cache/...`, `~/.gemini/extensions/...` or the folder `CLAUDE_PLUGIN_ROOT` points to). The working directory is checked more strictly: it is also refused when it, or a folder up to four levels above it, has `.claude-plugin/plugin.json`, an Agent Plugins `plugin.json` or `gemini-extension.json`, because a plugin that starts the server in its own install folder would otherwise expose the wrong files and write recipes into a folder that is wiped on update. When nothing usable is found, recipe tools say that no recipe folder is set, why, and that `COOK_RECIPES_DIR` fixes it; `auth_status` shows `recipe_root_source: "unset"`. Otherwise `recipe_root_source` is `env`, `roots` or `cwd`.

What each client does:

- **Claude Code** sends its project folder as a root (verified), so nothing to configure.
- **Codex** sends no roots (verified with 0.160.1) and may start the server in a plugin folder, so set `COOK_RECIPES_DIR`:

  ```sh
  codex mcp add cook --env COOK_RECIPES_DIR=/path/to/recipes -- npx -y @cookmd/mcp
  ```

- **VS Code and Cursor** document support for roots (not verified here). If recipe tools say no folder is set, set `COOK_RECIPES_DIR` in the server's config.

## Things to ask

- "Plan dinners for next week from my recipes and make the shopping list."
- "What can I cook with what's in my pantry?"
- "Check my whole collection for broken references."
- "Import https://example.com/some-recipe as a recipe."
- "How much protein is in this week's plan?" (needs Cook Basic or Pro)

## Building from source

```sh
cargo build --release
cargo test
```

The binary is `target/release/cook-mcp`. It speaks MCP over stdio. `cook-mcp login` and `cook-mcp logout` manage the cook.md login from a terminal.

## Migrating from nutrition-mcp

`@cookmd/nutrition-mcp` keeps working: it is now a thin shim that runs `@cookmd/mcp`. To switch, change the package name in your MCP config to `@cookmd/mcp`. Your login carries over.

Two things changed. Recipe tools need `COOK_RECIPES_DIR` (old configs did not set it), or start the client in your recipe folder; without either, recipe tools tell the agent that no recipe folder is set. And `render_report` paths are now relative to the recipe folder.

## License

MIT
