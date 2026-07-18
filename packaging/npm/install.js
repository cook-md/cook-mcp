// Downloads the prebuilt nutrition-mcp binary for this platform from the
// public releases repo. No compilation on the client.
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { Readable } = require("stream");
const { pipeline } = require("stream/promises");

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
};

// NUTRITION_MCP_DOWNLOAD_BASE lets internal mirrors override where binaries
// are fetched from; layout must match the GitHub releases URL scheme.
const BASE =
  process.env.NUTRITION_MCP_DOWNLOAD_BASE ||
  "https://github.com/cook-md/nutrition-mcp/releases/download";

async function main() {
  const key = `${process.platform}-${process.arch}`;
  const target = TARGETS[key];
  if (!target) {
    throw new Error(
      `unsupported platform: ${key} (supported: ${Object.keys(TARGETS).join(", ")})`
    );
  }
  const version = require("./package.json").version;
  // Written by the release workflow at publish time: {target: sha256}.
  const expected = require("./checksums.json")[target];
  if (!expected) throw new Error(`no checksum recorded for ${target}`);
  const url = `${BASE}/v${version}/nutrition-mcp-${target}.tar.gz`;
  const destDir = path.join(__dirname, "dist");
  fs.mkdirSync(destDir, { recursive: true });
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`download failed ${res.status}: ${url}`);
  const tarPath = path.join(destDir, "bin.tar.gz");
  await pipeline(Readable.fromWeb(res.body), fs.createWriteStream(tarPath));
  const actual = crypto
    .createHash("sha256")
    .update(fs.readFileSync(tarPath))
    .digest("hex");
  if (actual !== expected) {
    fs.rmSync(tarPath);
    throw new Error("checksum mismatch — refusing to install");
  }
  const { execFileSync } = require("child_process");
  execFileSync("tar", ["-xzf", tarPath, "-C", destDir]);
  fs.rmSync(tarPath);
  fs.chmodSync(path.join(destDir, "nutrition-mcp"), 0o755);
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
