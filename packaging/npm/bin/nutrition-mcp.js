#!/usr/bin/env node
const path = require("path");
const { spawn } = require("child_process");
const bin = path.join(__dirname, "..", "dist", "nutrition-mcp");
const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });
child.on("exit", (code, signal) => process.exit(signal ? 1 : code ?? 1));
child.on("error", (e) => {
  console.error(
    `nutrition-mcp binary missing or not executable (${e.message}). Reinstall the package.`
  );
  process.exit(1);
});
