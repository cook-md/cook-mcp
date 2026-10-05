#!/usr/bin/env node
const fs = require("fs");
const path = require("path");
const { spawn, execFileSync } = require("child_process");
const bin = path.join(__dirname, "..", "dist", "cook-mcp");
if (!fs.existsSync(bin)) {
  // postinstall was skipped (e.g. --ignore-scripts): fetch the binary now.
  // Installer output goes to stderr; stdout carries the MCP protocol.
  try {
    execFileSync(process.execPath, [path.join(__dirname, "..", "install.js")], {
      stdio: ["ignore", process.stderr, process.stderr],
    });
  } catch (e) {
    // fall through to the error below
  }
}
const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });
child.on("exit", (code, signal) => process.exit(signal ? 1 : code ?? 1));
child.on("error", (e) => {
  console.error(
    `cook-mcp binary missing or not executable (${e.message}). Reinstall the package.`
  );
  process.exit(1);
});
