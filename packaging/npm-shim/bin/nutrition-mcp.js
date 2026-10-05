#!/usr/bin/env node
// @cookmd/nutrition-mcp was renamed to @cookmd/mcp; run that.
const { spawn } = require("child_process");
const bin = require.resolve("@cookmd/mcp/bin/cook-mcp.js");
const child = spawn(process.execPath, [bin, ...process.argv.slice(2)], { stdio: "inherit" });
child.on("exit", (code, signal) => process.exit(signal ? 1 : code ?? 1));
