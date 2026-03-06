#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const https = require("node:https");

const owner = "nikitaclicks";
const repo = "context-cutter";
const version = process.env.npm_package_version;
const releaseTag = `v${version}`;

function resolveAssetName() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "linux" && arch === "x64") {
    return "context-cutter-mcp-x86_64-linux-gnu";
  }
  if (platform === "linux" && arch === "arm64") {
    return "context-cutter-mcp-aarch64-linux-gnu";
  }
  if (platform === "darwin" && arch === "x64") {
    return "context-cutter-mcp-x86_64-apple-darwin";
  }
  if (platform === "darwin" && arch === "arm64") {
    return "context-cutter-mcp-aarch64-apple-darwin";
  }
  if (platform === "win32" && arch === "x64") {
    return "context-cutter-mcp-x86_64-pc-windows-msvc.exe";
  }

  throw new Error(`Unsupported platform/arch: ${platform}/${arch}`);
}

function download(url, destination) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "context-cutter-mcp-installer" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return resolve(download(res.headers.location, destination));
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Download failed: ${res.statusCode} ${res.statusMessage}`));
        }

        const file = fs.createWriteStream(destination, { mode: 0o755 });
        res.pipe(file);
        file.on("finish", () => file.close(resolve));
        file.on("error", reject);
      })
      .on("error", reject);
  });
}

async function main() {
  const asset = resolveAssetName();
  const url = `https://github.com/${owner}/${repo}/releases/download/${releaseTag}/${asset}`;

  const binDir = path.join(__dirname, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const output = path.join(binDir, os.platform() === "win32" ? "context-cutter-mcp.exe" : "context-cutter-mcp");
  await download(url, output);
  fs.chmodSync(output, 0o755);
}

main().catch((err) => {
  console.error(`[context-cutter-mcp] install failed: ${err.message}`);
  process.exit(1);
});
