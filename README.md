# nutrition-mcp

Prebuilt binaries for **nutrition-mcp** — the cook.md MCP server for nutrition
evaluation and Cooklang report rendering in AI agent tools.

**Install:** add to your MCP client config (Claude Code, Claude Desktop, Cursor, ...):

```json
{
  "mcpServers": {
    "nutrition": {
      "command": "npx",
      "args": ["-y", "@cookmd/nutrition-mcp"]
    }
  }
}
```

The npm wrapper downloads the right binary for your platform from this repo's
[Releases](https://github.com/cook-md/nutrition-mcp/releases) and verifies its
SHA-256 checksum before installing. Supported platforms: macOS (Apple Silicon +
Intel) and Linux (x86_64 + arm64, glibc ≥ 2.35).

First use: ask your agent to run the `login` tool (cook.md device login), or set
`NUTRITION_API_TOKEN` for organization API keys.

This repository hosts release artifacts only — it contains no source code.
